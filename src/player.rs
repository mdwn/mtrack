// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
mod hardware;
mod hardware_status;
pub mod lock_policy;
mod navigation;
mod play_declined;
mod playback;
mod seek;
mod status;

pub use hardware::{process_id, ConfigApply, ConfigChange, DeferredReloads, HardwareReload};
pub use hardware_status::{
    AudioOutputHealth, ControllerStatus, HardwareStatusSnapshot, LightingVenueStatus,
    SubsystemStatus, TestOutputStatus,
};
pub use play_declined::{PlayDeclined, PLAY_DECLINED_KEY};
pub use status::{
    AmbientStatus, DmxRead, LightingStatus, PlayerStatus, AMBIENT_UNREAD, STATUS_DMX_READ_TIMEOUT,
};

use midly::live::LiveEvent;
use parking_lot::RwLock;
use std::fmt;
use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};
use tokio::{
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, span, warn, Level, Span};

use crate::samples::SampleEngine;
use crate::songs::Songs;
use crate::trigger::TriggerEngine;
use crate::{
    audio, config, dmx, midi,
    playlist::{self, Playlist},
    playsync::CancelHandle,
    songs::Song,
};

/// What every surface says for the lighting effects when there is no
/// effect engine (no DMX engine configured).
pub const NO_EFFECT_ENGINE: &str = "no effect engine configured";

/// Direction for playlist navigation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlaylistDirection {
    Next,
    Prev,
}

impl fmt::Display for PlaylistDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaylistDirection::Next => write!(f, "next"),
            PlaylistDirection::Prev => write!(f, "previous"),
        }
    }
}

/// Holds the ingredients for constructing a per-song `PlaybackClock`.
/// When an audio device is present, clocks are derived from its hardware
/// sample counter. Otherwise, clocks fall back to `Instant::now()`.
#[derive(Clone)]
enum ClockSource {
    Audio {
        sample_counter: Arc<std::sync::atomic::AtomicU64>,
        sample_rate: u32,
    },
    Wall,
}

impl ClockSource {
    fn new_clock(&self) -> crate::clock::PlaybackClock {
        match self {
            ClockSource::Audio {
                sample_counter,
                sample_rate,
            } => crate::clock::PlaybackClock::from_sample_counter(
                sample_counter.clone(),
                *sample_rate,
            ),
            ClockSource::Wall => crate::clock::PlaybackClock::wall(),
        }
    }
}

/// Notified when the current song changes. Implementations capture whatever
/// device handles they need at construction time, keeping the player unaware
/// of protocol-specific details.
pub trait SongChangeNotifier: Send + Sync {
    /// Called when the player advances to a new song.
    fn notify(&self, song: &Song);
}

/// Groups all hardware device state so it can be atomically swapped on reload.
#[derive(Clone)]
struct HardwareState {
    device: Option<Arc<dyn audio::Device>>,
    mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    /// Shared per-output-track gains, built from the active profile's
    /// track_mappings and track_gains. Also installed into the device mixer.
    track_gains: Option<Arc<crate::audio::track_gains::TrackGains>>,
    midi_device: Option<Arc<dyn midi::Device>>,
    /// The MIDI config `midi_device` was opened with, as YAML: a reload of
    /// the same config keeps the device, opened afresh in place.
    midi_config: Option<String>,
    /// The MIDI device a reload took down, with its config, until a round
    /// keeps it, opens another or finds no MIDI config. Handed to the round
    /// itself, it was lost with a round cancelled before its MIDI phase, and
    /// the round after opened the device anew (#578).
    midi_handoff: Option<(String, Arc<dyn midi::Device>)>,
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    sample_engine: Option<Arc<RwLock<SampleEngine>>>,
    trigger_engine: Option<Arc<TriggerEngine>>,
    clock_source: ClockSource,
    /// Notifiers invoked on every song change.
    song_change_notifiers: Vec<Arc<dyn SongChangeNotifier>>,
    /// The hostname of the active hardware profile (None if no profile matched).
    profile_name: Option<String>,
    /// The file the active profile was loaded from (`stage.yml`); None for
    /// an inline profile, or none matched.
    profile_file: Option<String>,
    /// The resolved machine hostname used for profile matching.
    hostname: Option<String>,
    /// Subsystems whose init gave up, keyed by name, with the reason.
    ///
    /// Only terminal failures land here — a device still being waited for is
    /// not a failure, it is a wait. Cleared by a reload, so fixing the config
    /// and reloading clears the report along with the cause.
    init_errors: HashMap<String, String>,
    /// Problems found at startup in a subsystem that did come up, keyed by
    /// name: reported beside `connected` rather than as a failure. Cleared by
    /// a reload, as `init_errors` is.
    init_warnings: HashMap<String, String>,
}

/// Alias for the shared state sampler sender stored on Player.
type StateTx = Arc<
    parking_lot::Mutex<Option<Arc<tokio::sync::watch::Sender<Arc<crate::state::StateSnapshot>>>>>,
>;

/// Lightweight, change-detect-friendly view of the player's transport state.
///
/// This is broadcast on the `transport_tx` watch channel and used by
/// subscribers (e.g. the MCP `mtrack://status` notification) to detect
/// meaningful state transitions. Fields that change continuously during
/// playback — most notably `elapsed` — are deliberately excluded, so a
/// passive subscriber is only woken when the user-visible transport state
/// actually transitions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransportSnapshot {
    pub playlist_name: String,
    pub current_song: Option<String>,
    pub playing: bool,
    pub section_loop: Option<String>,
    pub reactive_loop_state: ReactiveLoopStateKind,
    pub locked: bool,
}

/// Variants of [`ReactiveLoopState`] without the inner `SectionBounds` payload.
/// We only care about transitions between variants, not the bounds themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReactiveLoopStateKind {
    #[default]
    Idle,
    SectionOffered,
    LoopArmed,
    Looping,
    BreakRequested,
}

impl From<&ReactiveLoopState> for ReactiveLoopStateKind {
    fn from(state: &ReactiveLoopState) -> Self {
        match state {
            ReactiveLoopState::Idle => Self::Idle,
            ReactiveLoopState::SectionOffered(_) => Self::SectionOffered,
            ReactiveLoopState::LoopArmed(_) => Self::LoopArmed,
            ReactiveLoopState::Looping(_) => Self::Looping,
            ReactiveLoopState::BreakRequested(_) => Self::BreakRequested,
        }
    }
}

struct PlayHandles {
    join: JoinHandle<()>,
    cancel: CancelHandle,
}

/// Where a playing song's position comes from.
#[derive(Clone, Copy, Debug)]
enum PlayPosition {
    /// The playback started at this position and waits on its subsystems,
    /// its clock not yet running: it is still there (#634). Without it the
    /// position read as none, 0:00, for most of a second after a play from
    /// part way in, while the lights already showed the start position.
    Starting(Duration),
    /// The clock runs: when the song's start would have been, on the wall.
    Running(SystemTime),
}

/// Groups the parameters needed for `play_files` to avoid excessive argument counts.
struct PlaybackContext {
    device: Option<Arc<dyn audio::Device>>,
    mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    midi_device: Option<Arc<dyn midi::Device>>,
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    clock: crate::clock::PlaybackClock,
    song: Arc<Song>,
    cancel_handle: CancelHandle,
    play_tx: oneshot::Sender<Result<(), String>>,
    start_time: Duration,
    play_start_time: Arc<Mutex<Option<PlayPosition>>>,
    /// Loop control state shared with audio/MIDI/DMX subsystems.
    loop_control: crate::playsync::LoopControl,
}

/// Groups hardware devices for constructing a Player without discovering real hardware.
pub struct PlayerDevices {
    pub audio: Option<Arc<dyn audio::Device>>,
    pub mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    pub midi: Option<Arc<dyn midi::Device>>,
    pub dmx_engine: Option<Arc<dmx::engine::Engine>>,
    pub sample_engine: Option<Arc<RwLock<SampleEngine>>>,
    pub trigger_engine: Option<Arc<TriggerEngine>>,
}

impl LightingVenueStatus {
    /// The status of a registration outcome (or of none).
    pub fn from_registration(
        registration: Option<&crate::dmx::engine::VenueRegistration>,
    ) -> LightingVenueStatus {
        match registration {
            Some(r) if r.venue.is_some() => LightingVenueStatus {
                name: r.venue.clone(),
                status: if r.ok { "ok" } else { "failed" }.to_string(),
                error: if r.ok { None } else { r.error.clone() },
            },
            _ => LightingVenueStatus {
                name: None,
                status: "none".to_string(),
                error: None,
            },
        }
    }
}

/// Plays back individual wav files as multichannel audio for the configured audio interface.
#[derive(Clone)]
pub struct Player {
    /// All hardware device state, behind a lock so it can be atomically swapped on reload.
    hardware: Arc<parking_lot::RwLock<HardwareState>>,
    /// Base path for resolving sample/DMX config files. None in test paths.
    base_path: Option<PathBuf>,
    /// All playlists keyed by name. Always includes "all_songs".
    playlists: Arc<parking_lot::RwLock<HashMap<String, Arc<Playlist>>>>,
    /// The name of the currently active playlist.
    active_playlist: Arc<parking_lot::RwLock<String>>,
    /// Held by a reload from copying each playlist's place to swapping the
    /// new playlists in, and by a move of the active playlist's place, so
    /// a move cannot land on a playlist a reload has already copied.
    playlist_moves: Arc<parking_lot::Mutex<()>>,
    /// The persisted active playlist name (last non-all_songs choice). Used by
    /// MIDI/OSC `Playlist` action to return to the user's "real" playlist.
    persisted_playlist: Arc<parking_lot::RwLock<String>>,
    /// Where the playing song's position comes from: none while stopped.
    play_start_time: Arc<Mutex<Option<PlayPosition>>>,
    /// Keeps track of the player joins. There should only be one task on here at a time.
    join: Arc<Mutex<Option<PlayHandles>>>,
    /// Whether a song is playing: set as `join` becomes `Some`, and cleared
    /// as it goes back to `None`, but readable without the join lock. A
    /// playlist reload asks it rather than the lock, which every status
    /// poll takes too, so a poll cannot pass for a song playing (#576).
    song_playing: Arc<AtomicBool>,
    /// After stop is set, this will be set to true. This will prevent stop from being run again until
    /// it is unset, which should be handled by a cleanup async process after playback finishes.
    stop_run: Arc<AtomicBool>,
    /// The logging span.
    span: Span,
    /// Mutable configuration store for runtime config changes.
    config_store: Arc<parking_lot::Mutex<Option<Arc<config::ConfigStore>>>>,
    /// Where songs and playlists are read from: resolved from the config
    /// store by one resolver, again as the config changes, and read by
    /// every surface's reload and file access alike.
    song_sources: Arc<parking_lot::RwLock<Option<SongSources>>>,
    /// The legacy playlist named on the command line, which wins over the
    /// config's whenever the sources are resolved.
    cli_playlist: Arc<parking_lot::RwLock<Option<PathBuf>>>,
    /// Held by a song rescan from its scan to its swap, on the blocking
    /// thread it runs on: a rescan outlives a request dropped mid-way, and
    /// one let go with the request's lock overlapped the next, the older
    /// scan swapping its songs in last.
    song_scans: Arc<parking_lot::Mutex<()>>,
    /// The status's reads of the DMX engine outstanding, one of each kind
    /// at most (see `status::InflightRead`).
    ambient_reads: Arc<status::InflightRead<dmx::engine::AmbientReport>>,
    /// Counts changes to any track's gain or mute, from any surface, so a
    /// remote screen can tell its levels are stale without reading them. It
    /// lives here, not in `TrackGains`, because a hardware reload builds a
    /// new `TrackGains`. Wraps on overflow.
    gains_revision: Arc<std::sync::atomic::AtomicU32>,
    lighting_reads: Arc<status::InflightRead<Option<LightingStatus>>>,
    /// Player-wide "metronome on by default" flag, applied when (re)loading
    /// songs (see `Song::apply_metronome_default`).
    default_metronome: Arc<std::sync::atomic::AtomicBool>,
    /// Cancellation token for the current hardware init round. On reload,
    /// the old token is cancelled and a new one is created.
    init_cancel: Arc<parking_lot::Mutex<CancellationToken>>,
    /// The latest hardware init round's task. A round starts only once the
    /// one before it has finished (see `hardware::spawn_after`).
    init_round: Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Serializes `reload_hardware` from start to spawn.
    ///
    /// The config is read before the cancellation token is replaced, and that
    /// read awaits — so two concurrent reloads could interleave such that the
    /// one holding the *older* config cancelled the newer round and spawned
    /// last, leaving the rig built from a config the store had already
    /// superseded. Serializing means the last reload to run is the one that
    /// read the newest config.
    reload_lock: Arc<tokio::sync::Mutex<()>>,
    /// A hardware reload asked for while a song played or an auto play
    /// waited, which runs once the player is idle. Refusing it instead left
    /// a config save acknowledged while the rig kept the old config (#578).
    hardware_reload_pending: Arc<AtomicBool>,
    /// The deferred hardware reloads started or refused, counted.
    deferred_reloads: Arc<parking_lot::Mutex<hardware::DeferredReloads>>,
    /// Broadcast channel sender, stored so async init can wire DMX engine.
    broadcast_tx: Arc<parking_lot::Mutex<Option<tokio::sync::broadcast::Sender<String>>>>,
    /// Watch channel to signal hardware init completion.
    init_done_tx: Arc<tokio::sync::watch::Sender<bool>>,
    /// State sampler watch sender. Shared so async init can start the sampler
    /// when the DMX engine becomes available, and restart it on reload.
    state_tx: StateTx,
    /// Transport-state watch channel. A low-rate poller refreshes this and
    /// uses `send_if_modified` so receivers (e.g. MCP `mtrack://status`
    /// subscribers) only wake on meaningful transitions, never on the
    /// per-tick fixture sampler.
    transport_tx: Arc<tokio::sync::watch::Sender<TransportSnapshot>>,
    /// When true, state-altering operations (config changes, song edits, etc.)
    /// are rejected. Playback controls always work. Locked by default on startup.
    locked: Arc<AtomicBool>,
    /// Active controllers (gRPC, OSC, MIDI). Replaced on reload.
    controller: Arc<parking_lot::Mutex<Option<crate::controller::Controller>>>,
    /// The transport's state machine, behind one lock: what a loop break
    /// asked of the looping song does, a looping song ending, an auto play
    /// waiting out its gap, the position kept for the next play, whether
    /// the last entry has played out, and when a song fading out is out.
    /// Read and changed together, they cannot be seen half changed.
    ///
    /// Lock ordering: this lock is taken after `join`, `play_start_time`,
    /// `reload_lock` and `playlist_moves` when they are held, never before
    /// them, and nothing awaits while it is held. Only
    /// [`Player::set_ambient_mode`] holds it across another call, into the
    /// ambient light (the hardware, ambient mode, playlist and ambient source
    /// locks, and the DMX engine), none of which takes it in turn.
    transport: Arc<parking_lot::Mutex<TransportState>>,
    /// The engines' copy of whether a loop break is asked of the song
    /// playing: the audio, MIDI and DMX threads poll it through the
    /// playback's `LoopControl`, so it is an atomic of its own rather than a
    /// read of `transport`. Set and cleared only with
    /// `TransportState::loop_exit`, under the transport lock, so the two
    /// always agree.
    loop_break: Arc<AtomicBool>,
    /// Ambient light's mode, kept here so an engine a reload brings in
    /// takes it up.
    ambient_mode: Arc<parking_lot::Mutex<dmx::engine::AmbientMode>>,
    /// Whose settings the ambient light the player last brought up plays.
    ambient_source: Arc<parking_lot::Mutex<Option<crate::playlist::AmbientSource>>>,
    /// A looping song fading out after a loop break (a playlist `fade_out`):
    /// when the break came and how long the fade lasts. The audio monitor
    /// reads it to carry the fade across a loop boundary. Shared with the
    /// audio threads through the playback's `LoopControl`, so it is not part
    /// of `transport`, as are `break_fade_ids` and `start_fade`.
    break_fade: Arc<parking_lot::Mutex<Option<crate::playsync::BreakFade>>>,
    /// The mixer sources fading out after a loop break, which a stop or seek
    /// cuts rather than letting them finish.
    break_fade_ids: Arc<parking_lot::Mutex<Vec<u64>>>,
    /// How the song a loop break moves to fades in under the one fading out
    /// (a playlist `fade_in`), taken by the audio device as it starts.
    start_fade: Arc<parking_lot::Mutex<Option<crate::playsync::StartFade>>>,
    /// The section loop armed in the playback, if one is, and the reactive
    /// loop's state.
    section_loop: SectionLoop,
    /// The playing song's section loop jumps (its playback's log): the
    /// position counts them as they fall due, exactly.
    section_jumps: Arc<parking_lot::Mutex<Arc<crate::playsync::SectionJumpLog>>>,
    /// Notification engine for section loop audio feedback.
    notification_engine: Arc<crate::notification::NotificationEngine>,
    /// Pending debounced task persisting track gains to the config store.
    /// Aborted and replaced on every gain change so rapid fader moves
    /// produce a single disk write.
    gain_persist_task: Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

/// Errors from setting an output track gain. Variants are matched by the
/// gRPC layer to choose status codes, so controllers never classify errors
/// by message text.
#[derive(Debug, thiserror::Error)]
pub enum TrackGainError {
    #[error("gain must be a finite number, got {0}")]
    NonFinite(f32),
    #[error("no audio profile active")]
    NoAudioProfile,
    #[error(transparent)]
    UnknownTrack(#[from] crate::audio::track_gains::UnknownTrackError),
}

impl TrackGainError {
    /// What kind of fault this is, for a transport's code: no audio
    /// profile is a precondition; a gain or track the caller named wrong is
    /// invalid.
    pub fn kind(&self) -> crate::util::FaultKind {
        match self {
            TrackGainError::NoAudioProfile => crate::util::FaultKind::Precondition,
            TrackGainError::NonFinite(_) | TrackGainError::UnknownTrack(_) => {
                crate::util::FaultKind::Invalid
            }
        }
    }
}

/// A section loop's state: the section armed, a break asked of it, when it
/// was armed, and the reactive loop's state. It belongs to one playback, or
/// a paused song's, and [`SectionLoop::reset`] forgets it as each starts
/// fresh and stops (a pause and its resume keep it): kept, the next playback
/// -- the same song from the top, or another song -- looped the old section,
/// at the old song's bounds, once it reached the section's end.
#[derive(Clone, Default)]
pub(crate) struct SectionLoop {
    /// Active section loop bounds. When Some, audio/MIDI/DMX subsystems
    /// loop the specified time region instead of the full song.
    active: Arc<parking_lot::RwLock<Option<SectionBounds>>>,
    /// Signal to stop section looping. The current iteration finishes and
    /// the song continues from the section end.
    break_requested: Arc<AtomicBool>,
    /// The section loop jumps due when the section loop now armed was
    /// armed: the loop is going round once more have fallen due.
    jumps_at_arm: Arc<parking_lot::Mutex<usize>>,
    /// Reactive loop state machine.
    reactive: Arc<parking_lot::RwLock<ReactiveLoopState>>,
}

impl SectionLoop {
    /// Forgets the section loop and the reactive loop's state: nothing
    /// armed, offered or breaking.
    fn reset(&self) {
        *self.active.write() = None;
        self.break_requested.store(false, Ordering::Relaxed);
        *self.jumps_at_arm.lock() = 0;
        *self.reactive.write() = ReactiveLoopState::Idle;
    }

    /// Takes the section loop up in the playback resuming a paused song:
    /// armed, offered, looping or breaking as the pause left it. A loop
    /// armed and not yet round counts its jumps from none in the new
    /// playback's log; counted from the old log's, it never read as
    /// looping.
    fn resume(&self) {
        *self.jumps_at_arm.lock() = 0;
    }
}

/// A state-altering operation was refused because the player is locked.
/// Every control surface (web UI, gRPC, MCP) answers with this message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Player is locked. Unlock to make changes.")]
pub struct LockedError;

/// Bounds of an active section loop.
///
/// Used together with the section loop's break flag to form a state machine
/// shared by the audio, MIDI, and DMX engine threads, which see the two as
/// `LoopControl::active_section` and `LoopControl::section_loop_break`:
///
/// | State    | `active_section`  | `section_loop_break` |
/// |----------|-------------------|----------------------|
/// | Idle     | `None`            | `false`              |
/// | Looping  | `Some(bounds)`    | `false`              |
/// | Breaking | `Some(bounds)`*   | `true`               |
///
/// Transitions:
///   - [`Player::loop_section`], or [`Player::section_ack`] on a section
///     offered: Idle → Looping (sets `active_section`, clears
///     `section_loop_break`)
///   - [`Player::stop_section_loop`]: Looping → Breaking → Idle (sets
///     `section_loop_break` first, then clears `active_section`)
///   - Each engine thread detects the break and exits its loop
///
/// (*) `active_section` is cleared shortly after `section_loop_break` is
/// set. DMX caches the section bounds so it can compute a resume position
/// even after the field is cleared.
///
/// Trigger scheduling within each engine is handled by
/// [`crate::section_loop::SectionLoopTrigger`].
#[derive(Debug, Clone, PartialEq)]
pub struct SectionBounds {
    pub name: String,
    pub start_time: Duration,
    pub end_time: Duration,
}

impl SectionBounds {
    /// The section of `song` the playhead at `at` sits inside, if any,
    /// resolved to absolute time bounds via the song's beat grid: the first
    /// in the song's order whose bounds hold `at`.
    pub fn at(song: &Song, at: Duration) -> Option<SectionBounds> {
        song.sections().iter().find_map(|s| {
            let (start_time, end_time) = song.resolve_section(&s.name)?;
            (at >= start_time && at < end_time).then(|| SectionBounds {
                name: s.name.clone(),
                start_time,
                end_time,
            })
        })
    }
}

/// State machine for reactive section looping.
///
/// In reactive mode, sections are "offered" to the performer as playback
/// enters each section boundary. The performer can ack to arm the loop,
/// and later break to exit.
#[derive(Debug, Clone, Default)]
pub enum ReactiveLoopState {
    /// No section is currently being offered or looped.
    #[default]
    Idle,
    /// Playback entered a section; waiting for performer ack.
    SectionOffered(SectionBounds),
    /// Performer acked; loop will engage at section end.
    LoopArmed(SectionBounds),
    /// Section loop is active.
    Looping(SectionBounds),
    /// Break requested; will exit at end of current iteration.
    BreakRequested(SectionBounds),
}

impl Player {
    /// Creates a new player and spawns asynchronous hardware discovery.
    ///
    /// Returns immediately with no hardware initialized. Device discovery
    /// runs in background tasks that retry perpetually until each device is
    /// found or the init round is cancelled (via `reload_hardware`). Use
    /// `await_hardware_ready()` to wait for init to complete (mainly useful
    /// in tests).
    pub fn new(
        playlists: HashMap<String, Arc<Playlist>>,
        active_playlist: String,
        config: &config::Player,
        base_path: Option<&Path>,
    ) -> Result<Arc<Player>, Box<dyn Error>> {
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };

        let player = Arc::new(Self::new_with_devices(
            devices,
            playlists,
            active_playlist,
            base_path,
        )?);

        // Mark as not ready — async init will set to true when complete.
        // Use send_modify() because send() is a no-op when no receivers exist yet.
        player.init_done_tx.send_modify(|v| *v = false);

        // Spawn async hardware init, capturing the cancellation token now
        // rather than letting the task read it when it is first polled — by
        // then a reload may have replaced it, and the round would adopt its
        // successor's token and never be cancelled.
        let init_player = player.clone();
        let config = config.clone();
        let bp = base_path.map(Path::to_path_buf);
        let init_cancel = player.init_cancel.lock().clone();
        hardware::spawn_after(&player.init_round, async move {
            init_player
                .init_hardware_async(config, bp, init_cancel)
                .await;
        });

        // Spawn the low-rate transport-state poller. Holds a `Weak` so it
        // self-terminates when the player is dropped. The poll cadence
        // (200ms / 5Hz) caps subscribers' notification rate while staying
        // responsive enough for UI use.
        let weak = Arc::downgrade(&player);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(200));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                match weak.upgrade() {
                    Some(p) => p.refresh_transport_state().await,
                    None => break,
                }
            }
        });

        Ok(player)
    }

    /// Creates a new player with pre-constructed devices.
    ///
    /// This is the core constructor used by `new()` after device discovery,
    /// and can be called directly in tests with mock devices.
    pub fn new_with_devices(
        devices: PlayerDevices,
        playlists: HashMap<String, Arc<Playlist>>,
        active_playlist: String,
        base_path: Option<&Path>,
    ) -> Result<Player, Box<dyn Error>> {
        // Store the clock source so each song can create a fresh PlaybackClock.
        let clock_source = match devices
            .audio
            .as_ref()
            .and_then(|d| Some((d.sample_counter()?, d.sample_rate()?)))
        {
            Some((counter, rate)) => ClockSource::Audio {
                sample_counter: counter,
                sample_rate: rate,
            },
            None => ClockSource::Wall,
        };

        // Build track gains from the mapping keys (all at 0 dB) so gain
        // controls work in test/mock-device setups too.
        let track_gains = devices.mappings.as_ref().map(|m| {
            let tg = Arc::new(crate::audio::track_gains::TrackGains::from_config(m, None));
            if let Some(ref audio_device) = devices.audio {
                if let Some(mixer) = audio_device.mixer() {
                    mixer.set_track_gains(tg.clone());
                }
            }
            tg
        });

        let hw = HardwareState {
            device: devices.audio,
            mappings: devices.mappings,
            track_gains,
            midi_device: devices.midi,
            midi_config: None,
            midi_handoff: None,
            dmx_engine: devices.dmx_engine,
            sample_engine: devices.sample_engine,
            trigger_engine: devices.trigger_engine,
            clock_source,
            song_change_notifiers: Vec::new(),
            profile_name: None,
            profile_file: None,
            hostname: None,
            init_errors: HashMap::new(),
            init_warnings: HashMap::new(),
        };

        let (init_done_tx, _init_done_rx) = tokio::sync::watch::channel(true);

        // Resolve the active playlist: use the requested name if it exists, else fall back to all_songs.
        let resolved_active = if playlists.contains_key(&active_playlist) {
            active_playlist
        } else {
            playlist::ALL_SONGS.to_string()
        };

        Ok(Player {
            hardware: Arc::new(parking_lot::RwLock::new(hw)),
            base_path: base_path.map(Path::to_path_buf),
            playlists: Arc::new(parking_lot::RwLock::new(playlists)),
            active_playlist: Arc::new(parking_lot::RwLock::new(resolved_active.clone())),
            playlist_moves: Arc::new(parking_lot::Mutex::new(())),
            persisted_playlist: Arc::new(parking_lot::RwLock::new(
                if resolved_active == playlist::ALL_SONGS {
                    "playlist".to_string()
                } else {
                    resolved_active
                },
            )),
            play_start_time: Arc::new(Mutex::new(None)),
            join: Arc::new(Mutex::new(None)),
            song_playing: Arc::new(AtomicBool::new(false)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span!(Level::INFO, "player"),
            config_store: Arc::new(parking_lot::Mutex::new(None)),
            song_sources: Arc::new(parking_lot::RwLock::new(None)),
            cli_playlist: Arc::new(parking_lot::RwLock::new(None)),
            song_scans: Arc::new(parking_lot::Mutex::new(())),
            ambient_reads: Arc::default(),
            gains_revision: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            lighting_reads: Arc::default(),
            default_metronome: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            init_cancel: Arc::new(parking_lot::Mutex::new(CancellationToken::new())),
            init_round: Arc::new(parking_lot::Mutex::new(None)),
            reload_lock: Arc::new(tokio::sync::Mutex::new(())),
            hardware_reload_pending: Arc::new(AtomicBool::new(false)),
            deferred_reloads: Arc::default(),
            broadcast_tx: Arc::new(parking_lot::Mutex::new(None)),
            init_done_tx: Arc::new(init_done_tx),
            state_tx: Arc::new(parking_lot::Mutex::new(None)),
            transport_tx: Arc::new(tokio::sync::watch::channel(TransportSnapshot::default()).0),
            gain_persist_task: Arc::new(parking_lot::Mutex::new(None)),
            locked: Arc::new(AtomicBool::new(true)),
            controller: Arc::new(parking_lot::Mutex::new(None)),
            transport: Arc::new(parking_lot::Mutex::new(TransportState::default())),
            loop_break: Arc::new(AtomicBool::new(false)),
            ambient_mode: Arc::new(parking_lot::Mutex::new(Default::default())),
            ambient_source: Arc::new(parking_lot::Mutex::new(None)),
            break_fade: Arc::new(parking_lot::Mutex::new(None)),
            break_fade_ids: Arc::new(parking_lot::Mutex::new(Vec::new())),
            start_fade: Arc::new(parking_lot::Mutex::new(None)),
            section_loop: SectionLoop::default(),
            section_jumps: Arc::new(parking_lot::Mutex::new(Default::default())),
            notification_engine: Arc::new(crate::notification::NotificationEngine::with_defaults(
                44100,
            )),
        })
    }

    /// Waits until hardware initialization is complete. Mainly useful in tests.
    #[cfg(test)]
    pub async fn await_hardware_ready(&self) {
        let mut rx = self.init_done_tx.subscribe();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// Gets the audio device currently in use by the player.
    #[cfg(test)]
    pub fn audio_device(&self) -> Option<Arc<dyn audio::Device>> {
        self.hardware.read().device.clone()
    }

    /// Gets the MIDI device currently in use by the player.
    pub fn midi_device(&self) -> Option<Arc<dyn midi::Device>> {
        self.hardware.read().midi_device.clone()
    }

    /// Processes a MIDI event for triggered samples.
    /// This should be called by the MIDI controller when events are received.
    /// Uses std::sync::RwLock for minimal latency (no async overhead).
    pub fn process_sample_trigger(&self, raw_event: &[u8]) {
        let sample_engine = self.hardware.read().sample_engine.clone();
        if let Some(ref sample_engine) = sample_engine {
            let engine = sample_engine.read();
            engine.process_midi_event(raw_event);
        }
    }

    /// Loads the sample configuration for a song.
    /// This preloads samples for the song so they're ready for instant playback.
    /// Note: Active voices continue playing through song transitions.
    fn load_song_samples(&self, song: &Song) {
        let sample_engine = self.hardware.read().sample_engine.clone();
        if let Some(ref sample_engine) = sample_engine {
            // Load the new song's sample config if it has one
            let samples_config = song.samples_config();
            if !samples_config.samples().is_empty() || !samples_config.sample_triggers().is_empty()
            {
                let mut engine = sample_engine.write();
                if let Err(e) = engine.load_song_config(samples_config, song.base_path()) {
                    warn!(
                        song = song.name(),
                        error = %e,
                        "Failed to load song sample config"
                    );
                } else {
                    info!(
                        song = song.name(),
                        samples = samples_config.samples().len(),
                        triggers = samples_config.sample_triggers().len(),
                        "Loaded song sample config"
                    );
                }
            }
        }
    }

    /// Stops all triggered sample playback.
    pub fn stop_samples(&self) {
        let sample_engine = self.hardware.read().sample_engine.clone();
        if let Some(ref sample_engine) = sample_engine {
            let engine = sample_engine.read();
            engine.stop_all();
        }
    }

    /// Gets the DMX engine currently in use by the player, if any.
    pub fn dmx_engine(&self) -> Option<Arc<dmx::engine::Engine>> {
        self.hardware.read().dmx_engine.clone()
    }

    /// Gets all cues from the current song's lighting timeline.
    pub fn get_cues(&self) -> Vec<(Duration, usize)> {
        let dmx_engine = self.hardware.read().dmx_engine.clone();
        if let Some(ref dmx_engine) = dmx_engine {
            dmx_engine.get_timeline_cues()
        } else {
            Vec::new()
        }
    }

    /// Returns handles needed for reading lighting state, or None if no DMX engine is configured.
    pub fn broadcast_handles(&self) -> Option<dmx::engine::BroadcastHandles> {
        self.hardware
            .read()
            .dmx_engine
            .clone()
            .map(|e| e.broadcast_handles())
    }

    /// Re-reads the venues from disk, re-registers the current venue's
    /// fixtures with the running DMX engine, and tells every web client
    /// the stage geometry changed. The venue file is the durable truth;
    /// this is how an edit to it (positions, focus points, tags) reaches a
    /// running player without a hardware reload. A player with no DMX
    /// engine has nothing to reload, which is not an error.
    pub fn reload_current_venue(&self) -> Result<(), String> {
        let Some(engine) = self.dmx_engine() else {
            return Ok(());
        };
        engine.reload_current_venue().map_err(|e| e.to_string())?;
        let metadata = crate::webui::state::build_metadata_json(
            engine.broadcast_handles().lighting_system.as_ref(),
        );
        if let Some(tx) = self.broadcast_tx.lock().as_ref() {
            // No receivers is fine: nobody is watching the stage right now.
            let _ = tx.send(metadata);
        }
        Ok(())
    }

    /// [`Self::reload_current_venue`] for an edit to a fixture type: re-reads
    /// the types and the venues, re-registers the current venue's fixtures
    /// and tells every web client the stage changed. No DMX engine is not an
    /// error.
    pub fn reload_fixture_types(&self) -> Result<(), String> {
        let Some(engine) = self.dmx_engine() else {
            return Ok(());
        };
        engine.reload_fixture_types().map_err(|e| e.to_string())?;
        let metadata = crate::webui::state::build_metadata_json(
            engine.broadcast_handles().lighting_system.as_ref(),
        );
        if let Some(tx) = self.broadcast_tx.lock().as_ref() {
            let _ = tx.send(metadata);
        }
        Ok(())
    }

    /// Stores the broadcast channel and wires it to the DMX engine if one exists.
    /// If the DMX engine hasn't initialized yet, the channel is stored and will
    /// be wired when the engine comes up during async init.
    pub fn set_broadcast_tx(&self, tx: tokio::sync::broadcast::Sender<String>) {
        let dmx_engine = self.hardware.read().dmx_engine.clone();
        if let Some(ref engine) = dmx_engine {
            engine.set_broadcast_tx(tx.clone());
        }
        *self.broadcast_tx.lock() = Some(tx);
    }

    /// Stores the state sampler watch sender. When the DMX engine comes up
    /// during async init, the sampler will be started using this sender.
    pub fn set_state_tx(&self, tx: tokio::sync::watch::Sender<Arc<crate::state::StateSnapshot>>) {
        *self.state_tx.lock() = Some(Arc::new(tx));
    }

    /// Returns a fresh state-watch receiver, if a state sender has been wired.
    /// Each call produces an independent receiver via `watch::Sender::subscribe`.
    pub fn state_rx(
        &self,
    ) -> Option<tokio::sync::watch::Receiver<Arc<crate::state::StateSnapshot>>> {
        self.state_tx.lock().as_ref().map(|tx| tx.subscribe())
    }

    /// Returns the state-watch sender, if one has been wired. Lets callers
    /// push state snapshots into the player from outside the normal sampler
    /// path — primarily useful for tests, but also for headless integrations
    /// that want to drive the state stream without a DMX engine running.
    pub fn state_tx(
        &self,
    ) -> Option<Arc<tokio::sync::watch::Sender<Arc<crate::state::StateSnapshot>>>> {
        self.state_tx.lock().clone()
    }

    /// Returns a fresh receiver for the transport-state watch channel.
    /// The channel only emits a "changed" wake-up when the underlying
    /// [`TransportSnapshot`] transitions to a new value.
    pub fn transport_rx(&self) -> tokio::sync::watch::Receiver<TransportSnapshot> {
        self.transport_tx.subscribe()
    }

    /// Computes the current transport snapshot from live player state: the
    /// player's status, without its DMX reads.
    ///
    /// `is_playing()` briefly acquires the play-join lock, so this should
    /// not be called from contexts that hold that lock.
    pub async fn compute_transport_snapshot(&self) -> TransportSnapshot {
        self.transport_status().await.transport()
    }

    /// Recomputes the transport snapshot and broadcasts it iff it differs
    /// from the last published value. Safe to call repeatedly — only real
    /// transitions wake subscribers.
    pub async fn refresh_transport_state(&self) {
        let next = self.compute_transport_snapshot().await;
        self.transport_tx.send_if_modified(|cur| {
            if *cur != next {
                *cur = next.clone();
                true
            } else {
                false
            }
        });
    }

    /// Sets the config store on the player. Called once after startup.
    /// The song sources are resolved from it at once.
    pub fn set_config_store(&self, store: Arc<config::ConfigStore>) {
        if let Some(config) = store.try_read_config() {
            *self.song_sources.write() = Some(SongSources::resolve(
                &config,
                store.path(),
                self.cli_playlist.read().as_deref(),
            ));
        }
        *self.config_store.lock() = Some(store);
    }

    /// Returns the config store, if one has been set.
    pub fn config_store(&self) -> Option<Arc<config::ConfigStore>> {
        self.config_store.lock().clone()
    }

    /// Sets where songs and playlists are read from, for a player with no
    /// config store to resolve them from (a test's). A config store, once
    /// set, resolves them in their place.
    pub fn set_song_sources(&self, sources: SongSources) {
        *self.song_sources.write() = Some(sources);
    }

    /// Sets the legacy playlist named on the command line, which wins over
    /// the config's each time the sources are resolved. Set before the
    /// config store.
    pub fn set_cli_playlist(&self, playlist: Option<PathBuf>) {
        *self.cli_playlist.write() = playlist;
    }

    /// Where songs and playlists are read from, as last resolved.
    pub fn song_sources(&self) -> Option<SongSources> {
        self.song_sources.read().clone()
    }

    /// The song sources the config store's current config names, with the
    /// command line's playlist over its own, without taking them up: they
    /// become the player's only once songs have been reloaded from them, so
    /// a `songs:` naming a directory that cannot be read leaves every surface
    /// on the songs it has. Without a config store, the sources set.
    pub async fn resolve_song_sources(&self) -> Option<SongSources> {
        let Some(store) = self.config_store() else {
            return self.song_sources();
        };
        let config = store.read_config().await;
        Some(SongSources::resolve(
            &config,
            store.path(),
            self.cli_playlist.read().as_deref(),
        ))
    }

    /// [`SongSources::prepare`] for a config about to be saved to this
    /// player's config store, with the command line's playlist.
    /// Only when the songs or playlists `config` names are not the ones the
    /// player has: a save that leaves them as they are is not refused for a
    /// location already in use that has since become unreadable. Returns the
    /// songs directory if this made it.
    pub fn prepare_song_sources(&self, config: &config::Player) -> Result<Option<PathBuf>, String> {
        let store = self.config_store().ok_or("No config store available")?;
        let cli = self.cli_playlist.read().clone();
        let resolved = SongSources::resolve(config, store.path(), cli.as_deref());
        if Some(&resolved) == self.song_sources().as_ref() {
            return Ok(None);
        }
        SongSources::prepare(config, store.path(), cli.as_deref()).map(|(_, made)| made)
    }

    /// Sets the player-wide "metronome on by default" flag. Takes effect on
    /// the next song (re)load.
    pub fn set_default_metronome(&self, enabled: bool) {
        self.default_metronome
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns the player-wide "metronome on by default" flag.
    pub fn default_metronome(&self) -> bool {
        self.default_metronome
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Returns the track-to-output-channel mappings, if audio is configured.
    pub fn track_mappings(&self) -> Option<Arc<HashMap<String, Vec<u16>>>> {
        self.hardware.read().mappings.clone()
    }

    /// Sets the gain of an output track in dB, returning the (clamped)
    /// applied value. Takes effect on the next audio callback for playing
    /// sources and at source start otherwise, so gains can be preset while
    /// stopped. Allowed while locked — this is a performance control, like
    /// play/stop.
    ///
    /// The new gains are persisted to the active profile after a short
    /// debounce so rapid fader moves produce a single disk write.
    pub fn set_track_gain(&self, track: &str, gain_db: f32) -> Result<f32, TrackGainError> {
        if !gain_db.is_finite() {
            return Err(TrackGainError::NonFinite(gain_db));
        }

        let (track_gains, hostname) = {
            let hw = self.hardware.read();
            (hw.track_gains.clone(), hw.hostname.clone())
        };
        let track_gains = track_gains.ok_or(TrackGainError::NoAudioProfile)?;
        let before = track_gains.get_db(track);
        let applied = track_gains.set_db(track, gain_db)?;
        if before != Some(applied) {
            self.bump_gains_revision();
        }

        // Debounced persistence: replace any pending save with a fresh timer.
        match (self.config_store(), hostname) {
            (Some(store), Some(hostname)) => {
                let gains = track_gains.snapshot_db_map();
                let mut pending = self.gain_persist_task.lock();
                if let Some(task) = pending.take() {
                    task.abort();
                }
                *pending = Some(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(750)).await;
                    if let Err(e) = store.set_track_gains(&hostname, gains).await {
                        error!(err = %e, "Failed to persist track gains");
                    }
                }));
            }
            _ => {
                debug!(
                    track,
                    gain_db = applied,
                    "track gain applied but not persisted (no config store or hostname)"
                );
            }
        }

        Ok(applied)
    }

    /// The count of changes to any track's gain or mute so far; it only
    /// matters when it differs from the last one read.
    pub fn gains_revision(&self) -> u32 {
        self.gains_revision.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_gains_revision(&self) {
        self.gains_revision.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns all output track gains as (name, dB) pairs, or None when no
    /// audio profile is active.
    pub fn get_track_gains(&self) -> Option<Vec<(String, f32)>> {
        self.hardware
            .read()
            .track_gains
            .as_ref()
            .map(|tg| tg.snapshot_db())
    }

    /// Mutes or unmutes an output track without changing its gain, returning
    /// the applied state. Mute is a runtime performance control and is never
    /// persisted — a restart starts unmuted.
    pub fn set_track_mute(&self, track: &str, muted: bool) -> Result<bool, TrackGainError> {
        let track_gains = self
            .hardware
            .read()
            .track_gains
            .clone()
            .ok_or(TrackGainError::NoAudioProfile)?;
        let before = track_gains.get_muted(track);
        let applied = track_gains.set_muted(track, muted)?;
        if before != Some(applied) {
            self.bump_gains_revision();
        }
        info!(track, muted = applied, "track mute changed");
        Ok(applied)
    }

    /// Returns all output track mute states as (name, muted) pairs, or None
    /// when no audio profile is active.
    pub fn get_track_mutes(&self) -> Option<Vec<(String, bool)>> {
        self.hardware
            .read()
            .track_gains
            .as_ref()
            .map(|tg| tg.snapshot_muted())
    }

    /// Returns true if the player is in locked mode (state-altering operations blocked).
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Refuses a state-altering operation while the player is locked. Every
    /// control surface calls this (or the web UI's lock middleware) before a
    /// config, song, playlist or lighting write. Playback controls never do.
    pub fn require_unlocked(&self) -> Result<(), LockedError> {
        if self.is_locked() {
            Err(LockedError)
        } else {
            Ok(())
        }
    }

    /// Sets the locked state. When locked, state-altering operations are rejected
    /// but playback controls continue to work.
    pub fn set_locked(&self, locked: bool) {
        self.locked.store(locked, Ordering::Relaxed);
        // A fixture test is an edit-time tool: locking for the show ends it.
        if locked {
            if let Some(engine) = self.dmx_engine() {
                engine.release_test_output();
            }
        }
    }

    /// Returns the effect engine, if a DMX engine is configured.
    pub fn effect_engine(&self) -> Option<Arc<parking_lot::Mutex<crate::lighting::EffectEngine>>> {
        self.hardware
            .read()
            .dmx_engine
            .clone()
            .map(|e| e.effect_engine())
    }

    /// Gets a formatted string listing all active lighting effects
    pub fn format_active_effects(&self) -> Option<String> {
        self.hardware
            .read()
            .dmx_engine
            .clone()
            .map(|engine| engine.format_active_effects())
    }

    /// The active lighting effects as text for a surface to show:
    /// [`Player::format_active_effects`], or [`NO_EFFECT_ENGINE`] without one.
    pub fn active_effects_summary(&self) -> String {
        self.format_active_effects()
            .unwrap_or_else(|| NO_EFFECT_ENGINE.to_string())
    }

    /// Emits the per-song MIDI event and notifies all song-change notifiers.
    fn emit_song_change(&self, song: &Song) {
        let hw = self.hardware.read();
        let midi_device = hw.midi_device.clone();
        let notifiers = hw.song_change_notifiers.clone();
        drop(hw);

        if let Some(ref device) = midi_device {
            if let Err(e) = device.emit(song.midi_event()) {
                error!("Error emitting MIDI event: {:?}", e);
            }
        }
        for notifier in &notifiers {
            notifier.notify(song);
        }
    }
}

/// Describes how to report status via MIDI.
pub struct StatusEvents {
    /// The events to emit to clear the status.
    off_events: Vec<LiveEvent<'static>>,
    /// The events to emit to indicate that the player is idling and waiting for input.
    idling_events: Vec<LiveEvent<'static>>,
    /// The events to emit to indicate that the player is currently playing.
    playing_events: Vec<LiveEvent<'static>>,
    /// The events to emit, in place of the idling events, while an auto
    /// play waits; empty for the idling events.
    auto_play_events: Vec<LiveEvent<'static>>,
}

impl StatusEvents {
    /// Creates a new status events configuration.
    pub fn new(
        config: Option<config::StatusEvents>,
    ) -> Result<Option<StatusEvents>, Box<dyn Error>> {
        Ok(match config {
            Some(config) => Some(StatusEvents {
                off_events: config.off_events()?,
                idling_events: config.idling_events()?,
                playing_events: config.playing_events()?,
                auto_play_events: config.auto_play_events()?,
            }),
            None => None,
        })
    }

    /// The events that report the player's state: playing, an auto play
    /// waiting to start (when it has events of its own), or idling.
    fn current(&self, playing: bool, auto_play_waiting: bool) -> &[LiveEvent<'static>] {
        if playing {
            &self.playing_events
        } else if auto_play_waiting && !self.auto_play_events.is_empty() {
            &self.auto_play_events
        } else {
            &self.idling_events
        }
    }
}

/// A playlist entry's `auto_play`, waiting out its gap: each schedule takes
/// the next generation, and a wait whose generation has moved on, or that
/// was called off, plays nothing.
#[derive(Default)]
struct AutoPlayState {
    generation: u64,
    /// The song waiting and when it starts, while one is.
    waiting: Option<(String, std::time::Instant)>,
}

/// What a loop break asked of the looping song playing does once its loop
/// lets go: nothing asked, move on to the next entry, move back to the one
/// before, or end the song where it is (a stop, or next or previous at the
/// end of the list), starting nothing. One value, so a break cannot be both
/// an ending and a move back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LoopExit {
    #[default]
    None,
    Next,
    Prev,
    End,
}

/// The transport's state machine, kept behind the player's one transport
/// lock (see `Player::transport` for the lock ordering).
#[derive(Default)]
struct TransportState {
    /// What a loop break asked of the song playing does. Mirrored into
    /// `Player::loop_break` for the engines.
    loop_exit: LoopExit,
    /// A looping song ending, fading out with nothing after it, if one is.
    ending: EndingState,
    /// A playlist entry's `auto_play` waiting out its gap, if one is.
    auto_play: AutoPlayState,
    /// Start position for the next play(), set by a pause or by seeking
    /// while stopped. Cleared when the playlist position changes.
    pending_start: Option<seek::PendingStart>,
    /// The song at the playlist's entry played to its end with nowhere to
    /// move on to: the gap is after the last entry, not before it, and the
    /// playlist's ambient light plays rather than the entry's.
    past_last_entry: bool,
    /// When a song fading out after a loop break, or as it ends, is out,
    /// audio and lights both. A hardware reload waits for it.
    fading_until: Option<std::time::Instant>,
}

impl TransportState {
    /// The looping song ending, and how long until its fade is over, while
    /// one is.
    fn ending_status(&self) -> Option<(String, Duration)> {
        self.ending.fading.as_ref().map(|(song, at)| {
            (
                song.clone(),
                at.saturating_duration_since(std::time::Instant::now()),
            )
        })
    }

    /// The song an auto play is waiting to start, and how long until it
    /// does, while one is.
    fn auto_play_status(&self) -> Option<(String, Duration)> {
        self.auto_play.waiting.as_ref().map(|(song, at)| {
            (
                song.clone(),
                at.saturating_duration_since(std::time::Instant::now()),
            )
        })
    }

    /// Whether a song is paused: stopped by a pause, with a position to
    /// resume from.
    fn is_paused(&self) -> bool {
        self.pending_start.is_some_and(|p| p.paused)
    }

    /// How long until a song fading out is out, while one is: a looping
    /// song ending, or one a loop break moved on from. A hardware reload
    /// waits for it, as the reset would cut it short.
    fn fading_out(&self) -> Option<Duration> {
        let ending = self.ending_status().map(|(_, left)| left);
        let fading = self
            .fading_until
            .map(|until| until.saturating_duration_since(std::time::Instant::now()))
            .filter(|left| !left.is_zero());
        ending.max(fading)
    }
}

/// A looping song ending: each ending takes the next generation, and one
/// whose generation has moved on, or that was cut, brings no ambient light
/// when its fade would have ended.
#[derive(Default)]
struct EndingState {
    generation: u64,
    /// The song fading out and when its fade ends, while one is.
    fading: Option<(String, std::time::Instant)>,
}

/// The result of receiving a playback completion signal.
#[derive(Debug)]
enum PlaybackResult {
    Success,
    Failed(String),
    SenderDropped,
}

/// What the cleanup task should do after playback finishes.
#[derive(Debug, PartialEq)]
enum CleanupAction {
    AdvancePlaylist,
    StopCancelled,
    /// Loop was broken via play/next — advance playlist and auto-play next song.
    LoopBreakAndPlay,
}

/// Decides whether to advance the playlist or stop after playback finishes.
fn decide_cleanup_action(
    result: PlaybackResult,
    cancelled: bool,
    loop_broken: bool,
    self_cancelled: bool,
) -> CleanupAction {
    // Loop break takes priority over cancel — we intentionally cancel
    // playback to break out of a loop immediately, but the intent is
    // to advance and play, not to stop.
    if loop_broken {
        return CleanupAction::LoopBreakAndPlay;
    }
    // Playback that cancelled *itself* — the audio callback stopped mid-song, so
    // the audio thread cancelled the handle to bring MIDI and DMX down off a
    // frozen clock. `StopCancelled` would be wrong: it skips the cleanup on the
    // grounds that `stop()` already did it, and nothing called `stop()`, so
    // `join` and `play_start_time` would stay set and the player would believe a
    // song was playing forever.
    //
    // Deliberately keyed on *who cancelled*, not on whether there was also an
    // error. A seek or a stop cancels an in-flight playback that can report an
    // error on its way out, and treating that as a self-cancellation lets this
    // cleanup clear state belonging to the playback the seek has already
    // started — which is precisely what the `StopCancelled` early return exists
    // to prevent.
    if self_cancelled {
        warn!("Playback ended itself; advancing playlist so the player is not left wedged");
        return CleanupAction::AdvancePlaylist;
    }
    if cancelled {
        return CleanupAction::StopCancelled;
    }
    match &result {
        PlaybackResult::Failed(e) => {
            warn!(
                err = %e,
                "Advancing playlist despite playback failure so user is not stuck"
            );
        }
        PlaybackResult::SenderDropped => {
            error!("Error receiving playback signal (receiver dropped)");
        }
        PlaybackResult::Success => {}
    }
    CleanupAction::AdvancePlaylist
}

/// Resolves the final playback outcome from the audio thread result.
fn resolve_playback_outcome(
    has_audio: bool,
    audio_outcome: Option<Result<(), String>>,
) -> Result<(), String> {
    if has_audio {
        audio_outcome.unwrap_or_else(|| {
            warn!(
                "Audio thread did not set outcome (e.g. panicked before setting); \
                 treating as success so playlist is not stuck"
            );
            Ok(())
        })
    } else {
        Ok(())
    }
}

/// Where the player's songs and playlists are read from, resolved from the
/// config by one resolver, [`SongSources::resolve`], whichever surface asks.
/// Resolved per surface, the web UI and MCP reloaded different sets after a
/// config change, MCP ignored a playlist named on the command line, and the
/// web UI loaded a `playlist.yaml` beside the config that nothing had
/// configured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongSources {
    /// The songs directory, resolved.
    pub songs: PathBuf,
    /// The playlists directory: the configured `playlists_dir`, else
    /// `playlists/` beside the config.
    pub playlists_dir: Option<PathBuf>,
    /// The legacy single playlist file, if one is configured (the command
    /// line's over the config's), made absolute against the config's
    /// directory.
    pub legacy_playlist: Option<PathBuf>,
}

impl SongSources {
    /// Checks the songs and playlists `config` names can be used, before a
    /// config naming them is saved or taken up, and makes the songs
    /// directory where startup would (inside the project): a config whose
    /// songs cannot be read is refused, rather than saved with the player
    /// left on songs from somewhere else. Returns the sources, resolved,
    /// and the songs directory if this made it, for a save that does not
    /// happen after all to take away again.
    pub fn prepare(
        config: &config::Player,
        config_path: &Path,
        cli_playlist: Option<&Path>,
    ) -> Result<(SongSources, Option<PathBuf>), String> {
        let mut sources = SongSources::resolve(config, config_path, cli_playlist);
        let made = (!sources.songs.exists()).then(|| sources.songs.clone());
        let project = crate::util::project_dir_of(config_path);
        let created = crate::util::create_dir_within(&sources.songs, &project)
            .map_err(|e| format!("songs: {e}"))?;
        std::fs::read_dir(created.as_path())
            .map_err(|e| format!("songs: {}: {e}", created.as_path().display()))?;
        sources.songs = crate::util::resolved_dir(created.as_path());
        if let Some(dir) = &sources.playlists_dir {
            if dir.exists() {
                std::fs::read_dir(dir)
                    .map_err(|e| format!("playlists_dir: {}: {e}", dir.display()))?;
            }
        }
        if let Some(file) = &sources.legacy_playlist {
            if file.exists() && !file.is_file() {
                return Err(format!("playlist: {} is not a file", file.display()));
            }
        }
        let made = made.map(|_| sources.songs.clone());
        Ok((sources, made))
    }

    /// Resolves the sources from `config`, read from `config_path`: the
    /// songs directory it names, resolved as far as the filesystem allows,
    /// and the playlist named on the command line, if one is, over the
    /// config's.
    pub fn resolve(
        config: &config::Player,
        config_path: &Path,
        cli_playlist: Option<&Path>,
    ) -> SongSources {
        let project_dir = crate::util::project_dir_of(config_path);
        let playlists_dir = config
            .playlists_dir(config_path)
            .unwrap_or_else(|| project_dir.join("playlists"));
        let legacy_playlist = cli_playlist
            .map(Path::to_path_buf)
            .or_else(|| config.playlist())
            .map(|path| project_dir.join(path));
        SongSources {
            songs: crate::util::resolved_dir(&config.songs(config_path)),
            playlists_dir: Some(playlists_dir),
            legacy_playlist,
        }
    }
}

/// Why [`Player::switch_to_playlist`] did not switch. Matched by the
/// surfaces to choose their answer, never by its message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SwitchPlaylistError {
    #[error("Cannot switch playlist while playing")]
    Playing,
    #[error("Playlist '{0}' not found")]
    NotFound(String),
}

impl SwitchPlaylistError {
    /// What kind of fault this is, for a transport's code.
    pub fn kind(&self) -> crate::util::FaultKind {
        match self {
            SwitchPlaylistError::Playing => crate::util::FaultKind::Precondition,
            SwitchPlaylistError::NotFound(_) => crate::util::FaultKind::NotFound,
        }
    }
}

/// Why [`Player::reload_songs`] left the songs as they were.
#[derive(Debug, thiserror::Error)]
pub enum ReloadSongsError {
    /// The player was given no song sources and has no config store to
    /// resolve them from.
    #[error("no songs directory is configured for the player")]
    NoSources,
    #[error("failed to rescan songs: {0}")]
    Songs(String),
    #[error("failed to rebuild playlists: {0}")]
    Playlists(String),
    #[error("the song reload did not finish: {0}")]
    Interrupted(String),
}

/// Loads all playlists from a directory and/or legacy playlist path.
/// Always includes the computed [`playlist::ALL_SONGS`] playlist.
pub fn load_playlists(
    playlists_dir: Option<&Path>,
    legacy_playlist_path: Option<&Path>,
    songs: Arc<Songs>,
) -> Result<HashMap<String, Arc<Playlist>>, Box<dyn Error>> {
    let mut playlists = HashMap::new();

    // Always create all_songs.
    playlists.insert(
        playlist::ALL_SONGS.to_string(),
        playlist::from_songs(songs.clone())?,
    );

    // Load playlists from directory.
    if let Some(dir) = playlists_dir {
        if dir.is_dir() {
            // Each name's file (`playlist::playlist_files`): with both
            // `<name>.yaml` and `<name>.yml`, the `.yaml`, with a warning.
            for playlist::PlaylistFile { name, path, unread } in playlist::playlist_files(dir)? {
                if let Some(unread) = unread {
                    warn!(
                        name = %name,
                        "Playlist has two files: {} is read, {} is not",
                        path.display(),
                        unread.display()
                    );
                }
                match config::Playlist::deserialize(&path) {
                    Ok(playlist_config) => {
                        match Playlist::new(&name, &playlist_config, songs.clone()) {
                            Ok(pl) => {
                                info!(name = %name, "Loaded playlist from directory");
                                playlists.insert(name, pl);
                            }
                            Err(e) => {
                                warn!(name = %name, error = %e, "Playlist references missing songs, skipping");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(path = ?path, error = %e, "Failed to parse playlist file, skipping");
                    }
                }
            }
        }
    }

    // Load legacy playlist file as name "playlist" (if not already loaded from dir).
    if let Some(legacy_path) = legacy_playlist_path {
        if !playlists.contains_key("playlist") {
            match config::Playlist::deserialize(legacy_path) {
                Ok(playlist_config) => {
                    match Playlist::new("playlist", &playlist_config, songs.clone()) {
                        Ok(pl) => {
                            info!("Loaded legacy playlist");
                            playlists.insert("playlist".to_string(), pl);
                        }
                        Err(e) => {
                            info!("Legacy playlist references missing songs ({}); skipping", e);
                        }
                    }
                }
                Err(_) => {
                    info!("Legacy playlist file not found or invalid; skipping");
                }
            }
        }
    }

    Ok(playlists)
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, fs, path::Path, sync::Arc};

    use crate::{
        config,
        playlist::Playlist,
        songs,
        testutil::{
            eventually, eventually_async,
            player::{assets_player, assets_player_with_devices, no_devices},
        },
    };

    use super::*;

    /// Test helper: builds a playlists map from a single playlist + songs registry.
    fn test_playlists(
        playlist: Arc<Playlist>,
        songs: Arc<Songs>,
    ) -> HashMap<String, Arc<Playlist>> {
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs).unwrap(),
        );
        playlists.insert("playlist".to_string(), playlist);
        playlists
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player() -> Result<(), Box<dyn Error>> {
        let player = assets_player().midi("mock-midi-device").build()?;
        player.await_hardware_ready().await;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;
        let midi_device = player
            .midi_device()
            .expect("MIDI should be present")
            .to_mock()?;

        // Direct the player.
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.next().await;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        player.prev().await;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        println!("Switch to AllSongs");
        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.next().await;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 10");

        // No emitted events yet
        assert!(midi_device.get_emitted_event().is_none());

        player.next().await;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 2");

        let expected_event = midly::live::LiveEvent::Midi {
            channel: 15.into(),
            message: midly::MidiMessage::ProgramChange { program: 0.into() },
        };
        let actual_event_buf = midi_device
            .get_emitted_event()
            .expect("expected emitted event");
        let actual_event = midly::live::LiveEvent::parse(&actual_event_buf)?;
        assert_eq!(expected_event, actual_event);

        midi_device.reset_emitted_event();

        player.next().await;
        println!("AllSongs -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        assert!(midi_device.get_emitted_event().is_none());

        player.switch_to_playlist("playlist").await.unwrap();
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.next().await;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        player.play().await?;

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().unwrap().name()
            )
            .as_str(),
        );

        // Next song should have emitted an event.
        let expected_event = midly::live::LiveEvent::Midi {
            channel: 15.into(),
            message: midly::MidiMessage::ProgramChange { program: 5.into() },
        };
        let actual_event_buf = midi_device
            .get_emitted_event()
            .expect("expected emitted event");
        let actual_event = midly::live::LiveEvent::parse(&actual_event_buf)?;
        assert_eq!(expected_event, actual_event);

        midi_device.reset_emitted_event();

        // Play a song and cancel it.
        player.play().await?;
        println!("Play Song 5.");
        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        // Player should not have moved to the next song.
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 5");

        assert!(midi_device.get_emitted_event().is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player_rejects_invalid_lighting_shows() -> Result<(), Box<dyn Error>> {
        // Create a temporary directory for test files
        let temp_dir = tempfile::tempdir()?;
        let temp_path = temp_dir.path();

        // Create a valid lighting show file with invalid group reference
        let lighting_show_content = r#"show "Test Show" {
    @00:00.000
    invalid_group: static color: "blue", duration: 5s, dimmer: 60%
}"#;
        let lighting_file = temp_path.join("invalid_show.light");
        fs::write(&lighting_file, lighting_show_content)?;

        // Create a song with the invalid lighting show
        let song_config = config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            Some(vec![config::LightingShow::new(
                lighting_file
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            )]),
            vec![],
            HashMap::new(),
            Vec::new(),
        );

        // Create a lighting config with valid groups (but not "invalid_group")
        let mut groups = HashMap::new();
        groups.insert(
            "front_wash".to_string(),
            config::lighting::LogicalGroup::new(
                "front_wash".to_string(),
                vec![config::lighting::GroupConstraint::AllOf(vec![
                    "wash".to_string(),
                    "front".to_string(),
                ])],
            ),
        );
        let lighting_config =
            config::Lighting::new(Some("test_venue".to_string()), Some(groups), None);

        // Create DMX config with lighting
        let dmx_config = config::Dmx::new(
            Some(1.0),
            Some("0s".to_string()),
            Some(9090),
            vec![config::Universe::new(1, "test_universe".to_string())],
            Some(lighting_config),
        );

        // Create a simple playlist with one song
        let playlist_songs = vec!["Test Song".to_string()];
        let playlist_config = config::Playlist::new(&playlist_songs);
        let song = songs::Song::new(temp_path, &song_config)?;
        let songs_map = HashMap::from([("Test Song".to_string(), Arc::new(song))]);
        let songs = Arc::new(songs::Songs::new(songs_map));
        let playlist = Playlist::new("Test Playlist", &playlist_config, songs.clone())?;

        // Create player with DMX engine that has lighting config
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                Some(dmx_config),
                HashMap::new(),
                temp_path.to_str().unwrap(),
            ),
            Some(temp_path),
        )?;
        player.await_hardware_ready().await;

        // Try to play the song - it should fail due to invalid lighting show
        let result = player.play().await;
        assert!(
            result.is_err(),
            "Player should reject song with invalid lighting show"
        );

        Ok(())
    }

    /// Waits until the player reports an elapsed position, and returns it.
    ///
    /// A start stamps `play_start_time` as it goes in, and `play_files` again
    /// as the clock starts; one started some other way (an auto play) may
    /// not have gone in yet when `is_playing` turns true.
    async fn elapsed_eventually(player: &Arc<Player>) -> Result<Duration, Box<dyn Error>> {
        let deadline = std::time::Instant::now() + crate::testutil::PATIENCE;
        loop {
            if let Some(elapsed) = player.elapsed().await? {
                return Ok(elapsed);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "player never reported an elapsed position"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Helper to create a player with the standard test assets (audio + MIDI).
    async fn make_test_player() -> Result<Arc<Player>, Box<dyn Error>> {
        assets_player().midi("mock-midi-device").ready().await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_when_not_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // stop() when nothing is playing should return None.
        let result = player.stop().await;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_is_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert!(!player.is_playing().await);

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        assert!(player.is_playing().await);

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_elapsed_stopped() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // elapsed() when not playing should be Ok(None).
        let elapsed = player.elapsed().await?;
        assert!(elapsed.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_elapsed_while_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        elapsed_eventually(&player).await?;

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_concurrent_play_returns_none() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // First play should succeed.
        let result = player.play().await?;
        assert!(result.is_some());
        eventually(|| device.is_playing(), "Song never started playing");

        // Second play while already playing should return Ok(None).
        let result = player.play().await?;
        assert!(result.is_none(), "play() while playing should return None");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_navigation() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        let song = player.next().await.unwrap();
        assert_eq!(song.name(), "Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        let song = player.prev().await.unwrap();
        assert_eq!(song.name(), "Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_playlists() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        assert_eq!(player.get_playlist().name(), "playlist");
        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.get_playlist().name(), "all_songs");
        player.switch_to_playlist("playlist").await.unwrap();
        assert_eq!(player.get_playlist().name(), "playlist");

        Ok(())
    }

    /// `refresh_transport_state` must broadcast only when the snapshot
    /// actually changes — repeated refreshes against unchanged state
    /// must not wake subscribers. This is what keeps `mtrack://status`
    /// subscribers quiet during steady-state playback.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_transport_channel_coalesces_no_op_refreshes() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // Prime the channel so it reflects the real (post-init) snapshot,
        // not the default value the watch was constructed with.
        player.refresh_transport_state().await;
        let mut rx = player.transport_rx();
        rx.mark_unchanged();

        // Refresh repeatedly against unchanged state. None should wake the receiver.
        for _ in 0..10 {
            player.refresh_transport_state().await;
        }
        let result = tokio::time::timeout(Duration::from_millis(50), rx.changed()).await;
        assert!(
            result.is_err(),
            "transport channel emitted on no-op refresh: {:?}",
            rx.borrow().clone()
        );

        // A real transport transition must wake the receiver.
        player.switch_to_playlist("all_songs").await.unwrap();
        player.refresh_transport_state().await;
        let result = tokio::time::timeout(crate::testutil::PATIENCE, rx.changed()).await;
        assert!(result.is_ok(), "transport channel did not emit on switch");
        assert_eq!(rx.borrow().playlist_name, "all_songs");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_all_songs_playlist() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let all = player
            .get_all_songs_playlist()
            .expect("all_songs should be present in test");
        assert_eq!(all.name(), "all_songs");
        assert!(!all.songs().is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_format_active_effects_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // No DMX engine configured, should return None.
        assert!(player.format_active_effects().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dmx_engine_none_without_config() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.dmx_engine().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_cues_empty_without_lighting() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let cues = player.get_cues();
        assert!(cues.is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player_rejects_song_with_multiple_invalid_groups() -> Result<(), Box<dyn Error>> {
        // Create a temporary directory for test files
        let temp_dir = tempfile::tempdir()?;
        let temp_path = temp_dir.path();

        // Create a valid lighting show file with multiple invalid group references
        let lighting_show_content = r#"show "Test Show" {
    @00:00.000
    invalid_group_1: static color: "blue", duration: 5s, dimmer: 60%
    invalid_group_2: static color: "red", duration: 5s, dimmer: 80%
}"#;
        let lighting_file = temp_path.join("invalid_groups.light");
        fs::write(&lighting_file, lighting_show_content)?;

        // Create a song with the invalid lighting show
        let song_config = config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            Some(vec![config::LightingShow::new(
                lighting_file
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            )]),
            vec![],
            HashMap::new(),
            Vec::new(),
        );

        // Create a lighting config with valid groups (but not the invalid ones)
        let mut groups = HashMap::new();
        groups.insert(
            "front_wash".to_string(),
            config::lighting::LogicalGroup::new(
                "front_wash".to_string(),
                vec![config::lighting::GroupConstraint::AllOf(vec![
                    "wash".to_string(),
                    "front".to_string(),
                ])],
            ),
        );
        let lighting_config =
            config::Lighting::new(Some("test_venue".to_string()), Some(groups), None);

        // Create DMX config with lighting
        let dmx_config = config::Dmx::new(
            Some(1.0),
            Some("0s".to_string()),
            Some(9090),
            vec![config::Universe::new(1, "test_universe".to_string())],
            Some(lighting_config),
        );

        // Create a simple playlist with one song
        let playlist_songs = vec!["Test Song".to_string()];
        let playlist_config = config::Playlist::new(&playlist_songs);
        let song = songs::Song::new(temp_path, &song_config)?;
        let songs_map = HashMap::from([("Test Song".to_string(), Arc::new(song))]);
        let songs = Arc::new(songs::Songs::new(songs_map));
        let playlist = Playlist::new("Test Playlist", &playlist_config, songs.clone())?;

        // Create player with DMX engine that has lighting config
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                Some(dmx_config),
                HashMap::new(),
                temp_path.to_str().unwrap(),
            ),
            Some(temp_path),
        )?;
        player.await_hardware_ready().await;

        // Try to play the song - it should fail due to invalid lighting show groups
        let result = player.play().await;
        assert!(
            result.is_err(),
            "Player should reject song with invalid lighting show groups"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_returns_current_song() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let song = player.stop().await;
        assert!(song.is_some(), "stop() should return the current song");
        assert_eq!(song.unwrap().name(), "Song 1");

        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pause_stops_and_preserves_position_for_resume() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        tokio::time::sleep(Duration::from_millis(50)).await;

        let paused_at = player.pause().await?.expect("playing song should pause");
        eventually(|| !device.is_playing(), "Song never paused");
        assert_eq!(player.pending_start(), Some(paused_at));
        assert_eq!(
            player.elapsed().await?,
            Some(paused_at),
            "paused position should remain visible to OSC and browser status"
        );

        player.play().await?;
        eventually(|| device.is_playing(), "Song never resumed");
        let resumed_at = elapsed_eventually(&player).await?;
        assert!(resumed_at >= paused_at);

        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_after_pause_resets_preserved_position() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        tokio::time::sleep(Duration::from_millis(50)).await;
        player.pause().await?;
        eventually(|| !device.is_playing(), "Song never paused");
        assert!(player.elapsed().await?.is_some());

        player.stop().await;
        assert_eq!(player.pending_start(), None);
        assert_eq!(player.elapsed().await?, None);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_from_nonzero_start() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        let result = player.play_from(Duration::from_millis(100)).await?;
        assert!(result.is_some(), "play_from should succeed");

        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    /// #634: from the moment a play from part way in returns, the position
    /// is the start position, not none (0:00) until the subsystems are
    /// ready and the clock starts.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_play_from_reads_its_start_position_before_the_clock_starts(
    ) -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let start = Duration::from_millis(300);
        assert!(player.play_from(start).await?.is_some());
        let elapsed = player.elapsed().await?;
        assert!(
            elapsed.is_some_and(|e| e >= start && e < start + Duration::from_secs(2)),
            "{elapsed:?}"
        );
        player.stop().await;
        assert_eq!(player.elapsed().await?, None);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_seek_while_playing_restarts_at_position() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        player.seek_to(Duration::from_millis(200)).await?;
        eventually(|| device.is_playing(), "Song never resumed after seek");

        let elapsed = elapsed_eventually(&player).await?;
        assert!(
            elapsed >= Duration::from_millis(200),
            "elapsed after seek should be at least the seek target, got {:?}",
            elapsed
        );
        assert!(player.pending_start().is_none());

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_seek_while_stopped_sets_pending_start() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.seek_to(Duration::from_millis(150)).await?;
        assert_eq!(player.pending_start(), Some(Duration::from_millis(150)));
        assert!(!device.is_playing(), "seek while stopped must not play");

        // play() consumes the pending position.
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        assert!(player.pending_start().is_none());
        let elapsed = elapsed_eventually(&player).await?;
        assert!(
            elapsed >= Duration::from_millis(150),
            "elapsed should start at the pending position, got {:?}",
            elapsed
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_seek_pending_cleared_on_navigation() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        player.seek_to(Duration::from_millis(150)).await?;
        assert!(player.pending_start().is_some());
        player.next().await;
        assert!(
            player.pending_start().is_none(),
            "navigation must clear the pending seek position"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_seek_beyond_duration_errors() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        let result = player.seek_to(Duration::from_secs(3600)).await;
        assert!(result.is_err(), "seeking past the song end should error");
        assert!(player.pending_start().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seek_and_play_from_at_or_past_the_end_are_turned_down() -> Result<(), Box<dyn Error>> {
        // #584: audio has nothing to play at the end, so a start there
        // failed and the playlist skipped the song.
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;
        let song = player.get_playlist().current().expect("a song");
        let end = song.length();

        assert!(player.seek_to(end).await.is_err(), "seek to the very end");
        assert!(player.pending_start().is_none());
        for start in [end, end + Duration::from_secs(20)] {
            assert!(
                player.play_from(start).await.is_err(),
                "play_from {start:?} on a {end:?} song"
            );
        }
        assert!(!device.is_playing(), "a start past the end played");
        assert_eq!(
            player
                .get_playlist()
                .current()
                .map(|s| s.name().to_string()),
            Some(song.name().to_string()),
            "the playlist moved on"
        );

        // Nor does picking another song with a start past its end move the
        // playlist to it.
        let other = player.get_playlist().songs()[1].clone();
        assert!(player
            .play_song_from(&other, Duration::from_secs(3600))
            .await
            .is_err());
        assert_eq!(
            player
                .get_playlist()
                .current()
                .map(|s| s.name().to_string()),
            Some(song.name().to_string()),
            "play_song_from moved the playlist"
        );

        // Just before the end is still a start.
        player.seek_to(end - Duration::from_millis(1)).await?;
        assert_eq!(player.pending_start(), Some(end - Duration::from_millis(1)));
        Ok(())
    }

    /// A kept position the song can no longer play from -- a reload has
    /// shortened it -- is dropped by the Play it fails, so the next Play
    /// starts the song rather than failing on it for good.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_kept_position_past_the_songs_end_is_dropped_by_play() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let end = player.get_playlist().current().expect("a song").length();
        player.pause_at_for_test(end + Duration::from_secs(5));
        assert!(player.play().await.is_err(), "a start past the end");
        assert_eq!(player.pending_start(), None, "the position was kept");
        assert!(player.play().await?.is_some(), "Play failed for good");
        player.stop().await;
        Ok(())
    }

    /// A start turned down while the hardware initializes keeps the kept
    /// position, for the Play after it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_start_while_hardware_initializes_keeps_the_position() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let paused_at = Duration::from_millis(100);
        player.pause_at_for_test(paused_at);
        player.init_done_tx.send_modify(|v| *v = false);
        assert!(player.play().await.is_err());
        assert_eq!(player.pending_start(), Some(paused_at));
        player.init_done_tx.send_modify(|v| *v = true);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_seek_to_section_unknown_errors() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        let result = player.seek_to_section("no-such-section").await;
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_seek_spam_stays_consistent() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Rapid repeated seeks must not wedge the player.
        for ms in [50u64, 120, 80, 200, 30] {
            player.seek_to(Duration::from_millis(ms)).await?;
        }
        eventually(|| device.is_playing(), "Song not playing after seek spam");
        assert!(player.is_playing().await);

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_after_stop_restarts() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // First play/stop cycle.
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        // Second play should succeed (stop_run flag was reset).
        let result = player.play().await?;
        assert!(
            result.is_some(),
            "play() after stop should start a new song"
        );
        eventually(|| device.is_playing(), "Song never restarted");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_audio_only_no_midi() -> Result<(), Box<dyn Error>> {
        let player = assets_player().ready().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert!(
            player.midi_device().is_none(),
            "MIDI device should be absent"
        );

        // Song 2 has no midi_file, so barrier is audio-only.
        // Navigate to Song 2 (default playlist starts at Song 1).
        player.get_playlist().next(); // Song 3
        player.get_playlist().next(); // Song 5
                                      // Use the all songs playlist to reach Song 2 more easily.
        player.switch_to_playlist("all_songs").await.unwrap();
        // all_songs starts at Song 1, navigate to Song 2.
        player.get_playlist().next(); // Song 10
        player.get_playlist().next(); // Song 2
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 2");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_midi_only_no_audio() -> Result<(), Box<dyn Error>> {
        let player = assets_player()
            .audio(None)
            .midi("mock-midi-device")
            .ready()
            .await?;

        assert!(
            player.audio_device().is_none(),
            "Audio device should be absent"
        );

        // Song 1 has midi_file. Barrier = 1 (MIDI only).
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.play().await?;

        // Natural finish: playlist should advance.
        eventually_async(
            || async { player.get_playlist().current().unwrap().name() != "Song 1" },
            "Playlist never advanced after MIDI-only playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_subsystems_completes_immediately() -> Result<(), Box<dyn Error>> {
        // Build a Player directly with no subsystems to exercise the
        // num_barriers == 0 early-return in play_files().
        let player = assets_player_with_devices(no_devices())?;

        assert!(player.audio_device().is_none());
        assert!(player.midi_device().is_none());
        assert!(player.dmx_engine().is_none());

        player.play().await?;

        // num_barriers == 0 → play_files returns immediately → playlist advances.
        eventually_async(
            || async { player.get_playlist().current().unwrap().name() != "Song 1" },
            "Playlist never advanced after no-subsystem playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_natural_finish_clears_play_state() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Song 1 is short (~0.7s). Let it finish naturally.
        player.play().await?;

        // Wait for natural finish: playlist advances.
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after natural finish",
        )
        .await;

        let elapsed = player.elapsed().await?;
        assert!(
            elapsed.is_none(),
            "elapsed() should be None after natural finish"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_with_dmx_engine() -> Result<(), Box<dyn Error>> {
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            None,
        );
        let player = assets_player().dmx(Some(dmx_config)).ready().await?;

        assert!(
            player.dmx_engine().is_some(),
            "DMX engine should be present"
        );

        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    /// A player whose one song, "Lit", has a red show of its own and whose
    /// ambient light is blue, with the directory holding them.
    async fn ambient_player() -> Result<(tempfile::TempDir, Arc<Player>), Box<dyn Error>> {
        // A song with a show of its own, long enough to pause and stop.
        let dir = tempfile::tempdir()?;
        let song_dir = dir.path().join("songs").join("lit");
        fs::create_dir_all(&song_dir)?;
        fs::copy("assets/20-sec-long.wav", song_dir.join("track.wav"))?;
        let wash = |color: &str| {
            format!(
                "show \"s\" {{\n    @0.000\n    wash: static, color: \"{color}\", duration: 30s\n}}\n"
            )
        };
        fs::write(song_dir.join("show.light"), wash("red"))?;
        fs::write(
            song_dir.join("song.yaml"),
            "name: Lit\ntracks:\n- name: track\n  file: track.wav\n  file_channel: 1\nlighting:\n- file: show.light\n",
        )?;
        let ambient_show = dir.path().join("ambient.light");
        fs::write(&ambient_show, wash("blue"))?;

        let groups = HashMap::from([(
            "wash".to_string(),
            config::lighting::LogicalGroup::new("wash".to_string(), vec![]),
        )]);
        let lighting = config::Lighting::new(None, Some(groups), None).with_ambient(Some(
            config::ambient::Ambient::new(
                Some(ambient_show.display().to_string()),
                None,
                None,
                None,
            ),
        ));
        let dmx = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            Some(lighting),
        );
        let songs = songs::get_all_songs(&dir.path().join("songs"))?;
        assert!(songs.failures().is_empty(), "{:?}", songs.failures());
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::new(&["Lit".into()]),
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                Some(dmx),
                HashMap::new(),
                "songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok((dir, player))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ambient_light_plays_while_no_song_does() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = ambient_player().await?;
        let engine = player.dmx_engine().expect("DMX engine should be present");
        let state = || engine.ambient_status().map(|s| s.state);

        // Nothing has played yet: ambient at once.
        eventually(|| state() == Some("playing"), "ambient at startup");

        // A song with lights: ambient fades out under its show.
        let device = player.audio_device().expect("audio device");
        let device = device.to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        eventually(|| state().is_none(), "ambient leaves for the song");

        // A pause holds the song's lights.
        player.pause().await?;
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(state(), None, "a pause holds");

        // A stop is between songs.
        player.stop().await;
        eventually(|| state() == Some("playing"), "ambient after a stop");
        Ok(())
    }

    /// The status reads the DMX engine, its ambient light and the song's
    /// lighting, as every surface shows them, and agrees with the transport
    /// snapshot.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_status_reads_the_lights_and_agrees_with_the_transport(
    ) -> Result<(), Box<dyn Error>> {
        let (_dir, player) = ambient_player().await?;
        eventually(
            || {
                player
                    .dmx_engine()
                    .and_then(|engine| engine.ambient_status())
                    .is_some_and(|s| s.state == "playing")
            },
            "ambient at startup",
        );
        let status = player.status_snapshot().await;
        match &status.ambient {
            Some(DmxRead::Read(ambient)) => {
                assert_eq!(ambient.report.state, "playing");
                assert!(ambient.source.is_some(), "a show plays someone's settings");
            }
            other => panic!("the ambient light was not read: {other:?}"),
        }
        assert!(matches!(status.lighting, Some(DmxRead::Read(_))));
        assert!(!status.playing);
        assert_eq!(
            status.transport(),
            player.compute_transport_snapshot().await
        );
        Ok(())
    }

    /// A player on a playlist of half-second songs "One" and "Two", each
    /// with a red show of its own, laid out by `songs` (the playlist's
    /// `songs:` list, with `{green}` for the path of a green ambient show).
    /// The profile's ambient light is blue.
    async fn entry_ambient_player(
        songs: &str,
    ) -> Result<(tempfile::TempDir, Arc<Player>), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let wash = |color: &str| {
            format!(
                "show \"s\" {{\n    @0.000\n    wash: static, color: \"{color}\", duration: 30s\n}}\n"
            )
        };
        for name in ["One", "Two"] {
            let song_dir = dir.path().join("songs").join(name);
            fs::create_dir_all(&song_dir)?;
            fs::copy("assets/1Channel44.1k.wav", song_dir.join("track.wav"))?;
            fs::write(song_dir.join("show.light"), wash("red"))?;
            fs::write(
                song_dir.join("song.yaml"),
                format!(
                    "name: {name}\ntracks:\n- name: track\n  file: track.wav\n  file_channel: 1\nlighting:\n- file: show.light\n"
                ),
            )?;
        }
        let blue = dir.path().join("blue.light");
        fs::write(&blue, wash("blue"))?;
        let green = dir.path().join("green.light");
        fs::write(&green, wash("green"))?;
        let playlist_file = dir.path().join("playlist.yaml");
        fs::write(
            &playlist_file,
            format!(
                "songs:\n{}",
                songs.replace("{green}", &green.display().to_string())
            ),
        )?;

        let groups = HashMap::from([(
            "wash".to_string(),
            config::lighting::LogicalGroup::new("wash".to_string(), vec![]),
        )]);
        let lighting = config::Lighting::new(None, Some(groups), None).with_ambient(Some(
            config::ambient::Ambient::new(Some(blue.display().to_string()), None, None, None),
        ));
        let dmx = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            Some(lighting),
        );
        let songs = songs::get_all_songs(&dir.path().join("songs"))?;
        assert!(songs.failures().is_empty(), "{:?}", songs.failures());
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(&playlist_file)?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                Some(dmx),
                HashMap::new(),
                "songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok((dir, player))
    }

    /// The ambient show up, by its file's name, and whose it is.
    fn ambient_up(player: &Player) -> (Option<String>, Option<&'static str>) {
        let engine = player.dmx_engine().expect("DMX engine should be present");
        let report = engine.ambient_report();
        let show = report
            .show
            .filter(|_| report.state == "playing")
            .and_then(|show| show.file_name().map(|f| f.to_string_lossy().into_owned()));
        let source = show
            .as_ref()
            .and(player.ambient_source())
            .map(|s| s.as_str());
        (show, source)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_entrys_ambient_light_plays_in_the_gap_before_it() -> Result<(), Box<dyn Error>> {
        let (_dir, player) =
            entry_ambient_player("- One\n- song: Two\n  ambient:\n    show: {green}\n").await?;
        let up = |show: &str, source| (Some(show.to_string()), Some(source));
        eventually(
            || ambient_up(&player) == up("blue.light", "profile"),
            "the profile's before One",
        );

        // Next while stopped: the gap is before Two now.
        player.next().await;
        eventually(
            || ambient_up(&player) == up("green.light", "entry"),
            "Two's own before it",
        );
        player.prev().await;
        eventually(
            || ambient_up(&player) == up("blue.light", "profile"),
            "back to the profile's before One",
        );

        // One ends by itself: the player waits on Two, with its light.
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "One never started");
        eventually(|| player.get_playlist().position() == 1, "One never ended");
        eventually(
            || ambient_up(&player) == up("green.light", "entry"),
            "Two's own after One ends",
        );

        // Two, the last, ends by itself: the gap is after it, with the
        // playlist's light, not Two's.
        player.play().await?;
        eventually(
            || player.song_playing.load(Ordering::Relaxed),
            "Two never started",
        );
        eventually(
            || !player.song_playing.load(Ordering::Relaxed),
            "Two never ended",
        );
        eventually(
            || ambient_up(&player) == up("blue.light", "profile"),
            "the profile's after the last song",
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_entry_can_turn_ambient_light_off_before_it() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = entry_ambient_player("- One\n- song: Two\n  ambient: false\n").await?;
        eventually(|| ambient_up(&player).0.is_some(), "ambient before One");
        player.next().await;
        eventually(|| ambient_up(&player) == (None, None), "none before Two");
        player.prev().await;
        eventually(
            || ambient_up(&player).0.is_some(),
            "ambient before One again",
        );
        Ok(())
    }

    /// Rewrites the playlist of a player from [`entry_ambient_player`] as
    /// `yaml` (`{green}` for the green show) and reloads it, as a save does.
    fn edit_ambient_playlist(player: &Player, dir: &tempfile::TempDir, yaml: &str) {
        let green = dir.path().join("green.light").display().to_string();
        let path = dir.path().join("playlist.yaml");
        fs::write(&path, yaml.replace("{green}", &green)).unwrap();
        player
            .reload_songs_from(&SongSources {
                songs: dir.path().join("songs"),
                playlists_dir: None,
                legacy_playlist: Some(path),
            })
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_last_song_taken_off_the_list_ending_does_not_auto_play_the_entry_before(
    ) -> Result<(), Box<dyn Error>> {
        let (dir, player) = entry_ambient_player("- song: One\n  auto_play: 2s\n- Two\n").await?;
        player.next().await;
        assert!(player.auto_play_status().is_none(), "Two waits for play");
        player.play().await?;
        // Two, the last, is taken off the list while it plays.
        edit_ambient_playlist(&player, &dir, "songs:\n- song: One\n  auto_play: 2s\n");
        eventually(
            || !player.song_playing.load(Ordering::Relaxed),
            "Two never ended",
        );
        std::thread::sleep(Duration::from_millis(200));
        // One, already played before Two, must not start again by itself.
        assert_eq!(player.auto_play_status().map(|(s, _)| s), None);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_removing_the_waiting_song_does_not_auto_play_another(
    ) -> Result<(), Box<dyn Error>> {
        let (dir, player) = entry_ambient_player("- One\n- song: Two\n  auto_play: 1s\n").await?;
        player.next().await;
        let (song, _) = player.auto_play_status().expect("Two waits out its gap");
        assert_eq!(song, "Two");
        // Two is taken off the list during the gap.
        edit_ambient_playlist(&player, &dir, "songs:\n- One\n");
        assert_eq!(current_name(&player).await, "One");
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !player.song_playing.load(Ordering::Relaxed),
            "One, never waited on, started by itself in Two's place"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_turning_auto_play_off_calls_the_wait_off() -> Result<(), Box<dyn Error>> {
        let (dir, player) = entry_ambient_player("- One\n- song: Two\n  auto_play: 1s\n").await?;
        player.next().await;
        assert!(player.auto_play_status().is_some());
        edit_ambient_playlist(&player, &dir, "songs:\n- One\n- Two\n");
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !player.song_playing.load(Ordering::Relaxed),
            "Two started by itself with auto_play taken off it"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_to_the_playlists_ambient_while_stopped_takes_effect(
    ) -> Result<(), Box<dyn Error>> {
        let (dir, player) = entry_ambient_player("- One\n- Two\n").await?;
        let up = |show: &str, source| (Some(show.to_string()), Some(source));
        eventually(
            || ambient_up(&player) == up("blue.light", "profile"),
            "the profile's before One",
        );
        edit_ambient_playlist(
            &player,
            &dir,
            "ambient:\n  show: {green}\nsongs:\n- One\n- Two\n",
        );
        eventually(
            || ambient_up(&player) == up("green.light", "playlist"),
            "the playlist's new ambient light after the save",
        );
        Ok(())
    }

    /// A song that ends by itself takes its section boundary poller with it.
    /// The poller waited on a cancel, which a song ending by itself never
    /// makes, and outlived it: one more task for each song played.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_song_ending_by_itself_leaves_no_task_behind() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = entry_ambient_player("- One\n").await?;
        let alive = || {
            tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks()
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        let before = alive();
        for _ in 0..3 {
            player.play().await?;
            eventually_async(|| async { !player.is_playing().await }, "One never ended").await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let after = alive();
        assert!(after < before + 3, "{before} tasks before, {after} after");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_ambient_mode_holds_it_over_a_song_or_keeps_it_off() -> Result<(), Box<dyn Error>> {
        use crate::dmx::engine::AmbientMode;
        let (_dir, player) = ambient_player().await?;
        let engine = player.dmx_engine().expect("DMX engine should be present");
        let report = || {
            let report = engine.ambient_report();
            (report.mode, report.state)
        };
        assert_eq!(player.ambient_mode(), AmbientMode::Auto, "starts in auto");
        eventually(
            || report() == (AmbientMode::Auto, "playing"),
            "ambient at startup",
        );

        // On: ambient holds over the song.
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.set_ambient_mode(AmbientMode::On);
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(report(), (AmbientMode::On, "playing"), "on wins");

        // Auto, mid-song: back to the song's lights.
        player.set_ambient_mode(AmbientMode::Auto);
        assert_eq!(report(), (AmbientMode::Auto, "off"));

        // Off: a stop leaves the stage dark.
        player.set_ambient_mode(AmbientMode::Off);
        player.stop().await;
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(report(), (AmbientMode::Off, "off"));
        assert_eq!(player.ambient_mode(), AmbientMode::Off);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_playlist_while_playing_stays() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert_eq!(player.get_playlist().name(), "playlist");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Attempt switch while playing — should return error.
        let result = player.switch_to_playlist("all_songs").await;
        assert!(
            result.is_err(),
            "switch_to_playlist should fail while playing"
        );
        assert_eq!(
            player.get_playlist().name(),
            "playlist",
            "playlist should not change while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_playlist_clamps_at_end_on_natural_finish() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Navigate to Song 9 (last in playlist).
        // Playlist: Song 1, Song 3, Song 5, Song 7, Song 9
        player.next().await; // Song 3
        player.next().await; // Song 5
        player.next().await; // Song 7
        player.next().await; // Song 9
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 9");

        // Play Song 9 — short audio (0.5s), should finish naturally.
        player.play().await?;

        // After natural finish, playlist next() clamps at the last song.
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after Song 9 finished",
        )
        .await;

        // Playlist should still be at Song 9 (clamped, no wrap).
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 9");

        Ok(())
    }

    /// A test player on the assets' songs, with the playlist `yaml`.
    async fn make_test_player_with_playlist(yaml: &str) -> Result<Arc<Player>, Box<dyn Error>> {
        assets_player().playlist_yaml(yaml).ready().await
    }

    async fn current_name(player: &Player) -> String {
        player.get_playlist().current().unwrap().name().to_string()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_past_the_end_of_a_song_off_the_playlist_stays_put(
    ) -> Result<(), Box<dyn Error>> {
        // #584: a song only in all_songs must not switch the player to
        // all_songs when its start is turned down.
        let player = make_test_player_with_playlist("songs:\n- Song 1\n").await?;
        assert!(player
            .play_song_from("Song 3", Duration::from_secs(3600))
            .await
            .is_err());
        assert_eq!(*player.active_playlist.read(), "playlist");
        assert_eq!(current_name(&player).await, "Song 1");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn auto_play_starts_a_song_the_one_before_leads_into() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_playlist(
            "songs:\n- Song 1\n- song: Song 3\n  auto_play: true\n- Song 5\n",
        )
        .await?;
        // Song 1 ends by itself; Song 3 starts by itself and plays to its end;
        // Song 5 waits for play.
        player.play().await?;
        eventually_async(
            || async { current_name(&player).await == "Song 5" },
            "Song 3 never played through",
        )
        .await;
        eventually_async(
            || async { !player.is_playing().await },
            "the player never stopped",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!player.is_playing().await, "Song 5 waits for play");
        assert_eq!(current_name(&player).await, "Song 5");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn auto_play_starts_a_song_navigated_onto() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_playlist(
            "songs:\n- Song 1\n- song: Song 3\n  auto_play: true\n- Song 5\n",
        )
        .await?;
        player.next().await;
        eventually_async(
            || async { player.is_playing().await },
            "Song 3 never started",
        )
        .await;
        assert_eq!(current_name(&player).await, "Song 3");
        player.stop().await;

        // Next at the end of the list does not move, and does not replay it.
        let player =
            make_test_player_with_playlist("songs:\n- Song 1\n- song: Song 3\n  auto_play: true\n")
                .await?;
        player.next().await;
        eventually_async(
            || async { player.is_playing().await },
            "Song 3 never started",
        )
        .await;
        player.stop().await;
        player.next().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!player.is_playing().await, "a clamped next starts nothing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_or_navigation_in_the_gap_calls_auto_play_off() -> Result<(), Box<dyn Error>> {
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 500ms\n- Song 5\n";
        let player = make_test_player_with_playlist(yaml).await?;

        // The gap runs, then the song starts; status shows it waiting.
        player.next().await;
        let (song, starts_in) = player.auto_play_status().expect("waiting");
        assert_eq!(song, "Song 3");
        assert!(starts_in <= Duration::from_millis(500), "{starts_in:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!player.is_playing().await, "waits out the gap");
        eventually_async(
            || async { player.is_playing().await },
            "Song 3 never started",
        )
        .await;
        assert!(player.auto_play_status().is_none(), "started, not waiting");
        player.stop().await;

        // A stop inside the gap calls it off.
        player.prev().await;
        player.next().await;
        player.stop().await;
        assert!(player.auto_play_status().is_none(), "nothing waiting");
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(!player.is_playing().await, "a stop calls it off");

        // So does moving on.
        player.prev().await;
        player.next().await;
        player.next().await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(!player.is_playing().await, "moving on calls it off");
        assert_eq!(current_name(&player).await, "Song 5");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_once_an_auto_plays_gap_is_up_keeps_its_song_from_starting(
    ) -> Result<(), Box<dyn Error>> {
        // The gap ran out and the auto play, done waiting, went to start its
        // song; a stop that took the join lock first found nothing playing
        // and nothing waiting, and the song started after it regardless.
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 100ms\n- Song 5\n";
        let player = make_test_player_with_playlist(yaml).await?;
        player.next().await;
        {
            // Held over the end of the gap, as by a stop on a busy machine.
            let mut join = player.join.lock().await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            player.cut_locked(&mut join).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!player.is_playing().await, "Song 3 started after the stop");
        assert_eq!(current_name(&player).await, "Song 3");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancelling_a_waiting_auto_play_stops_on_its_song() -> Result<(), Box<dyn Error>> {
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 500ms\n- Song 5\n";
        let player = make_test_player_with_playlist(yaml).await?;
        assert!(!player.cancel_waiting_auto_play(), "nothing waiting");

        player.next().await;
        assert!(player.auto_play_status().is_some(), "waiting");
        assert!(player.cancel_waiting_auto_play());
        assert!(player.auto_play_status().is_none());
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(!player.is_playing().await, "called off");
        assert_eq!(current_name(&player).await, "Song 3");
        Ok(())
    }

    /// The songs the player announced as current, in order.
    struct Announced(parking_lot::Mutex<Vec<String>>);

    impl SongChangeNotifier for Announced {
        fn notify(&self, song: &Song) {
            self.0.lock().push(song.name().to_string());
        }
    }

    fn announce(player: &Player) -> Arc<Announced> {
        let announced = Arc::new(Announced(Default::default()));
        player.add_song_change_notifier(announced.clone());
        announced
    }

    /// The code a refused `navigate_to_song` is.
    fn entry_error(result: Result<Arc<Song>, playlist::EntryError>) -> playlist::EntryError {
        result.err().expect("the move should have been refused")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_moves_to_the_entry_and_announces_it_once(
    ) -> Result<(), Box<dyn Error>> {
        let player =
            make_test_player_with_playlist("songs:\n- Song 1\n- Song 3\n- Song 5\n").await?;
        let announced = announce(&player);
        let song = player.navigate_to_song(2, Some("Song 5")).await?;
        assert_eq!(song.name(), "Song 5");
        assert_eq!(current_name(&player).await, "Song 5");
        assert!(!player.is_playing().await);
        assert_eq!(*announced.0.lock(), ["Song 5"]);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_calls_a_pause_off_as_next_does() -> Result<(), Box<dyn Error>> {
        let player =
            make_test_player_with_playlist("songs:\n- Song 1\n- Song 3\n- Song 5\n").await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.pause().await?;
        eventually(|| !device.is_playing(), "Song never paused");
        assert!(player.pending_start().is_some(), "paused");

        player.navigate_to_song(1, None).await?;
        assert_eq!(current_name(&player).await, "Song 3");
        assert!(player.pending_start().is_none(), "the pause is called off");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_schedules_and_cancels_auto_play() -> Result<(), Box<dyn Error>> {
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 500ms\n- Song 5\n";
        let player = make_test_player_with_playlist(yaml).await?;
        player.navigate_to_song(1, None).await?;
        let (song, _) = player.auto_play_status().expect("scheduled");
        assert_eq!(song, "Song 3");
        // Leaving the entry calls the wait off.
        player.navigate_to_song(2, None).await?;
        assert!(player.auto_play_status().is_none());
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(!player.is_playing().await, "called off");
        assert_eq!(current_name(&player).await, "Song 5");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_reaches_the_second_entry_of_a_repeated_song(
    ) -> Result<(), Box<dyn Error>> {
        let player =
            make_test_player_with_playlist("songs:\n- Song 1\n- Song 3\n- Song 1\n").await?;
        let song = player.navigate_to_song(2, Some("Song 1")).await?;
        assert_eq!(song.name(), "Song 1");
        assert_eq!(player.get_playlist().entry(), Some(2));
        let status = crate::controller::ble::status::from_player(
            &player.transport_status().await,
            player.gains_revision(),
            None,
            player.is_locked(),
        )?;
        assert_eq!(status.position, Some(2));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_refuses_a_stale_name_and_a_bad_position() -> Result<(), Box<dyn Error>>
    {
        let player =
            make_test_player_with_playlist("songs:\n- Song 1\n- Song 3\n- Song 5\n").await?;
        let announced = announce(&player);
        let changed = entry_error(player.navigate_to_song(1, Some("Song 5")).await);
        assert!(matches!(changed, playlist::EntryError::Changed { .. }));
        let out = entry_error(player.navigate_to_song(3, None).await);
        assert_eq!(
            out,
            playlist::EntryError::OutOfRange {
                position: 3,
                len: 3
            }
        );
        assert_eq!(player.get_playlist().entry(), Some(0));
        assert!(announced.0.lock().is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigating_to_the_entry_it_is_on_changes_nothing() -> Result<(), Box<dyn Error>> {
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 5s\n- Song 5\n";
        let player = make_test_player_with_playlist(yaml).await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        let announced = announce(&player);

        // A waiting auto-play stays waiting.
        player.navigate_to_song(1, None).await?;
        assert_eq!(announced.0.lock().len(), 1);
        player.navigate_to_song(1, Some("Song 3")).await?;
        assert!(player.auto_play_status().is_some(), "still waiting");
        assert_eq!(announced.0.lock().len(), 1, "no second event");
        player.navigate_to_song(0, None).await?;

        // A paused song stays paused.
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.pause().await?;
        eventually(|| !device.is_playing(), "Song never paused");
        let events = announced.0.lock().len();
        let song = player.navigate_to_song(0, Some("Song 1")).await?;
        assert_eq!(song.name(), "Song 1");
        assert!(player.pending_start().is_some(), "still paused");
        assert_eq!(announced.0.lock().len(), events);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_while_playing_moves_nothing() -> Result<(), Box<dyn Error>> {
        let player =
            make_test_player_with_playlist("songs:\n- Song 1\n- Song 3\n- Song 5\n").await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        let announced = announce(&player);
        let song = player.navigate_to_song(2, Some("Song 5")).await?;
        assert_eq!(song.name(), "Song 1");
        assert_eq!(player.get_playlist().entry(), Some(0));
        assert!(announced.0.lock().is_empty());
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_to_song_works_in_all_songs() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        player.switch_to_playlist("all_songs").await?;
        let names = player.get_playlist().songs().clone();
        let last = names.len() - 1;
        let song = player.navigate_to_song(last, Some(&names[last])).await?;
        assert_eq!(song.name(), names[last]);
        assert_eq!(player.get_playlist().entry(), Some(last));
        Ok(())
    }

    #[test]
    fn status_events_show_a_waiting_auto_play_when_they_have_its_events() {
        let parse = |yaml: &str| -> StatusEvents {
            let config: config::StatusEvents = ::config::Config::builder()
                .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
            StatusEvents::new(Some(config)).unwrap().unwrap()
        };
        let cc = |value: u8| {
            format!("[{{ type: control_change, channel: 16, controller: 2, value: {value} }}]")
        };
        let base = format!(
            "off_events: {}\nidling_events: {}\nplaying_events: {}\n",
            cc(0),
            cc(1),
            cc(2)
        );
        let value = |events: &[LiveEvent<'static>]| match events[0] {
            LiveEvent::Midi {
                message: midly::MidiMessage::Controller { value, .. },
                ..
            } => u8::from(value),
            _ => panic!("a control change"),
        };

        let events = parse(&format!("{base}auto_play_events: {}\n", cc(3)));
        assert_eq!(value(events.current(false, false)), 1, "idling");
        assert_eq!(value(events.current(false, true)), 3, "auto play waiting");
        assert_eq!(value(events.current(true, false)), 2, "playing");

        // Without events of its own, a waiting auto play idles.
        let events = parse(&base);
        assert_eq!(value(events.current(false, true)), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_reporting_ends_with_its_init_round() -> Result<(), Box<dyn Error>> {
        let player = assets_player().midi("mock-midi").ready().await?;
        let cc = "[{ type: control_change, channel: 16, controller: 2, value: 0 }]";
        let config: config::StatusEvents = ::config::Config::builder()
            .add_source(::config::File::from_str(
                &format!("off_events: {cc}\nidling_events: {cc}\nplaying_events: {cc}\n"),
                ::config::FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        let events = StatusEvents::new(Some(config))?.expect("status events");
        let round = tokio_util::sync::CancellationToken::new();
        let reporting = player
            .report_status(events, round.clone())
            .expect("a MIDI device to report on");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !reporting.is_finished(),
            "reports while its round is current"
        );
        // A hardware reload cancels the round: its reporting stops, rather
        // than going on beside the next round's.
        round.cancel();
        tokio::time::timeout(crate::testutil::PATIENCE, reporting)
            .await
            .expect("the status loop outlived its init round")?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_reporting_cancelled_while_waiting_emits_nothing() -> Result<(), Box<dyn Error>>
    {
        let player = assets_player().midi("mock-midi").ready().await?;
        let midi = player.midi_device().expect("MIDI device").to_mock()?;
        let cc = "[{ type: control_change, channel: 16, controller: 2, value: 0 }]";
        let config: config::StatusEvents = ::config::Config::builder()
            .add_source(::config::File::from_str(
                &format!("off_events: {cc}\nidling_events: {cc}\nplaying_events: {cc}\n"),
                ::config::FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        let events = StatusEvents::new(Some(config))?.expect("status events");
        let round = tokio_util::sync::CancellationToken::new();
        // The loop waits for the join lock, as behind a song starting, while
        // a reload cancels its round.
        let join = player.join.lock().await;
        midi.reset_emitted_event();
        let reporting = player
            .report_status(events, round.clone())
            .expect("a MIDI device to report on");
        tokio::time::sleep(Duration::from_millis(100)).await;
        round.cancel();
        drop(join);
        tokio::time::timeout(crate::testutil::PATIENCE, reporting)
            .await
            .expect("the status loop outlived its init round")?;
        assert_eq!(midi.get_emitted_event(), None, "emitted for a round gone");
        Ok(())
    }

    // --- resolve_playback_outcome tests ---

    #[test]
    fn playback_outcome_no_audio() {
        assert_eq!(resolve_playback_outcome(false, None), Ok(()));
    }

    #[test]
    fn playback_outcome_audio_ok() {
        assert_eq!(resolve_playback_outcome(true, Some(Ok(()))), Ok(()));
    }

    #[test]
    fn playback_outcome_audio_err() {
        let err_msg = "device disconnected".to_string();
        assert_eq!(
            resolve_playback_outcome(true, Some(Err(err_msg.clone()))),
            Err(err_msg)
        );
    }

    #[test]
    fn playback_outcome_audio_none_panicked() {
        // Thread panicked before setting outcome — treated as success
        assert_eq!(resolve_playback_outcome(true, None), Ok(()));
    }

    // --- decide_cleanup_action tests ---

    #[test]
    fn cleanup_success_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, false, false, false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_success_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, true, false, false),
            CleanupAction::StopCancelled
        );
    }

    #[test]
    fn cleanup_failed_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), false, false, false),
            CleanupAction::AdvancePlaylist
        );
    }

    /// Playback that cancelled *itself* still needs the full cleanup.
    ///
    /// When the audio callback stops mid-song, the audio thread cancels the
    /// shared handle so MIDI and DMX don't sit waiting on a frozen clock, then
    /// reports the failure. `StopCancelled` would be wrong: it skips clearing
    /// `join` and `play_start_time` because it assumes `stop()` already did.
    /// Nothing called `stop()` here, so taking it left the player believing a
    /// song was still playing, with no way back short of a restart.
    #[test]
    fn cleanup_self_cancelled_advances() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), true, false, true),
            CleanupAction::AdvancePlaylist
        );
    }

    /// The mirror of the case above, and the reason it is keyed on *who*
    /// cancelled rather than on whether an error came with it.
    ///
    /// A seek cancels the in-flight playback and immediately starts a new one.
    /// That cancelled playback can report an error on its way out, so "failed
    /// and cancelled" is not enough to identify a self-cancellation — reading it
    /// that way let this cleanup run and clear `join` and `play_start_time`
    /// belonging to the playback the seek had already started, leaving
    /// `is_playing()` false in the middle of a seek. Caught by
    /// `test_seek_spam_stays_consistent` under CI load, not locally.
    #[test]
    fn cleanup_deliberate_cancel_with_an_error_still_stops() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), true, false, false),
            CleanupAction::StopCancelled
        );
    }

    /// A loop break outranks everything: it cancels deliberately in order to
    /// advance and keep playing.
    #[test]
    fn cleanup_loop_break_wins_over_failure() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), true, true, true),
            CleanupAction::LoopBreakAndPlay
        );
    }

    #[test]
    fn cleanup_sender_dropped_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::SenderDropped, false, false, false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_loop_broken() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, false, true, false),
            CleanupAction::LoopBreakAndPlay
        );
    }

    #[test]
    fn cleanup_loop_broken_takes_priority_over_cancel() {
        // If both cancelled and loop_broken, loop_broken wins — we intentionally
        // cancel to break out of the loop immediately, but the intent is to advance.
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, true, true, false),
            CleanupAction::LoopBreakAndPlay
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dmx_utility_methods() -> Result<(), Box<dyn Error>> {
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            None,
        );
        let player = assets_player().dmx(Some(dmx_config)).ready().await?;

        // get_cues() with DMX engine present (no timeline loaded → empty)
        let cues = player.get_cues();
        assert!(cues.is_empty());

        // broadcast_handles() returns Some when DMX engine is present
        assert!(
            player.broadcast_handles().is_some(),
            "broadcast_handles should be Some with DMX engine"
        );

        // set_broadcast_tx() should not panic
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        player.set_broadcast_tx(tx);

        // effect_engine() returns Some when DMX engine is present
        assert!(
            player.effect_engine().is_some(),
            "effect_engine should be Some with DMX engine"
        );

        // format_active_effects() returns Some when DMX engine is present
        assert!(
            player.format_active_effects().is_some(),
            "format_active_effects should be Some with DMX engine"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_track_mappings() -> Result<(), Box<dyn Error>> {
        // Player with audio → track_mappings is Some
        let player = make_test_player().await?;
        assert!(
            player.track_mappings().is_some(),
            "track_mappings should be Some when audio is configured"
        );

        // Player without audio → track_mappings is None
        let player = assets_player()
            .audio(None)
            .midi("mock-midi-device")
            .ready()
            .await?;
        assert!(
            player.track_mappings().is_none(),
            "track_mappings should be None without audio"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_next_while_playing_returns_current() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // next() while playing should return the current song without advancing
        let song = player.next().await.unwrap();
        assert_eq!(
            song.name(),
            "Song 1",
            "next() while playing should return current song"
        );
        assert_eq!(
            player.get_playlist().current().unwrap().name(),
            "Song 1",
            "playlist should not advance while playing"
        );

        // prev() while playing should also return the current song
        let song = player.prev().await.unwrap();
        assert_eq!(
            song.name(),
            "Song 1",
            "prev() while playing should return current song"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[test]
    fn status_events_new_none() {
        let result = StatusEvents::new(None).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_sample_trigger_no_engine() -> Result<(), Box<dyn Error>> {
        let player = assets_player_with_devices(no_devices())?;
        player.process_sample_trigger(&[0x90, 60, 127]);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_samples_no_engine() -> Result<(), Box<dyn Error>> {
        let player = assets_player_with_devices(no_devices())?;
        player.stop_samples();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_broadcast_handles_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.broadcast_handles().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_effect_engine_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.effect_engine().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_broadcast_tx_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        player.set_broadcast_tx(tx);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn emit_song_change_no_device() -> Result<(), Box<dyn Error>> {
        let player = assets_player().audio(None).ready().await?;
        let song = Song::new_for_test("test", &[]);
        // Should not panic when no MIDI device is configured.
        player.emit_song_change(&song);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_to_playlist_while_playing_stays() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.get_playlist().name(), "all_songs");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let result = player.switch_to_playlist("playlist").await;
        assert!(
            result.is_err(),
            "switch_to_playlist should fail while playing"
        );
        assert_eq!(
            player.get_playlist().name(),
            "all_songs",
            "playlist should not change while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_navigate_no_midi() -> Result<(), Box<dyn Error>> {
        let player = assets_player().audio(None).ready().await?;

        player.next().await;
        let song = player.prev().await.unwrap();
        assert_eq!(song.name(), "Song 1");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_new_with_devices_all_none() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "test",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let devices = no_devices();
        let player = Player::new_with_devices(
            devices,
            test_playlists(playlist, songs),
            "test".to_string(),
            None,
        )?;
        assert!(player.audio_device().is_none());
        assert!(player.midi_device().is_none());
        assert!(player.dmx_engine().is_none());
        assert!(player.track_mappings().is_none());
        assert!(player.broadcast_handles().is_none());
        assert!(player.effect_engine().is_none());
        assert!(player.format_active_effects().is_none());
        assert!(player.get_cues().is_empty());
        assert!(!player.is_playing().await);
        assert!(player.elapsed().await?.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_from_while_playing_returns_none() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let result = player.play_from(Duration::from_secs(1)).await?;
        assert!(
            result.is_none(),
            "play_from while playing should return None"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_subsystem_player_play_and_navigate() -> Result<(), Box<dyn Error>> {
        let player = assets_player_with_devices(no_devices())?;

        player.process_sample_trigger(&[0x90, 60, 127]);
        player.stop_samples();

        let song = player.next().await.unwrap();
        assert_eq!(song.name(), "Song 3");

        let song = player.prev().await.unwrap();
        assert_eq!(song.name(), "Song 1");

        player.play().await?;
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after no-subsystem playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_midi_device_accessor() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.midi_device().is_some());

        let player = assets_player().ready().await?;
        assert!(player.midi_device().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_config_store_getter_setter() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Initially None.
        assert!(player.config_store().is_none());

        // Set a config store.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: songs\n")?;
        let cfg = config::Player::deserialize(&path)?;
        let store = std::sync::Arc::new(config::ConfigStore::new(cfg, path));
        player.set_config_store(store.clone());

        // Now it should be Some.
        let retrieved = player.config_store();
        assert!(retrieved.is_some());

        // Should be the same Arc (read_yaml returns same checksum).
        let (_, checksum1) = store.read_yaml().await?;
        let (_, checksum2) = retrieved.unwrap().read_yaml().await?;
        assert_eq!(checksum1, checksum2);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_hardware_when_idle() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.audio_device().is_some());

        // Set up a config store with a profile that has no audio.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        let yaml = "songs: songs\nprofiles:\n  - midi:\n      device: mock-midi-device\n";
        std::fs::write(&path, yaml)?;
        let cfg = config::Player::deserialize(&path)?;
        let store = std::sync::Arc::new(config::ConfigStore::new(cfg, path));
        player.set_config_store(store);

        // Reload should swap hardware — audio device should now be None.
        player.reload_hardware().await?;
        player.await_hardware_ready().await;
        assert!(
            player.audio_device().is_none(),
            "Audio device should be None after reload with no audio profile"
        );

        Ok(())
    }

    /// Sets a config store on `player` whose profile has no audio, so a
    /// reload that has run shows as the audio device going away.
    fn set_store_without_audio(player: &Player) -> Result<tempfile::TempDir, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        let yaml = "songs: songs\nprofiles:\n  - midi:\n      device: mock-midi-device\n";
        std::fs::write(&path, yaml)?;
        let cfg = config::Player::deserialize(&path)?;
        player.set_config_store(std::sync::Arc::new(config::ConfigStore::new(cfg, path)));
        Ok(dir)
    }

    /// A reload asked for during playback was refused, and the config save
    /// that asked for it answered as saved while the rig kept the old config
    /// (#578). It waits for the song to stop instead.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_hardware_during_playback_is_deferred() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;
        let _dir = set_store_without_audio(&player)?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Deferred, not refused, and the song keeps its hardware. A second
        // request while one waits just keeps it waiting.
        player.reload_hardware().await?;
        player.reload_hardware().await?;
        assert!(player.audio_device().is_some(), "reloaded under the song");
        assert!(*player.init_done_tx.borrow(), "reset under the song");
        assert!(player.is_playing().await);

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        player.await_hardware_ready().await;
        assert!(!player.hardware_reload_pending.load(Ordering::Relaxed));
        Ok(())
    }

    /// A deferred config change names the deferred reload generation it
    /// waits for, and the status counts it started once the reload starts:
    /// a surface that missed every update in between still sees the wait is
    /// over. One that cannot start at all is counted too, refused, with why.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_counts_its_generation_started_or_refused(
    ) -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _dir = set_store_without_audio(&player)?;
        assert_eq!(player.deferred_reloads(), DeferredReloads::default());
        player.play().await?;
        let change = player.apply_config_change().await;
        assert_eq!(change.hardware, ConfigApply::Deferred);
        assert_eq!(change.awaits, Some(1));
        // A second save in the same wait waits for the same reload.
        assert_eq!(player.apply_config_change().await.awaits, Some(1));
        player.stop().await;
        eventually(
            || player.deferred_reloads().started >= 1,
            "the deferred reload never started",
        );
        assert!(player.deferred_reloads().refused.is_empty());
        player.await_hardware_ready().await;

        // Deferred again, with nothing to reload from by the time it runs.
        player.play().await?;
        assert_eq!(player.apply_config_change().await.awaits, Some(2));
        *player.config_store.lock() = None;
        player.stop().await;
        eventually(
            || player.deferred_reloads().started >= 2,
            "the refused reload was not counted",
        );
        let (generation, why) = player
            .deferred_reloads()
            .refused
            .back()
            .cloned()
            .expect("refused");
        assert_eq!(generation, 2);
        assert!(why.contains("config store"), "{why}");
        assert!(!player.hardware_reload_pending());
        // The generations are this process's, named by an id that holds.
        assert_eq!(process_id(), process_id());
        Ok(())
    }

    /// A deferred reload is no longer pending once it starts applying, even
    /// when a device it waits for is not there and its round retries: it
    /// read as waiting for playback to stop, for good, while idle.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_waiting_on_an_absent_device_is_not_pending(
    ) -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "songs: songs\nprofiles:\n  - audio:\n      device: no-such-device-anywhere\n",
        )?;
        player.set_config_store(std::sync::Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path,
        )));
        player.play().await?;
        player.reload_hardware().await?;
        assert!(player.hardware_reload_pending());
        player.stop().await;
        eventually(
            || !player.hardware_reload_pending(),
            "pending while the round retries",
        );
        assert!(!player.hardware_status().init_done, "the round is retrying");
        Ok(())
    }

    /// The deferred reload runs when a song ends by itself, too.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_runs_when_the_song_ends() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_playlist("songs:\n- Song 1\n").await?;
        let _dir = set_store_without_audio(&player)?;
        player.play().await?;
        player.reload_hardware().await?;
        assert!(player.audio_device().is_some(), "reloaded under the song");
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        Ok(())
    }

    /// A reload waiting for the player to go idle cannot run in the middle
    /// of a start: the start holds the join lock, and the reload takes it
    /// and finds the song playing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_waits_for_a_start_under_way() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _dir = set_store_without_audio(&player)?;
        let mut join = player.join.lock().await;
        player
            .hardware_reload_pending
            .store(true, Ordering::Relaxed);
        player.reload_deferred_hardware();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(player.audio_device().is_some(), "reloaded under the start");
        player.play_from_locked(Duration::ZERO, &mut join).await?;
        drop(join);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(player.audio_device().is_some(), "reloaded under the song");
        assert!(player.is_playing().await);
        assert!(player.hardware_reload_pending.load(Ordering::Relaxed));

        player.stop().await;
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        Ok(())
    }

    /// A reload in an auto play's gap set the hardware initializing, and the
    /// song failed to start when the gap ended, with nothing to retry it
    /// (#578). The gap counts as playing: the reload waits for the song.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_in_an_auto_plays_gap_waits_for_its_song() -> Result<(), Box<dyn Error>> {
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 300ms\n";
        let player = make_test_player_with_playlist(yaml).await?;
        let _dir = set_store_without_audio(&player)?;
        player.next().await;
        assert!(player.auto_play_status().is_some(), "waiting");

        player.reload_hardware().await?;
        assert!(player.audio_device().is_some(), "reloaded in the gap");
        eventually_async(
            || async { player.is_playing().await },
            "Song 3 never started",
        )
        .await;
        assert!(player.audio_device().is_some(), "reloaded under the song");

        player.stop().await;
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        Ok(())
    }

    /// An auto play called off leaves the player idle: a reload waiting for
    /// it runs then.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_runs_when_an_auto_play_is_called_off() -> Result<(), Box<dyn Error>>
    {
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 10s\n";
        let player = make_test_player_with_playlist(yaml).await?;
        let _dir = set_store_without_audio(&player)?;
        player.next().await;
        player.reload_hardware().await?;
        assert!(player.audio_device().is_some(), "reloaded in the gap");
        assert!(player.cancel_waiting_auto_play());
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_hardware_no_config_store() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // No config store set — reload should fail.
        let result = player.reload_hardware().await;
        assert!(
            result.is_err(),
            "reload_hardware should fail without config store"
        );

        Ok(())
    }

    /// A config change applied with no config store to reload from is not
    /// applied, and says why; with one, and the player idle, it is.
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_config_change_says_what_became_of_it() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        match player.apply_config_change().await.hardware {
            ConfigApply::Failed(e) => assert!(e.contains("config store"), "{e}"),
            other => panic!("applied with no store: {other:?}"),
        }
        let _store = set_store_without_audio(&player)?;
        assert_eq!(
            player.apply_config_change().await.hardware,
            ConfigApply::Applied
        );
        player.await_hardware_ready().await;
        Ok(())
    }

    /// Two rescans overlapping across a change to `songs:` both read the
    /// config as it is when each scans, under the scan lock, and take their
    /// sources up under it too: the songs and the sources the player ends
    /// with are the same set, the newest config's. Resolved before the lock,
    /// the first could swap in the old set last, or leave the songs of one
    /// and the sources of the other.
    #[tokio::test(flavor = "multi_thread")]
    async fn overlapping_rescans_end_on_the_newest_configs_songs() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        for (set, song) in [("songs_a", "Song A"), ("songs_b", "Song B")] {
            let song_dir = dir.path().join(set).join("one");
            fs::create_dir_all(&song_dir)?;
            crate::testutil::write_wav(song_dir.join("kick.wav"), vec![vec![0_i32; 441]], 44100)?;
            fs::write(
                song_dir.join("song.yaml"),
                format!("name: {song}\ntracks:\n- name: kick\n  file: kick.wav\n"),
            )?;
        }
        let path = dir.path().join("mtrack.yaml");
        fs::write(&path, "songs: songs_a\n")?;
        let player = Arc::new(assets_player_with_devices(no_devices())?);
        player.set_config_store(Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path.clone(),
        )));

        // The scan lock held while both rescans are asked for, and the
        // config changed between them.
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (locked, is_locked) = std::sync::mpsc::channel::<()>();
        let holder = {
            let scans = player.song_scans.clone();
            std::thread::spawn(move || {
                let _scan = scans.lock();
                locked.send(()).unwrap();
                held.recv().unwrap();
            })
        };
        is_locked.recv()?;
        let first = {
            let player = player.clone();
            tokio::spawn(async move { player.reload_songs().await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        fs::write(&path, "songs: songs_b\n")?;
        let store = player.config_store().expect("a store");
        store.lock_for_disk_write().await.reload().await?;
        let second = {
            let player = player.clone();
            tokio::spawn(async move { player.reload_songs().await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        release.send(())?;
        holder.join().unwrap();
        first.await??;
        second.await??;

        let sources = player.song_sources().expect("sources");
        assert!(sources.songs.ends_with("songs_b"), "{:?}", sources.songs);
        assert!(player.songs().get("Song B").is_ok(), "the newest set");
        assert!(
            player.songs().get("Song A").is_err(),
            "the old set swapped in"
        );
        Ok(())
    }

    /// A song reload that fails beside a hardware reload is carried with
    /// it: neither hides the other.
    #[test]
    fn a_config_change_carries_the_songs_beside_the_hardware() {
        let change = ConfigChange {
            hardware: ConfigApply::Deferred,
            songs_error: Some("unreadable".into()),
            awaits: None,
        };
        assert!(!change.applied());
        assert_eq!(change.kind(), "deferred");
        let message = change.message().unwrap();
        assert!(
            message.contains("idle") && message.contains("unreadable"),
            "{message}"
        );
        let change = ConfigChange {
            hardware: ConfigApply::Applied,
            songs_error: Some("unreadable".into()),
            awaits: None,
        };
        assert!(!change.applied());
        assert!(change.message().unwrap().contains("unreadable"));
        assert!(ConfigChange::from(ConfigApply::Applied).applied());
    }

    /// While a song plays, a config change is deferred until it stops.
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_config_change_waits_for_a_song() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _store = set_store_without_audio(&player)?;
        player.play().await?;
        assert_eq!(
            player.apply_config_change().await.hardware,
            ConfigApply::Deferred
        );
        assert!(player.audio_device().is_some(), "reset under the song");
        player.stop().await;
        crate::testutil::eventually(
            || player.audio_device().is_none(),
            "the deferred change was never applied",
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_hardware_status_no_devices() -> Result<(), Box<dyn Error>> {
        let player = assets_player().audio(None).ready().await?;
        let status = player.hardware_status();

        assert!(status.init_done);
        assert_eq!(status.audio.status, "not_connected");
        assert_eq!(status.midi.status, "not_connected");
        assert_eq!(status.dmx.status, "not_connected");
        assert_eq!(status.trigger.status, "not_connected");
        assert!(status.audio.name.is_none());
        assert!(status.midi.name.is_none());
        assert!(status.dmx.name.is_none());
        assert!(status.trigger.name.is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_hardware_status_with_devices() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let status = player.hardware_status();

        assert!(status.init_done);
        assert_eq!(status.audio.status, "connected");
        assert!(status.audio.name.is_some());
        assert_eq!(status.midi.status, "connected");
        assert!(status.midi.name.is_some());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_list_playlists() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let names = player.list_playlists();
        // Should contain at least "all_songs" and "playlist", sorted.
        assert!(names.contains(&"all_songs".to_string()));
        assert!(names.contains(&"playlist".to_string()));
        assert_eq!(
            names,
            {
                let mut sorted = names.clone();
                sorted.sort();
                sorted
            },
            "list_playlists should return sorted names"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_persisted_playlist_name() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // Active playlist is "playlist", so persisted should also be "playlist".
        assert_eq!(player.persisted_playlist_name(), "playlist");

        // After switching to all_songs, persisted should still be "playlist".
        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.persisted_playlist_name(), "playlist");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_playlists_snapshot() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let snapshot = player.playlists_snapshot();
        assert!(snapshot.contains_key("all_songs"));
        assert!(snapshot.contains_key("playlist"));
        assert_eq!(snapshot.len(), 2);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_songs() -> Result<(), Box<dyn Error>> {
        let player = assets_player_with_devices(no_devices())?;
        let initial_count = player.songs().len();

        // Create a temp directory that mirrors assets/ layout with an extra song.
        let temp_dir = tempfile::tempdir()?;
        let songs_dir = temp_dir.path().join("songs");
        fs::create_dir_all(&songs_dir)?;

        // Copy existing song YAML configs into temp songs dir.
        let src = Path::new("assets/songs");
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "yaml") {
                fs::copy(entry.path(), songs_dir.join(entry.file_name()))?;
            }
        }

        // Copy required audio/MIDI assets into the temp parent (songs configs use ../ paths).
        let assets = Path::new("assets");
        for entry in fs::read_dir(assets)? {
            let entry = entry?;
            if entry.path().is_file() {
                fs::copy(entry.path(), temp_dir.path().join(entry.file_name()))?;
            }
        }

        // Add a new song config referencing a copied audio file.
        let new_song_yaml =
            "name: \"New Test Song\"\ntracks:\n  - name: click\n    file: ../1Channel44.1k.wav\n";
        fs::write(songs_dir.join("newsong.yaml"), new_song_yaml)?;

        player.set_song_sources(SongSources {
            songs: songs_dir,
            playlists_dir: None,
            legacy_playlist: None,
        });
        player.reload_songs().await?;
        assert!(
            player.songs().len() > initial_count,
            "reload_songs should discover the new song (was {}, now {})",
            initial_count,
            player.songs().len(),
        );
        Ok(())
    }

    /// A reload that cannot rescan says so, and leaves the songs as they
    /// were: a surface answering "saved" over it left the player on stale
    /// songs with nothing said.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_that_cannot_rescan_says_so_and_keeps_the_songs() -> Result<(), Box<dyn Error>>
    {
        let player = assets_player_with_devices(no_devices())?;
        let before = player.songs().len();
        assert!(before > 0);
        let dir = tempfile::tempdir()?;
        player.set_song_sources(SongSources {
            songs: dir.path().join("no such songs"),
            playlists_dir: None,
            legacy_playlist: None,
        });
        let err = player
            .reload_songs()
            .await
            .expect_err("a rescan of nothing");
        assert!(matches!(err, ReloadSongsError::Songs(_)), "{err}");
        assert_eq!(player.songs().len(), before, "the songs were kept");
        Ok(())
    }

    /// A player with no song sources and no config store has nothing to
    /// reload from, and says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_with_no_sources_says_so() -> Result<(), Box<dyn Error>> {
        let player = assets_player_with_devices(no_devices())?;
        assert!(matches!(
            player.reload_songs().await,
            Err(ReloadSongsError::NoSources)
        ));
        Ok(())
    }

    /// The song sources resolve the songs and playlists directories,
    /// defaulting beside the config, and the command line's playlist over
    /// the config's, against the config's directory.
    #[test]
    fn song_sources_resolve_against_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        std::fs::create_dir(dir.join("songs")).unwrap();
        let path = dir.join("mtrack.yaml");
        let mut config = config::Player::default();
        config.set_songs("songs");
        let sources = SongSources::resolve(&config, &path, None);
        assert_eq!(sources.songs, dir.join("songs"));
        assert_eq!(sources.playlists_dir, Some(dir.join("playlists")));
        assert_eq!(sources.legacy_playlist, None);
        let sources = SongSources::resolve(&config, &path, Some(Path::new("setlist.yaml")));
        assert_eq!(sources.legacy_playlist, Some(dir.join("setlist.yaml")));
    }

    #[test]
    fn test_load_playlists_standalone() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;

        // Load with legacy playlist only.
        let playlists =
            super::load_playlists(None, Some(Path::new("assets/playlist.yaml")), songs.clone())?;
        assert!(playlists.contains_key("all_songs"));
        assert!(playlists.contains_key("playlist"));
        assert_eq!(playlists["playlist"].songs().len(), 5);

        // Load with no playlists dir and no legacy — just all_songs.
        let playlists = super::load_playlists(None, None, songs.clone())?;
        assert!(playlists.contains_key("all_songs"));
        assert_eq!(playlists.len(), 1);

        // Load from a playlists directory.
        let temp_dir = tempfile::tempdir()?;
        let pl_dir = temp_dir.path();
        fs::write(pl_dir.join("my_set.yaml"), "songs:\n- Song 1\n- Song 3\n")?;
        let playlists = super::load_playlists(Some(pl_dir), None, songs)?;
        assert!(playlists.contains_key("all_songs"));
        assert!(playlists.contains_key("my_set"));
        assert_eq!(playlists["my_set"].songs().len(), 2);

        Ok(())
    }

    /// The running profile's file (`hardware.profile_file`, which the web UI
    /// links it by) is the one it was loaded from, and stays so after the
    /// profile is updated by index, as MCP, the web UI and gRPC do.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_running_profiles_file_survives_a_profile_update() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join("profiles"))?;
        std::fs::create_dir(dir.path().join("songs"))?;
        std::fs::write(
            dir.path().join("profiles/rig.yml"),
            "kind: hardware_profile\n",
        )?;
        let path = dir.path().join("mtrack.yaml");
        std::fs::write(&path, "songs: songs\nprofiles_dir: profiles\n")?;
        let config = config::Player::deserialize(&path)?;
        let songs = songs::get_all_songs(&dir.path().join("songs"))?;
        let playlists = super::load_playlists(None, None, songs)?;
        let player = Player::new(playlists, "playlist".to_string(), &config, Some(dir.path()))?;
        let store = Arc::new(config::store::ConfigStore::new(config, path.clone()));
        player.set_config_store(store.clone());
        player.await_hardware_ready().await;
        assert_eq!(
            player.hardware_status().profile_file.as_deref(),
            Some("rig.yml")
        );

        let snap = store.read().await?;
        let updated = config::Profile::new(None, None, None, None);
        let snap = store.update_profile(0, updated, &snap.checksum).await?;
        assert_eq!(snap.config.all_profiles()[0].source_file(), Some("rig.yml"));
        let _ = player.apply_config_change().await;
        player.await_hardware_ready().await;
        assert_eq!(
            player.hardware_status().profile_file.as_deref(),
            Some("rig.yml")
        );
        Ok(())
    }

    /// With both `set.yaml` and `set.yml`, the loader takes `set.yaml` (it
    /// took `set.yml`, the later file name, before); `all_songs.yaml` and
    /// `loud.YAML` are not playlists.
    #[test]
    fn load_playlists_takes_a_names_yaml_over_its_yml() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let temp_dir = tempfile::tempdir()?;
        let pl_dir = temp_dir.path();
        fs::write(pl_dir.join("set.yaml"), "songs:\n- Song 1\n")?;
        fs::write(pl_dir.join("set.yml"), "songs:\n- Song 1\n- Song 3\n")?;
        fs::write(pl_dir.join("old.yml"), "songs:\n- Song 3\n")?;
        fs::write(pl_dir.join("all_songs.yaml"), "songs:\n- Song 1\n")?;
        fs::write(pl_dir.join("loud.YAML"), "songs:\n- Song 1\n")?;
        let playlists = super::load_playlists(Some(pl_dir), None, songs.clone())?;
        assert_eq!(playlists["set"].songs(), &vec!["Song 1".to_string()]);
        assert_eq!(playlists["old"].songs(), &vec!["Song 3".to_string()]);
        assert_eq!(
            playlists[playlist::ALL_SONGS].songs().len(),
            songs.sorted_list().len()
        );
        assert!(!playlists.contains_key("loud"));
        assert_eq!(playlists.len(), 3);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_valid() -> Result<(), Box<dyn Error>> {
        let player = assets_player().midi("mock-midi-device").build()?;
        player.await_hardware_ready().await;
        let device = player.audio_device().expect("audio device").to_mock()?;

        let result = player
            .play_song_from("Song 2", std::time::Duration::ZERO)
            .await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().name(), "Song 2");
        eventually(|| device.is_playing(), "Song never started playing");

        // Song 2 is not in the active playlist, so this switches to
        // all_songs (session-only) for the duration of playback.
        assert_eq!(player.get_playlist().name(), "all_songs");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_keeps_active_playlist() -> Result<(), Box<dyn Error>> {
        let player = assets_player().midi("mock-midi-device").build()?;
        player.await_hardware_ready().await;
        let device = player.audio_device().expect("audio device").to_mock()?;

        // Song 3 is in the active playlist: jumping to it must not switch
        // the player to all_songs.
        let result = player
            .play_song_from("Song 3", std::time::Duration::ZERO)
            .await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().name(), "Song 3");
        eventually(|| device.is_playing(), "Song never started playing");
        assert_eq!(player.get_playlist().name(), "playlist");
        assert_eq!(
            player
                .get_playlist()
                .current()
                .map(|s| s.name().to_string()),
            Some("Song 3".to_string())
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_announces_the_song_it_moves_to() -> Result<(), Box<dyn Error>> {
        struct Announced(parking_lot::Mutex<Vec<String>>);
        impl SongChangeNotifier for Announced {
            fn notify(&self, song: &Song) {
                self.0.lock().push(song.name().to_string());
            }
        }
        let player = make_test_player().await?;
        let announced = Arc::new(Announced(Default::default()));
        player.add_song_change_notifier(announced.clone());
        // The playlist moves to Song 3 as a navigation moves it, and says
        // so: a MIDI controller showing the current song kept showing the
        // one before.
        player
            .play_song_from("Song 3", std::time::Duration::ZERO)
            .await?;
        assert_eq!(
            announced.0.lock().last().map(String::as_str),
            Some("Song 3")
        );
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_not_found() -> Result<(), Box<dyn Error>> {
        let player = assets_player().midi("mock-midi-device").build()?;
        player.await_hardware_ready().await;

        let result = player
            .play_song_from("Nonexistent Song", std::time::Duration::ZERO)
            .await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.to_string(), "unable to find song Nonexistent Song");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_while_playing() -> Result<(), Box<dyn Error>> {
        let player = assets_player().midi("mock-midi-device").build()?;
        player.await_hardware_ready().await;
        let device = player.audio_device().expect("audio device").to_mock()?;

        // Start playing first
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // play_song_from while already playing should return None
        let result = player
            .play_song_from("Song 2", std::time::Duration::ZERO)
            .await?;
        assert!(result.is_none());

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_loop_section_not_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // loop_section when nothing is playing should error.
        let result = player.loop_section("verse").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no song is playing"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_loop_section_not_found() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Test songs have no sections/beat grid, so any section name should fail.
        let result = player.loop_section("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_section_loop_clears_state() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // active_section should be None initially.
        assert!(player.active_section().is_none());

        // Manually set active section state to simulate an active loop.
        {
            let mut active = player.section_loop.active.write();
            *active = Some(SectionBounds {
                name: "test".to_string(),
                start_time: Duration::from_secs(1),
                end_time: Duration::from_secs(5),
            });
        }
        assert!(player.active_section().is_some());

        // stop_section_loop should clear it.
        player.stop_section_loop();
        assert!(player.active_section().is_none());
        assert!(player.section_loop.break_requested.load(Ordering::Relaxed));

        Ok(())
    }

    /// Just past a section loop's jump, the position is already back in the
    /// section: it counts the jumps from the playback's log as they fall
    /// due, not from the counter the audio bumped up to a monitor poll late,
    /// which read past the section's end and flipped the reactive offer to
    /// the next section and back at every loop.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_position_counts_a_section_loop_jump_as_it_falls_due() -> Result<(), Box<dyn Error>>
    {
        let player = make_test_player().await?;
        // Gaps of tens of seconds, so a slow runner cannot cross them.
        let at = |since: Duration| {
            let log = Arc::new(crate::playsync::SectionJumpLog::default());
            log.push(crate::section_loop::SectionJump {
                at: Duration::from_secs(100),
                from: Duration::from_secs(100),
                to: Duration::from_secs(4),
                overlap: Duration::ZERO,
                pass: false,
            });
            log.set_timeline(
                crate::clock::PlaybackClock::wall_from(std::time::Instant::now() - since),
                Duration::ZERO,
            );
            (log, std::time::SystemTime::now() - since)
        };
        // Just past the jump: back in the section, not 96s past its end.
        let (log, start) = at(Duration::from_millis(100_003));
        *player.section_jumps.lock() = log;
        *player.play_start_time.lock().await = Some(PlayPosition::Running(start));
        let position = player.elapsed().await?.unwrap();
        assert!(
            position >= Duration::from_secs(4) && position < Duration::from_secs(50),
            "{position:?}"
        );
        // Well before it falls due, the song is where the clock says.
        let (log, start) = at(Duration::from_secs(50));
        *player.section_jumps.lock() = log;
        *player.play_start_time.lock().await = Some(PlayPosition::Running(start));
        let position = player.elapsed().await?.unwrap();
        assert!(
            position >= Duration::from_secs(50) && position < Duration::from_secs(100),
            "{position:?}"
        );
        Ok(())
    }

    /// The position runs on the playback's clock once it starts, as the
    /// jumps it counts do. Taken from the wall clock instead, it read just
    /// before the section's start after a jump whenever the playback clock
    /// read a little ahead (the audio's counter, just after a callback): the
    /// reactive offer reset at every loop.
    #[tokio::test]
    async fn the_position_runs_on_the_playback_clock() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        log.push(crate::section_loop::SectionJump {
            at: Duration::from_secs(100),
            from: Duration::from_secs(100),
            to: Duration::from_secs(4),
            overlap: Duration::ZERO,
            pass: false,
        });
        // Clock 10ms ahead of the wall: it reads 100.005s, the wall 99.995s.
        log.set_timeline(
            crate::clock::PlaybackClock::wall_from(
                std::time::Instant::now() - Duration::from_millis(100_005),
            ),
            Duration::ZERO,
        );
        *player.section_jumps.lock() = log;
        *player.play_start_time.lock().await = Some(PlayPosition::Running(
            std::time::SystemTime::now() - Duration::from_millis(99_995),
        ));
        let position = player.elapsed().await?.unwrap();
        assert!(
            position >= Duration::from_secs(4),
            "before the section: {position:?}"
        );
        Ok(())
    }

    /// A looping song's passes count in the position as sections' jumps do,
    /// each as it falls due: a pass that crossfades starts the song over its
    /// overlap before the last one ends. (They used to be counted by the
    /// audio apart from the log.)
    #[tokio::test(flavor = "multi_thread")]
    async fn the_position_counts_whole_song_passes_from_the_log() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        // A 100s song crossfading over its last 4s: passes at 96s and 192s.
        for at in [96, 192] {
            log.push(crate::section_loop::SectionJump {
                at: Duration::from_secs(at),
                from: Duration::from_secs(96),
                to: Duration::ZERO,
                overlap: Duration::from_secs(4),
                pass: true,
            });
        }
        // 200s into the playback: 8s into its third pass.
        log.set_timeline(
            crate::clock::PlaybackClock::wall_from(
                std::time::Instant::now() - Duration::from_secs(200),
            ),
            Duration::ZERO,
        );
        *player.section_jumps.lock() = log;
        *player.play_start_time.lock().await = Some(PlayPosition::Running(
            std::time::SystemTime::now() - Duration::from_secs(200),
        ));
        let position = player.elapsed().await?.unwrap();
        assert!(
            position >= Duration::from_secs(8) && position < Duration::from_secs(96),
            "not in the third pass: {position:?}"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_active_section_getter() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        assert!(player.active_section().is_none());

        let bounds = SectionBounds {
            name: "chorus".to_string(),
            start_time: Duration::from_secs(10),
            end_time: Duration::from_secs(20),
        };
        *player.section_loop.active.write() = Some(bounds.clone());

        let active = player.active_section().unwrap();
        assert_eq!(active.name, "chorus");
        assert_eq!(active.start_time, Duration::from_secs(10));
        assert_eq!(active.end_time, Duration::from_secs(20));

        Ok(())
    }

    /// Reactive looping, with sections back to back (as a song's are): a
    /// loop broken in one, playback goes straight on into the next, which is
    /// offered for its own ack. It used to stay on the broken loop for good,
    /// as only a stretch outside every section moved it on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_broken_loop_offers_the_section_after_it() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let songs_dir = dir.path().join("songs");
        let song_dir = songs_dir.join("S");
        std::fs::create_dir_all(&song_dir)?;
        crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 10]], 8000)?;
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: S\ntracks:\n  - name: t\n    file: t.wav\ntempo:\n  bpm: 120\n  \
             time_signature: 4/4\nsections:\n  - name: a\n    start_measure: 1\n    \
             end_measure: 2\n  - name: b\n    start_measure: 2\n    end_measure: 3\n",
        )?;
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs:\n- S\n")?;
        let songs = songs::get_all_songs(&songs_dir)?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(&playlist_path)?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                songs_dir.to_str().unwrap(),
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let song = player.get_playlist().current().unwrap();
        let (a_start, a_end) = song.resolve_section("a").unwrap();
        let (_, b_end) = song.resolve_section("b").unwrap();
        *player.section_loop.reactive.write() = ReactiveLoopState::Looping(SectionBounds {
            name: "a".to_string(),
            start_time: a_start,
            end_time: a_end,
        });
        player.stop_section_loop();
        // Still in a, finishing its pass; then into b.
        player.check_section_boundaries(a_end - Duration::from_millis(100));
        player.check_section_boundaries(a_end + Duration::from_millis(100));
        assert!(b_end > a_end + Duration::from_millis(100));
        let state = player.reactive_loop_state();
        assert!(
            matches!(&state, ReactiveLoopState::SectionOffered(b) if b.name == "b"),
            "{state:?}"
        );
        assert!(player.section_ack().await.is_ok());
        Ok(())
    }

    /// A section loops in a song looping with a crossfade, as in any other:
    /// `loop_section` takes it (it used to refuse such a song), and reactive
    /// looping offers its sections.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_section_loops_in_a_crossfading_song() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let songs_dir = dir.path().join("songs");
        let song_dir = songs_dir.join("S");
        std::fs::create_dir_all(&song_dir)?;
        crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 10]], 8000)?;
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: S\ntracks:\n  - name: t\n    file: t.wav\ntempo:\n  bpm: 120\n  \
             time_signature: 4/4\nloop_playback: true\nloop_crossfade: 1measure\n\
             sections:\n  - name: a\n    start_measure: 1\n    end_measure: 2\n  \
             - name: b\n    start_measure: 2\n    end_measure: 3\n",
        )?;
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs:\n- S\n")?;
        let songs = songs::get_all_songs(&songs_dir)?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(&playlist_path)?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                songs_dir.to_str().unwrap(),
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let song = player.get_playlist().current().unwrap();
        assert!(song.loop_crossfade().is_some());
        let (a_start, a_end) = song.resolve_section("a").unwrap();
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.loop_section("a").await?;
        let active = player.active_section().expect("the section armed");
        assert_eq!((active.start_time, active.end_time), (a_start, a_end));
        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        // Reactive looping offers the section playback is in.
        player.check_section_boundaries(a_start + Duration::from_millis(100));
        assert!(
            !matches!(player.reactive_loop_state(), ReactiveLoopState::Idle),
            "nothing offered: {:?}",
            player.reactive_loop_state()
        );
        Ok(())
    }

    /// A player on a playlist of songs S and T, each with a section `a`
    /// from 2 s to 4 s, on the mock audio device.
    async fn two_section_songs_player() -> Result<(tempfile::TempDir, Arc<Player>), Box<dyn Error>>
    {
        let dir = tempfile::tempdir()?;
        let songs_dir = dir.path().join("songs");
        for name in ["S", "T"] {
            let song_dir = songs_dir.join(name);
            std::fs::create_dir_all(&song_dir)?;
            crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 10]], 8000)?;
            std::fs::write(
                song_dir.join("song.yaml"),
                format!(
                    "name: {name}\ntracks:\n  - name: t\n    file: t.wav\ntempo:\n  bpm: 120\n  \
                     time_signature: 4/4\nsections:\n  - name: a\n    start_measure: 2\n    \
                     end_measure: 3\n"
                ),
            )?;
        }
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs:\n- S\n- T\n")?;
        let songs = songs::get_all_songs(&songs_dir)?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(&playlist_path)?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                songs_dir.to_str().unwrap(),
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok((dir, player))
    }

    /// A phone's status names the section a loop is set on from the call
    /// until the loop is stopped, as the web UI marks it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_phones_status_names_the_looping_section() -> Result<(), Box<dyn Error>> {
        use crate::controller::ble::status::from_player;

        let (_dir, player) = two_section_songs_player().await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        let status = from_player(
            &player.transport_status().await,
            player.gains_revision(),
            None,
            player.is_locked(),
        )?;
        assert_eq!(status.loop_section, None);

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.loop_section("a").await?;
        let status = from_player(
            &player.transport_status().await,
            player.gains_revision(),
            None,
            player.is_locked(),
        )?;
        assert_eq!(status.loop_section.as_deref(), Some("a"));

        player.stop_section_loop();
        let status = from_player(
            &player.transport_status().await,
            player.gains_revision(),
            None,
            player.is_locked(),
        )?;
        assert_eq!(status.loop_section, None);

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    /// A section loop armed, then Stop: the section is forgotten. Kept, the
    /// next playback -- the same song from the top, or another song after
    /// Next -- looped that section, at the old song's bounds, as soon as it
    /// reached its end, and the status reported it looping.
    #[tokio::test(flavor = "multi_thread")]
    async fn stop_forgets_the_armed_section() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = two_section_songs_player().await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.loop_section("a").await?;
        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        player.next().await;
        player.play().await?;
        eventually(|| device.is_playing(), "Song T never started playing");
        let armed = player.active_section();
        player.stop().await;
        assert!(
            armed.is_none(),
            "song T plays with song S's section still armed: {armed:?}"
        );
        Ok(())
    }

    /// A section loop armed, then a pause and a resume: the operator paused
    /// mid-loop, and the song loops on. A reactive arm (an ack) is kept with
    /// it, and counts its jumps from the resumed playback's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_and_resume_keep_the_armed_section() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = two_section_songs_player().await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.loop_section("a").await?;
        let armed = player.active_section().expect("the section armed");
        // As an ack arms it in reactive looping.
        *player.section_loop.reactive.write() = ReactiveLoopState::LoopArmed(armed.clone());
        player.pause().await?;
        eventually(|| !device.is_playing(), "Song never paused");
        assert!(player.is_paused());
        assert_eq!(player.active_section(), Some(armed.clone()), "kept paused");
        player.play().await?;
        eventually(|| device.is_playing(), "Song never resumed");
        assert_eq!(player.active_section(), Some(armed.clone()), "kept resumed");
        assert!(
            matches!(player.reactive_loop_state(), ReactiveLoopState::LoopArmed(ref b) if *b == armed),
            "{:?}",
            player.reactive_loop_state()
        );
        assert_eq!(*player.section_loop.jumps_at_arm.lock(), 0);
        // A stop then forgets it all.
        player.stop().await;
        assert!(player.active_section().is_none());
        assert!(matches!(
            player.reactive_loop_state(),
            ReactiveLoopState::Idle
        ));
        Ok(())
    }

    /// A pause on the song's very end keeps nothing to resume, and leaves
    /// the player stopped: the section loop goes with it, rather than the
    /// status showing a loop while stopped.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_on_the_very_end_forgets_the_armed_section() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = two_section_songs_player().await?;
        let song = player.get_playlist().current().expect("S");
        let (start_time, end_time) = song.resolve_section("a").expect("section a");
        *player.section_loop.active.write() = Some(SectionBounds {
            name: "a".to_string(),
            start_time,
            end_time,
        });
        player.keep_paused_position(song.length());
        assert!(!player.is_paused(), "nothing to resume");
        assert!(
            player.active_section().is_none(),
            "stopped with a loop armed"
        );
        Ok(())
    }

    /// A pause, then a navigation: the section loop kept for the resume is
    /// forgotten with it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_navigation_from_a_pause_forgets_the_armed_section() -> Result<(), Box<dyn Error>> {
        let (_dir, player) = two_section_songs_player().await?;
        let device = player.audio_device().expect("audio device").to_mock()?;
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.loop_section("a").await?;
        player.pause().await?;
        eventually(|| !device.is_playing(), "Song never paused");
        player.next().await;
        player.play().await?;
        eventually(|| device.is_playing(), "Song T never started playing");
        let armed = player.active_section();
        player.stop().await;
        assert!(armed.is_none(), "T plays with S's section armed: {armed:?}");
        Ok(())
    }

    /// Arming while another task holds `play_start_time` (a status poll in
    /// `elapsed()`, a seek) counts the jumps due as any other time: it read
    /// none (a `try_lock` that failed), so a section armed after an earlier
    /// loop in the same playback showed Looping at once, before it had gone
    /// round.
    #[tokio::test(flavor = "multi_thread")]
    async fn arming_under_contention_does_not_report_looping_early() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let songs_dir = dir.path().join("songs");
        let song_dir = songs_dir.join("S");
        std::fs::create_dir_all(&song_dir)?;
        crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 10]], 8000)?;
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: S\ntracks:\n  - name: t\n    file: t.wav\ntempo:\n  bpm: 120\n  \
             time_signature: 4/4\nsections:\n  - name: a\n    start_measure: 1\n    \
             end_measure: 2\n  - name: b\n    start_measure: 2\n    end_measure: 3\n",
        )?;
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs:\n- S\n")?;
        let songs = songs::get_all_songs(&songs_dir)?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(&playlist_path)?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                songs_dir.to_str().unwrap(),
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let song = player.get_playlist().current().unwrap();
        let (a_start, a_end) = song.resolve_section("a").unwrap();
        let (b_start, b_end) = song.resolve_section("b").unwrap();
        // a looped once, long ago on the playback timeline.
        let log = Arc::new(crate::playsync::SectionJumpLog::default());
        log.push(crate::section_loop::SectionJump {
            at: a_end,
            from: a_end,
            to: a_start,
            overlap: Duration::ZERO,
            pass: false,
        });
        // Now in b, mid-way: raw = a's pass put back + b's middle.
        let mid_b = b_start + (b_end - b_start) / 2;
        let raw = mid_b + (a_end - a_start);
        log.set_timeline(
            crate::clock::PlaybackClock::wall_from(std::time::Instant::now() - raw),
            Duration::ZERO,
        );
        *player.section_jumps.lock() = log;
        *player.play_start_time.lock().await =
            Some(PlayPosition::Running(std::time::SystemTime::now() - raw));
        let b = SectionBounds {
            name: "b".to_string(),
            start_time: b_start,
            end_time: b_end,
        };
        *player.section_loop.reactive.write() = ReactiveLoopState::SectionOffered(b);
        {
            // Another task in elapsed() / a seek, holding the lock.
            let _held = player.play_start_time.lock().await;
            assert!(player.section_ack().await.is_ok());
        }
        player.check_section_boundaries(mid_b);
        let state = player.reactive_loop_state();
        assert!(
            matches!(&state, ReactiveLoopState::LoopArmed(_)),
            "b has not gone round, yet: {state:?}"
        );
        Ok(())
    }

    /// A player on the mock device whose playlist is A, B, C, with B a
    /// 10-second looping song, playing B.
    async fn player_looping_b() -> Result<(Arc<Player>, tempfile::TempDir), Box<dyn Error>> {
        player_looping_b_with("songs:\n- A\n- B\n- C\n").await
    }

    /// As [`player_looping_b`], with the playlist written as `playlist`.
    async fn player_looping_b_with(
        playlist: &str,
    ) -> Result<(Arc<Player>, tempfile::TempDir), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let songs_dir = dir.path().join("songs");
        for (name, looping) in [("A", false), ("B", true), ("C", false)] {
            let song_dir = songs_dir.join(name);
            std::fs::create_dir_all(&song_dir)?;
            crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 10]], 8000)?;
            std::fs::write(
                song_dir.join("song.yaml"),
                format!(
                    "name: {name}\ntracks:\n  - name: t\n    file: t.wav\nloop_playback: {looping}\n{}",
                    // Only C has a grid: one measure is 2 s.
                    if name == "C" { "tempo:\n  bpm: 120\n  time_signature: 4/4\n" } else { "" }
                ),
            )?;
        }
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, playlist)?;
        let songs = songs::get_all_songs(&songs_dir)?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(&playlist_path)?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                songs_dir.to_str().unwrap(),
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        player.get_playlist().navigate_to("B");
        player.play().await?;
        assert!(player.is_current_song_looping());
        Ok((player, dir))
    }

    /// Waits until the playlist sits on `name` with a song playing.
    async fn playing_eventually(player: &Arc<Player>, name: &str) -> Result<(), Box<dyn Error>> {
        let deadline = std::time::Instant::now() + crate::testutil::PATIENCE;
        loop {
            let current = player
                .get_playlist()
                .current()
                .map(|s| s.name().to_string());
            if current.as_deref() == Some(name) && player.is_playing().await {
                return Ok(());
            }
            if std::time::Instant::now() > deadline {
                return Err(format!("never reached {name} playing (at {current:?})").into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn next_on_a_looping_song_reports_where_it_goes() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b().await?;
        let target = player.next().await.map(|s| s.name().to_string());
        assert_eq!(
            target.as_deref(),
            Some("C"),
            "next names the song it moves to"
        );
        playing_eventually(&player, "C").await?;
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn previous_on_a_looping_song_goes_back() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b().await?;
        let target = player.prev().await.map(|s| s.name().to_string());
        assert_eq!(
            target.as_deref(),
            Some("A"),
            "previous names the song it moves to"
        );
        // And goes there: the cleanup used to move on whatever the direction.
        playing_eventually(&player, "A").await?;
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_loop_break_takes_its_fades_from_the_playlist() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b_with(
            "songs:\n- song: A\n  fade_in: {length: 1s, exclude: [t]}\n- song: B\n  fade_out: 2s\n- C\n",
        )
        .await?;
        player.prev().await;
        // B fades out on its entry's setting, and A, which it gives way to,
        // fades in on its own.
        let fade = player.break_fade.lock().clone().expect("a break fade");
        assert_eq!(fade.length, Duration::from_secs(2));
        let fade_in = player.start_fade.lock().clone().expect("a fade-in");
        assert_eq!(fade_in.song, "A");
        let fade_in = fade_in.fade;
        assert_eq!(fade_in.length, Duration::from_secs(1));
        assert_eq!(fade_in.exclude, ["t".to_string()]);
        playing_eventually(&player, "A").await?;
        player.stop().await;
        assert!(player.start_fade.lock().is_none(), "a stop clears it");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fade_in_needs_a_fade_out_to_start_under() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b_with(
            "songs:\n- A\n- song: B\n  fade_out: 0s\n- song: C\n  fade_in: 1s\n",
        )
        .await?;
        player.next().await;
        assert!(player.break_fade.lock().is_none());
        assert!(player.start_fade.lock().is_none());
        playing_eventually(&player, "C").await?;
        player.stop().await;
        Ok(())
    }

    /// Waits until the player has stopped, and checks it stays stopped, on
    /// `name`: nothing started after it.
    async fn rests_stopped_on(player: &Arc<Player>, name: &str) {
        eventually_async(
            || async { !player.is_playing().await },
            "the player never stopped",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!player.is_playing().await, "nothing starts after it");
        assert_eq!(current_name(player).await, name, "the playlist stays put");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_ends_a_looping_song_over_its_fade_out() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 2s\n- C\n").await?;
        let stopped = player.stop().await.map(|s| s.name().to_string());
        assert_eq!(stopped.as_deref(), Some("B"));
        // B fades out on its entry's setting, as a loop break's would.
        let fade = player.break_fade.lock().clone().expect("a fade");
        assert_eq!(fade.length, Duration::from_secs(2));
        let (song, remaining) = player.ending_status().expect("ending");
        assert_eq!(song, "B");
        assert!(remaining <= Duration::from_secs(2), "{remaining:?}");
        rests_stopped_on(&player, "B").await;
        assert!(player.ending_status().is_some(), "still fading");
        assert!(player.elapsed().await?.is_none(), "stopped as by a stop");
        Ok(())
    }

    /// Rewrites the playlist of a player from [`player_looping_b_with`] as
    /// `yaml` and reloads it, as the playlist editor's save does.
    fn edit_playlist(player: &Player, dir: &tempfile::TempDir, yaml: &str) {
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, yaml).unwrap();
        player
            .reload_songs_from(&SongSources {
                songs: dir.path().join("songs"),
                playlists_dir: None,
                legacy_playlist: Some(playlist_path),
            })
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_moving_the_looping_song_keeps_its_ending() -> Result<(), Box<dyn Error>> {
        // #576: the playlist kept its index, so C took B's place and a stop
        // cut rather than fading B out.
        let (player, dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 2s\n- C\n").await?;
        edit_playlist(
            &player,
            &dir,
            "songs:\n- A\n- C\n- song: B\n  fade_out: 2s\n",
        );
        assert_eq!(current_name(&player).await, "B");
        assert_eq!(player.get_playlist().position(), 2);
        let stopped = player.stop().await.map(|s| s.name().to_string());
        assert_eq!(stopped.as_deref(), Some("B"));
        let (song, _) = player.ending_status().expect("B fades out");
        assert_eq!(song, "B");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_moving_a_song_under_one_playing_leaves_it_be() -> Result<(), Box<dyn Error>> {
        // #576: A playing, and an edit putting looping B where A was gave A
        // B's ending.
        let (player, dir) = player_looping_b().await?;
        player.stop().await;
        player.get_playlist().navigate_to("A");
        player.play().await?;
        playing_eventually(&player, "A").await?;
        edit_playlist(
            &player,
            &dir,
            "songs:\n- song: B\n  fade_out: 2s\n- A\n- C\n",
        );
        assert_eq!(current_name(&player).await, "A");
        assert!(!player.is_current_song_looping());
        let stopped = player.stop().await.map(|s| s.name().to_string());
        assert_eq!(stopped.as_deref(), Some("A"));
        assert!(player.ending_status().is_none(), "A is cut, not faded");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_the_active_playlist_keeps_the_playing_song() -> Result<(), Box<dyn Error>> {
        // #576: with "playlist" deleted or renamed while B plays from it, the
        // player falls back to all_songs, which took up its own place, on A.
        let (player, dir) = player_looping_b().await?;
        let empty = dir.path().join("playlists");
        std::fs::create_dir_all(&empty)?;
        player.reload_songs_from(&SongSources {
            songs: dir.path().join("songs"),
            playlists_dir: Some(empty),
            legacy_playlist: None,
        })?;
        assert_eq!(player.get_playlist().name(), "all_songs");
        assert_eq!(current_name(&player).await, "B", "B is still playing");
        assert!(player.is_current_song_looping());
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_song_taken_off_the_list_and_stopped_gives_way() -> Result<(), Box<dyn Error>> {
        // #576: B, taken off the list while it plays, stayed current once
        // stopped, so Play started it again. C took its place.
        let (player, dir) = player_looping_b().await?;
        edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
        player.stop().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(current_name(&player).await, "C");
        assert_eq!(player.get_playlist().entry(), Some(1));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_does_not_lose_a_move_made_while_it_runs() -> Result<(), Box<dyn Error>> {
        // A reload copies each playlist's place, then swaps the new ones in.
        // A song's end advancing the playlist between the two moved the old
        // one, and the new one came in a song behind.
        let (player, dir) = player_looping_b().await?;
        player.stop().await;
        let reload = {
            let player = player.clone();
            let songs = dir.path().join("songs");
            let playlist = dir.path().join("playlist.yaml");
            std::thread::spawn(move || {
                player.reload_songs_from(&SongSources {
                    songs,
                    playlists_dir: None,
                    legacy_playlist: Some(playlist),
                })
            })
        };
        let (_, moved) = player.move_playlist(|playlist| {
            // The reload runs to its end here unless the move holds it off.
            std::thread::sleep(Duration::from_millis(300));
            playlist.next().map(|song| song.name().to_string())
        });
        reload.join().unwrap()?;
        assert_eq!(moved.as_deref(), Some("C"));
        assert_eq!(current_name(&player).await, "C");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_looping_song_an_edit_takes_away_still_breaks() -> Result<(), Box<dyn Error>> {
        // #576: with B gone from the list, the playlist named A, which does
        // not loop, and next did nothing.
        let (player, dir) = player_looping_b().await?;
        edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
        assert_eq!(current_name(&player).await, "B", "B plays on, off the list");
        assert!(player.is_current_song_looping());
        // What took its place is what comes next.
        let target = player.next().await.map(|s| s.name().to_string());
        assert_eq!(target.as_deref(), Some("C"));
        playing_eventually(&player, "C").await?;
        assert_eq!(player.get_playlist().entry(), Some(1));
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_counts_measures_on_the_next_songs_grid() -> Result<(), Box<dyn Error>> {
        // #569: B has no grid of its own, so a fade in measures cut.
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 1measure\n- C\n").await?;
        player.stop().await;
        let fade = player.break_fade.lock().clone().expect("a fade");
        assert_eq!(fade.length, Duration::from_secs(2));
        assert!(player.ending_status().is_some());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_stop_cuts_the_ending() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 10s\n- C\n").await?;
        player.stop().await;
        assert!(player.ending_status().is_some());
        // At once, whether or not the playback has wound down yet.
        player.stop().await;
        assert!(player.ending_status().is_none(), "cut");
        assert!(player.break_fade.lock().is_none(), "cut");
        assert!(!player.is_playing().await);
        rests_stopped_on(&player, "B").await;

        // And once it has.
        player.play().await?;
        playing_eventually(&player, "B").await?;
        player.stop().await;
        rests_stopped_on(&player, "B").await;
        assert!(player.ending_status().is_some());
        player.stop().await;
        assert!(player.ending_status().is_none(), "cut");
        assert!(player.break_fade.lock().is_none(), "cut");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_during_an_ending_stops_the_song() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 10s\n- C\n").await?;
        player.stop().await;
        assert!(player.ending_status().is_some());
        // The operator ended B: a pause has nothing to resume, and stops it
        // as a second stop does, whether or not it has wound down yet.
        assert_eq!(player.pause().await?, None, "nothing to resume");
        assert!(!player.is_paused(), "ending and paused at once");
        assert!(player.ending_status().is_none(), "cut, as by a stop");
        rests_stopped_on(&player, "B").await;
        assert!(player.elapsed().await?.is_none(), "no position kept");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_ending_is_over_when_its_fade_is() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 300ms\n- C\n").await?;
        player.stop().await;
        assert!(player.ending_status().is_some());
        eventually(
            || player.ending_status().is_none(),
            "the ending never ended",
        );
        rests_stopped_on(&player, "B").await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_during_an_ending_starts_the_song_again() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 10s\n- C\n").await?;
        player.stop().await;
        eventually_async(
            || async { !player.is_playing().await },
            "the player never stopped",
        )
        .await;
        // As on a stopped player: the playlist's song, the fade under it.
        player.play().await?;
        playing_eventually(&player, "B").await?;
        assert!(player.ending_status().is_none(), "no longer ending");
        assert!(player.break_fade.lock().is_some(), "fading on under it");
        player.stop().await;
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_cuts_a_looping_song_without_a_fade_out() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b().await?;
        player.stop().await;
        assert!(!player.is_playing().await, "at once");
        assert!(player.ending_status().is_none());
        assert!(player.break_fade.lock().is_none());
        rests_stopped_on(&player, "B").await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_cuts_a_song_that_does_not_loop() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b_with(
            "songs:\n- song: A\n  fade_out: 2s\n- song: B\n  fade_out: 2s\n- C\n",
        )
        .await?;
        player.stop().await;
        player.stop().await;
        player.get_playlist().navigate_to("A");
        player.play().await?;
        playing_eventually(&player, "A").await?;
        player.stop().await;
        assert!(!player.is_playing().await, "at once");
        assert!(player.ending_status().is_none());
        assert!(player.break_fade.lock().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn next_on_the_last_looping_song_ends_it() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 2s\n").await?;
        let at = player.next().await.map(|s| s.name().to_string());
        assert_eq!(at.as_deref(), Some("B"), "where the playlist stays");
        let fade = player.break_fade.lock().clone().expect("a fade");
        assert_eq!(fade.length, Duration::from_secs(2));
        assert!(player.ending_status().is_some());
        // Rather than starting B over.
        rests_stopped_on(&player, "B").await;

        // Without a fade_out, it stops at once.
        let (player, _dir) = player_looping_b_with("songs:\n- A\n- B\n").await?;
        player.next().await;
        assert!(!player.is_playing().await, "at once");
        assert!(player.ending_status().is_none());
        rests_stopped_on(&player, "B").await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn previous_on_the_first_looping_song_ends_it() -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- song: B\n  fade_out: 2s\n- C\n").await?;
        let at = player.prev().await.map(|s| s.name().to_string());
        assert_eq!(at.as_deref(), Some("B"));
        assert!(player.ending_status().is_some());
        rests_stopped_on(&player, "B").await;
        Ok(())
    }

    /// A player on a looping Vamp whose entry fades out over 3s, red under
    /// a blue ambient light with `delay`, the ambient up at startup.
    async fn ending_ambient_player(
        delay: &str,
    ) -> Result<(tempfile::TempDir, Arc<Player>), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let song_dir = dir.path().join("songs").join("vamp");
        fs::create_dir_all(&song_dir)?;
        fs::copy("assets/20-sec-long.wav", song_dir.join("track.wav"))?;
        let wash = |color: &str| {
            format!(
                "show \"s\" {{\n    @0.000\n    wash: static, color: \"{color}\", duration: 30s\n}}\n"
            )
        };
        fs::write(song_dir.join("show.light"), wash("red"))?;
        fs::write(
            song_dir.join("song.yaml"),
            "name: Vamp\ntracks:\n- name: track\n  file: track.wav\n  file_channel: 1\nlighting:\n- file: show.light\nloop_playback: true\n",
        )?;
        let ambient_show = dir.path().join("ambient.light");
        fs::write(&ambient_show, wash("blue"))?;
        let groups = HashMap::from([(
            "wash".to_string(),
            config::lighting::LogicalGroup::new("wash".to_string(), vec![]),
        )]);
        let lighting = config::Lighting::new(None, Some(groups), None).with_ambient(Some(
            config::ambient::Ambient::new(
                Some(ambient_show.display().to_string()),
                Some(delay.to_string()),
                None,
                None,
            ),
        ));
        let dmx = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            Some(lighting),
        );
        let songs = songs::get_all_songs(&dir.path().join("songs"))?;
        assert!(songs.failures().is_empty(), "{:?}", songs.failures());
        let config: config::Playlist = ::config::Config::builder()
            .add_source(::config::File::from_str(
                "songs:\n- song: Vamp\n  fade_out: 3s\n",
                ::config::FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        let playlist = Playlist::new("playlist", &config, songs.clone())?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                Some(dmx),
                HashMap::new(),
                "songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok((dir, player))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ambient_light_counts_its_delay_from_the_start_of_an_ending(
    ) -> Result<(), Box<dyn Error>> {
        let (_dir, player) = ending_ambient_player("500ms").await?;
        let engine = player.dmx_engine().expect("DMX engine should be present");
        let state = || engine.ambient_status().map(|s| s.state);
        eventually(|| state() == Some("playing"), "ambient at startup");
        player.play().await?;
        playing_eventually(&player, "Vamp").await?;
        eventually(|| state().is_none(), "ambient leaves for the song");

        // Ambient's delay counts from the Stop, not from the end of the fade,
        // so it comes in over the fade's tail rather than after a dark gap.
        player.stop().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!player.is_playing().await);
        assert_eq!(state(), Some("pending"), "ambient waits out its delay");
        eventually(|| state() == Some("playing"), "ambient during the ending");
        assert!(player.ending_status().is_some(), "the fade still runs");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cut_ending_brings_ambient_in_at_once() -> Result<(), Box<dyn Error>> {
        // A second Stop is "now": ambient doesn't wait out the rest of its
        // delay over lights hurrying to dark, which left a dark gap.
        let (_dir, player) = ending_ambient_player("30s").await?;
        let engine = player.dmx_engine().expect("DMX engine should be present");
        let state = || engine.ambient_status().map(|s| s.state);
        eventually(|| state() == Some("playing"), "ambient at startup");
        player.play().await?;
        playing_eventually(&player, "Vamp").await?;
        eventually(|| state().is_none(), "ambient leaves for the song");

        player.stop().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(state(), Some("pending"), "ambient waits out its delay");
        player.stop().await;
        assert!(player.ending_status().is_none(), "the cut ends the ending");
        eventually(|| state() == Some("playing"), "ambient at the cut");
        Ok(())
    }

    /// A player whose playlist is a short lit song, then a long one that
    /// starts by itself `gap` after it ends, with DMX.
    async fn auto_play_lit_player(
        gap: &str,
    ) -> Result<(tempfile::TempDir, Arc<Player>), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let show = "show \"s\" {\n    @0.000\n    wash: static, color: \"red\", duration: 30s\n}\n";
        for (name, wav) in [
            ("First", "assets/1Channel44.1k.wav"),
            ("Second", "assets/20-sec-long.wav"),
        ] {
            let song_dir = dir.path().join("songs").join(name);
            fs::create_dir_all(&song_dir)?;
            fs::copy(wav, song_dir.join("track.wav"))?;
            fs::write(song_dir.join("show.light"), show)?;
            fs::write(
                song_dir.join("song.yaml"),
                format!("name: {name}\ntracks:\n- name: track\n  file: track.wav\n  file_channel: 1\nlighting:\n- file: show.light\n"),
            )?;
        }
        let groups = HashMap::from([(
            "wash".to_string(),
            config::lighting::LogicalGroup::new("wash".to_string(), vec![]),
        )]);
        let dmx = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            Some(config::Lighting::new(None, Some(groups), None)),
        );
        let songs = songs::get_all_songs(&dir.path().join("songs"))?;
        assert!(songs.failures().is_empty(), "{:?}", songs.failures());
        let config: config::Playlist = ::config::Config::builder()
            .add_source(::config::File::from_str(
                &format!("songs:\n- First\n- song: Second\n  auto_play: {gap}\n"),
                ::config::FileFormat::Yaml,
            ))
            .build()?
            .try_deserialize()?;
        let playlist = Playlist::new("playlist", &config, songs.clone())?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                Some(dmx),
                HashMap::new(),
                "songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok((dir, player))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ambient_off_goes_dark_in_an_auto_play_gap() -> Result<(), Box<dyn Error>> {
        // The song before held its lights through the gap, Off or not.
        let (_dir, player) = auto_play_lit_player("5s").await?;
        player.set_ambient_mode(crate::dmx::engine::AmbientMode::Off);
        let engine = player.dmx_engine().expect("DMX engine should be present");
        player.play().await?;
        playing_eventually(&player, "First").await?;
        eventually(|| player.auto_play_status().is_some(), "the gap");
        eventually(|| engine.gone_dark(), "dark in the gap");
        player.cancel_waiting_auto_play();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn off_set_in_an_auto_play_gap_goes_dark() -> Result<(), Box<dyn Error>> {
        // In Auto the gap holds the song before's lights, as a segue; Off
        // set then takes them dark.
        let (_dir, player) = auto_play_lit_player("5s").await?;
        let engine = player.dmx_engine().expect("DMX engine should be present");
        player.play().await?;
        playing_eventually(&player, "First").await?;
        eventually(|| player.auto_play_status().is_some(), "the gap");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!engine.gone_dark(), "held through the gap in Auto");
        player.set_ambient_mode(crate::dmx::engine::AmbientMode::Off);
        assert!(engine.gone_dark(), "dark once Off");
        player.cancel_waiting_auto_play();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_is_current_song_looping() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Test songs don't have loop_playback set, so this should be false.
        assert!(!player.is_current_song_looping());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_gains_revision_moves_on_a_change_and_not_on_a_repeat() -> Result<(), Box<dyn Error>>
    {
        let mappings: HashMap<String, Vec<u16>> = HashMap::from([("click".to_string(), vec![1])]);
        let player = assets_player_with_devices(PlayerDevices {
            mappings: Some(Arc::new(mappings)),
            ..no_devices()
        })?;

        let start = player.gains_revision();
        player.set_track_gain("click", -6.0)?;
        assert_eq!(player.gains_revision(), start.wrapping_add(1));
        // Setting the value it already has is not a change.
        player.set_track_gain("click", -6.0)?;
        assert_eq!(player.gains_revision(), start.wrapping_add(1));

        player.set_track_mute("click", true)?;
        assert_eq!(player.gains_revision(), start.wrapping_add(2));
        player.set_track_mute("click", true)?;
        assert_eq!(player.gains_revision(), start.wrapping_add(2));
        player.set_track_mute("click", false)?;
        assert_eq!(player.gains_revision(), start.wrapping_add(3));

        // A refused set changes nothing.
        assert!(player.set_track_gain("nope", 0.0).is_err());
        assert!(player.set_track_gain("click", f32::NAN).is_err());
        assert_eq!(player.gains_revision(), start.wrapping_add(3));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_track_gain_set_and_get() -> Result<(), Box<dyn Error>> {
        let mappings: HashMap<String, Vec<u16>> = HashMap::from([
            ("click".to_string(), vec![1]),
            ("keys".to_string(), vec![2]),
        ]);
        let player = assets_player_with_devices(PlayerDevices {
            mappings: Some(Arc::new(mappings)),
            ..no_devices()
        })?;

        // All tracks start at unity.
        let gains = player.get_track_gains().unwrap();
        assert_eq!(gains.len(), 2);
        assert!(gains.iter().all(|(_, db)| *db == 0.0));

        // Set, clamp, and read back.
        assert_eq!(player.set_track_gain("click", -6.0)?, -6.0);
        assert_eq!(player.set_track_gain("keys", 20.0)?, 12.0);
        let gains: HashMap<String, f32> = player.get_track_gains().unwrap().into_iter().collect();
        assert_eq!(gains["click"], -6.0);
        assert_eq!(gains["keys"], 12.0);

        // Unknown track and non-finite gain are rejected.
        assert!(player.set_track_gain("nope", 0.0).is_err());
        assert!(player.set_track_gain("click", f32::NAN).is_err());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_track_gain_without_audio_errors() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // make_test_player has no track mappings → no gain state.
        if player.get_track_gains().is_none() {
            assert!(player.set_track_gain("click", 0.0).is_err());
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_paused_song_taken_off_the_list_stays_current_to_resume() -> Result<(), Box<dyn Error>>
    {
        // #576: pause left no playback, so the edit gave B's place to C, and
        // play then started C at B's paused position.
        let (player, dir) = player_looping_b().await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let paused_at = player.pause().await?.expect("paused");
        assert!(paused_at > Duration::ZERO);
        edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
        assert_eq!(current_name(&player).await, "B");
        assert_eq!(player.get_playlist().entry(), None);
        assert_eq!(player.pending_start(), Some(paused_at));
        // Play resumes B where it was.
        let played = player.play().await?.map(|s| s.name().to_string());
        assert_eq!(played.as_deref(), Some("B"));
        let elapsed = elapsed_eventually(&player).await?;
        assert!(elapsed >= paused_at, "{elapsed:?} < {paused_at:?}");
        player.stop().await;
        // Stopped, it gives way to the entry that took its place.
        assert_eq!(player.get_playlist().entry(), Some(1));
        assert_eq!(current_name(&player).await, "C");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_paused_song_taken_off_the_list_gives_way_on_a_playlist_switch(
    ) -> Result<(), Box<dyn Error>> {
        let (player, dir) = player_looping_b().await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        player.pause().await?.expect("paused");
        edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
        let left = player.get_playlist();
        player.switch_to_playlist("all_songs").await?;
        assert_eq!(left.entry(), Some(1));
        assert_eq!(left.current().unwrap().name(), "C");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_song_taken_off_the_list_while_ending_gives_way_once_stopped(
    ) -> Result<(), Box<dyn Error>> {
        // #576: an edit while a looping song ended over its fade_out kept it
        // current, off the list, as playing; the ending's cleanup stopped
        // the player but left it there, so Play would start it again.
        let (player, dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 1s\n- C\n").await?;
        let mut caught = false;
        for _ in 0..10 {
            let stopped = player.stop().await.map(|s| s.name().to_string());
            assert_eq!(stopped.as_deref(), Some("B"));
            assert!(player.ending_status().is_some(), "B fades out");
            // Holding the join lock, the cleanup waits on it and the edit
            // takes B as playing: the edit comes first, as on a busy machine.
            let join = player.join.lock().await;
            if join.is_none() {
                // The cleanup was quicker: B ends again for another try.
                drop(join);
                player.play().await?;
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
            assert_eq!(player.get_playlist().entry(), None, "B kept as playing");
            caught = true;
            break;
        }
        assert!(caught, "the ending's cleanup always ran before the edit");
        eventually_async(|| async { !player.is_playing().await }, "never stopped").await;
        eventually_async(
            || async { player.get_playlist().entry() == Some(1) },
            "B never gave way to C",
        )
        .await;
        assert_eq!(current_name(&player).await, "C");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn off_the_list_before_the_first_entry_previous_goes_to_it() -> Result<(), Box<dyn Error>>
    {
        // #579: the web UI took position 0 as the start of the list, and
        // turned Previous off, though Previous goes on to the entry there.
        let (player, dir) = player_looping_b_with("songs:\n- B\n- A\n- C\n").await?;
        edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
        let pl = player.get_playlist();
        assert_eq!(pl.entry(), None);
        assert_eq!(pl.position(), 0);
        assert!(pl.can_move(true));
        let prev = player.prev().await.map(|s| s.name().to_string());
        player.stop().await;
        assert_eq!(prev.as_deref(), Some("A"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stopped_song_taken_off_the_list_gives_way_during_a_status_poll(
    ) -> Result<(), Box<dyn Error>> {
        // #576: the edit took any holder of the join lock for a song
        // starting, so a status poll under way kept stopped B current, off
        // the list, and Play would start it again.
        let (player, dir) = player_looping_b().await?;
        player.stop().await;
        rests_stopped_on(&player, "B").await;
        assert_eq!(player.pending_start(), None);
        // Held as `is_playing()` holds it.
        let poll = player.join.lock().await;
        assert!(poll.is_none(), "stopped");
        edit_playlist(&player, &dir, "songs:\n- A\n- C\n");
        drop(poll);
        assert_eq!(
            player.get_playlist().entry(),
            Some(1),
            "stopped B gives way to C"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn next_past_the_end_keeps_a_paused_song_off_the_list_paused(
    ) -> Result<(), Box<dyn Error>> {
        // #576: off the list after the last entry, Next had nowhere to go
        // but cleared the pause anyway, and left B current off the list,
        // neither paused nor on an entry.
        let (player, dir) = player_looping_b().await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let paused_at = player.pause().await?.expect("paused");
        edit_playlist(&player, &dir, "songs:\n- A\n");
        assert_eq!(player.get_playlist().entry(), None);
        let next = player.next().await.map(|s| s.name().to_string());
        assert_eq!(next.as_deref(), Some("B"), "nothing to go to");
        assert_eq!(player.pending_start(), Some(paused_at), "still paused");
        assert_eq!(current_name(&player).await, "B");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_seek_racing_a_songs_own_end_moves_the_playlist_on_once() -> Result<(), Box<dyn Error>>
    {
        // No subsystems: every playback ends by itself at once, so a seek
        // straight after play races the song's own end. The end's cleanup
        // moved the playlist on and dropped the seek's restart, whose own
        // end then moved it on again.
        let mut doubled = 0;
        for _ in 0..200 {
            let player = assets_player().audio(None).ready().await?;
            let first = player.get_playlist().entry().unwrap();
            player.play().await?;
            let _ = player.seek_to(Duration::ZERO).await;
            // Let every cleanup run.
            let deadline = std::time::Instant::now() + crate::testutil::PATIENCE;
            while player.join.lock().await.is_some() && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            let at = player.get_playlist().entry().unwrap();
            if at as i64 - first as i64 != 1 {
                doubled += 1;
                eprintln!("moved {}", at as i64 - first as i64);
            }
        }
        assert_eq!(
            doubled, 0,
            "one play and one seek moved the playlist on twice {doubled}/200 times"
        );
        Ok(())
    }

    /// Plays the current song with no subsystems, so that it ends by itself
    /// at once, and returns the join lock taken before its cleanup could
    /// take it: the cleanup is left waiting on it, as behind a pause or a
    /// stop on a busy machine.
    async fn hold_join_over_a_songs_own_end(
        player: &Arc<Player>,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<PlayHandles>>, Box<dyn Error>> {
        for _ in 0..50 {
            player.play().await?;
            let join = player.join.lock().await;
            if join.is_some() {
                // The song ends, and its cleanup comes to wait on the lock.
                tokio::time::sleep(Duration::from_millis(50)).await;
                return Ok(join);
            }
            // The cleanup was quicker: back to the song to try again.
            drop(join);
            player.prev().await;
        }
        Err("the song's cleanup always took the join lock first".into())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_racing_a_songs_own_end_keeps_its_position_for_that_song(
    ) -> Result<(), Box<dyn Error>> {
        // The end's cleanup, behind the pause, moved the playlist on: the
        // next song inherited the paused position.
        let player = assets_player().audio(None).ready().await?;
        let first = player.get_playlist().entry();
        let paused_at = Duration::from_millis(100);
        {
            // As pause() does it, under the lock.
            let mut join = hold_join_over_a_songs_own_end(&player).await?;
            player.stop_for_pause(&mut join).await;
            player.transport.lock().pending_start = Some(seek::PendingStart {
                position: paused_at,
                paused: true,
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            player.get_playlist().entry(),
            first,
            "the playlist moved on"
        );
        assert_eq!(player.pending_start(), Some(paused_at));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_racing_a_songs_own_end_leaves_the_playlist_where_it_stopped(
    ) -> Result<(), Box<dyn Error>> {
        // The end's cleanup, behind the stop, moved the playlist on past the
        // song the stop stopped on.
        let player = assets_player().audio(None).ready().await?;
        let first = player.get_playlist().entry();
        {
            // As stop() does it, under the lock.
            let mut join = hold_join_over_a_songs_own_end(&player).await?;
            player.cut_locked(&mut join).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            player.get_playlist().entry(),
            first,
            "the playlist moved on"
        );
        assert!(!player.is_playing().await);
        Ok(())
    }

    /// Runs `then` while a Next (or, `back`, a Previous) on looping B breaks
    /// its loop, before the break's cleanup: both wait on the join lock held
    /// here, Next first, and the cleanup comes to it only once B, cancelled
    /// by Next, has wound down. Returns the song Next named, and `then`'s.
    async fn during_a_loop_break<T: Send + 'static>(
        player: &Arc<Player>,
        back: bool,
        then: impl FnOnce(Arc<Player>) -> tokio::task::JoinHandle<T>,
    ) -> Result<(Option<String>, T), Box<dyn Error>> {
        let held = player.join.lock().await;
        let navigating = player.clone();
        let navigation = tokio::spawn(async move {
            let target = if back {
                navigating.prev().await
            } else {
                navigating.next().await
            };
            target.map(|song| song.name().to_string())
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let then = then(player.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(held);
        let target = navigation.await?;
        Ok((target, then.await?))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_during_a_loop_break_stops_on_its_target() -> Result<(), Box<dyn Error>> {
        // #576: Next on looping B had named C and begun fading into it, and
        // a stop before the break's cleanup left the player stopped on B --
        // with a `fade_out` on B's entry, ending B over it instead.
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 2s\n- C\n").await?;
        elapsed_eventually(&player).await?;
        let (target, stopped) = during_a_loop_break(&player, false, |player| {
            tokio::spawn(async move { player.stop().await.map(|song| song.name().to_string()) })
        })
        .await?;
        assert_eq!(target.as_deref(), Some("C"));
        assert_eq!(stopped.as_deref(), Some("B"));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!player.is_playing().await, "C started after the stop");
        assert_eq!(current_name(&player).await, "C");
        assert_eq!(player.pending_start(), None);
        assert!(player.ending_status().is_none(), "B ended instead");
        // Nor is the break left for the next song played to take as its own.
        assert!(
            !player.loop_break.load(Ordering::Relaxed),
            "the break outlived the stop"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_during_a_loop_break_stops_on_its_target() -> Result<(), Box<dyn Error>> {
        // #576: Previous on looping B had named A, and a pause before the
        // break's cleanup kept B's position on B instead.
        let (player, _dir) = player_looping_b().await?;
        elapsed_eventually(&player).await?;
        let (target, paused) = during_a_loop_break(&player, true, |player| {
            tokio::spawn(async move { player.pause().await.map_err(|e| e.to_string()) })
        })
        .await?;
        assert_eq!(target.as_deref(), Some("A"));
        assert_eq!(paused, Ok(None), "nothing to resume");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!player.is_playing().await, "A started after the pause");
        assert_eq!(current_name(&player).await, "A");
        assert_eq!(player.pending_start(), None, "a position kept for A");
        assert!(!player.loop_break.load(Ordering::Relaxed));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_seek_during_a_loop_break_calls_it_off_and_seeks_the_looping_song(
    ) -> Result<(), Box<dyn Error>> {
        // A seek is asked of the song playing: it restarts B there, and the
        // break is called off rather than moving on once B is restarted.
        let (player, _dir) = player_looping_b().await?;
        elapsed_eventually(&player).await?;
        let (target, seeked) = during_a_loop_break(&player, false, |player| {
            tokio::spawn(async move {
                player
                    .seek_to(Duration::from_secs(5))
                    .await
                    .map_err(|e| e.to_string())
            })
        })
        .await?;
        assert_eq!(target.as_deref(), Some("C"));
        assert_eq!(seeked, Ok(()));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(player.is_playing().await);
        assert_eq!(current_name(&player).await, "B");
        let elapsed = player.elapsed().await?.expect("playing");
        assert!(elapsed >= Duration::from_secs(5), "{elapsed:?}");
        assert!(!player.loop_break.load(Ordering::Relaxed));
        player.stop().await;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_after_a_loop_breaks_cleanup_keeps_the_next_song_from_starting(
    ) -> Result<(), Box<dyn Error>> {
        // Next breaks looping B's loop, and its cleanup moves on to C and
        // starts it. It let go of the join lock in between, and a stop that
        // took the lock there found nothing playing to stop: C started
        // after it regardless.
        let (player, _dir) = player_looping_b().await?;
        elapsed_eventually(&player).await?;
        let b = player.join.lock().await.as_ref().unwrap().cancel.clone();
        player.next().await;
        let deadline = std::time::Instant::now() + crate::testutil::PATIENCE;
        let mut stopped = false;
        while std::time::Instant::now() < deadline {
            let mut join = player.join.lock().await;
            // The first chance after the cleanup has had the lock.
            if !join
                .as_ref()
                .is_some_and(|handles| handles.cancel.same_as(&b))
            {
                player.cut_locked(&mut join).await;
                stopped = true;
                break;
            }
            drop(join);
            tokio::task::yield_now().await;
        }
        assert!(stopped, "the break's cleanup never ran");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !player.is_playing().await,
            "C started after the stop: {}",
            current_name(&player).await
        );
        assert_eq!(current_name(&player).await, "C");
        Ok(())
    }

    /// A pause queued behind a loop break's cleanup, which moves on to the
    /// next song and starts it, pauses that song at its own position, not
    /// the one before's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_queued_behind_a_loop_breaks_cleanup_keeps_no_position_of_the_song_before(
    ) -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b().await?;
        elapsed_eventually(&player).await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let pause = {
            let join = player.join.lock().await;
            // Next's break, as navigate asks it.
            player.set_loop_exit(LoopExit::Next);
            join.as_ref().unwrap().cancel.cancel();
            drop(join);
            // B winds down, so its cleanup queues on the lock first...
            let held = player.join.lock().await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            // ...then the pause queues after it.
            let pausing = player.clone();
            let pause =
                tokio::spawn(async move { pausing.pause().await.map_err(|e| e.to_string()) });
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(held);
            pause
        };
        let paused = pause.await?;
        assert_eq!(current_name(&player).await, "C");
        assert!(!player.is_playing().await);
        let kept = player.pending_start();
        assert!(
            kept.is_none_or(|k| k < Duration::from_millis(500)),
            "C kept B's position: pause returned {paused:?}, pending {kept:?}"
        );
        Ok(())
    }

    /// A pause before `play_files` has stamped the start time kept 0, so a
    /// resume restarted the song: until the stamp, the song is where it
    /// started (#576).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_before_the_start_is_stamped_keeps_the_start_position(
    ) -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b().await?;
        player.stop().await;
        eventually(
            || player.join.try_lock().is_ok_and(|j| j.is_none()),
            "stopped",
        );
        // Resume at 5 s (as after a pause), then pause again before
        // play_files has stamped play_start_time with the running clock.
        // Simulated by putting it back to starting under the join lock once
        // the start has gone in.
        {
            let mut join = player.join.lock().await;
            player
                .play_from_locked(Duration::from_secs(5), &mut join)
                .await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            *player.play_start_time.lock().await =
                Some(PlayPosition::Starting(Duration::from_secs(5)));
        }
        let paused = player.pause().await?;
        assert_eq!(
            paused,
            Some(Duration::from_secs(5)),
            "a pause before the stamp kept {paused:?}, pending {:?}",
            player.pending_start()
        );
        Ok(())
    }

    /// A pause straight after a seek lands in that window as a matter of
    /// course: it keeps the seek's position.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pause_straight_after_a_seek_keeps_the_seek() -> Result<(), Box<dyn Error>> {
        let (player, _dir) = player_looping_b().await?;
        elapsed_eventually(&player).await?;
        let mut zeros = 0;
        for _ in 0..10 {
            player.seek_to(Duration::from_secs(5)).await?;
            let paused = player.pause().await?;
            if paused == Some(Duration::ZERO) {
                zeros += 1;
            }
            player.play().await?;
            elapsed_eventually(&player).await?;
        }
        assert_eq!(
            zeros, 0,
            "{zeros}/10 pauses straight after a seek to 5 s kept 0"
        );
        Ok(())
    }

    /// A reload deferred under a looping song ran as the song's ending
    /// began, and tore the audio down under its fade-out. It waits for the
    /// fade, and runs once it is over.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_waits_for_a_looping_songs_ending_fade() -> Result<(), Box<dyn Error>>
    {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 5s\n- C\n").await?;
        let _store = set_store_without_audio(&player)?;
        player.reload_hardware().await?;
        assert!(player.audio_device().is_some(), "deferred");
        // Stop ends B over its 5 s fade-out.
        player.stop().await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(player.ending_status().is_some(), "still fading out");
        assert!(
            player.audio_device().is_some(),
            "the deferred reload tore the audio device down under the ending fade"
        );
        eventually(
            || player.ending_status().is_none() && player.audio_device().is_none(),
            "The deferred reload never ran after the fade",
        );
        Ok(())
    }

    /// A loop break's fade can outlast the song it moved to, which leaves
    /// the player idle with nothing to come along and run a reload waiting
    /// for the fade: it runs once the fade is over.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_waits_for_a_loop_breaks_fade() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _store = set_store_without_audio(&player)?;
        player.transport.lock().fading_until =
            Some(std::time::Instant::now() + Duration::from_millis(800));
        player.reload_hardware().await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            player.audio_device().is_some(),
            "the reload tore the audio device down under the fade"
        );
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran after the fade",
        );
        Ok(())
    }

    /// Next during an ending forgets it, the fade left to play out: a
    /// reload waiting for the ending runs once the fade is over, though
    /// the ending will not say so.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_runs_after_an_ending_navigated_away_from(
    ) -> Result<(), Box<dyn Error>> {
        let (player, _dir) =
            player_looping_b_with("songs:\n- A\n- song: B\n  fade_out: 1s\n- C\n").await?;
        let _store = set_store_without_audio(&player)?;
        player.reload_hardware().await?;
        player.stop().await;
        eventually(|| player.ending_status().is_some(), "B never began to end");
        eventually(
            || player.join.try_lock().is_ok_and(|j| j.is_none()),
            "stopped",
        );
        player.next().await;
        assert!(player.ending_status().is_none(), "the ending is forgotten");
        assert!(player.audio_device().is_some(), "reloaded under the fade");
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran after the fade",
        );
        Ok(())
    }

    /// A pause ran a reload deferred under the song, and a resume was then
    /// refused while the hardware came back up, for good when the new
    /// config's audio interface was slow or not there. A paused song is a
    /// show in progress: the reload waits on, and the song resumes at once.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_deferred_reload_waits_out_a_pause() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // A saved config whose audio interface is slow or not there yet.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "songs: songs\nprofiles:\n  - audio:\n      device: no-such-device\n      track_mappings:\n        click: [1]\n",
        )?;
        let cfg = config::Player::deserialize(&path)?;
        player.set_config_store(std::sync::Arc::new(config::ConfigStore::new(cfg, path)));

        player.play().await?;
        elapsed_eventually(&player).await?;
        player.reload_hardware().await?; // deferred under the song
        let paused = player.pause().await?;
        assert!(paused.is_some());
        tokio::time::sleep(Duration::from_millis(500)).await;
        let resumed = player.play().await;
        player.init_cancel.lock().cancel();
        assert!(
            matches!(resumed, Ok(Some(_))),
            "resume after a pause refused: {:?}",
            resumed
                .map(|s| s.map(|s| s.name().to_string()))
                .map_err(|e| e.to_string())
        );
        Ok(())
    }

    /// A reload asked for while a song is paused waits as under a playing
    /// one, and runs once the paused song is stopped.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_while_paused_runs_when_the_song_stops() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _store = set_store_without_audio(&player)?;
        player.play().await?;
        elapsed_eventually(&player).await?;
        assert!(player.pause().await?.is_some());
        player.reload_hardware().await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(player.audio_device().is_some(), "reloaded under the pause");
        assert!(*player.init_done_tx.borrow(), "reset under the pause");

        player.stop().await;
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        Ok(())
    }

    /// Moving off a paused song calls the pause off: a reload waiting for
    /// it runs then.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_while_paused_runs_when_the_playlist_moves_on() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _store = set_store_without_audio(&player)?;
        player.play().await?;
        elapsed_eventually(&player).await?;
        assert!(player.pause().await?.is_some());
        player.reload_hardware().await?;
        assert!(player.audio_device().is_some(), "reloaded under the pause");

        player.next().await;
        assert!(player.pending_start().is_none(), "still paused");
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        Ok(())
    }

    /// A position cued by a seek while stopped held a reload off as a pause
    /// does, though nothing plays and no lights are held. Only a pause does;
    /// a seek while paused leaves the song paused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_seek_while_stopped_does_not_hold_a_reload() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _store = set_store_without_audio(&player)?;
        player.play().await?;
        elapsed_eventually(&player).await?;
        assert!(player.pause().await?.is_some());
        player.seek_to(Duration::from_millis(150)).await?;
        assert!(
            player.is_paused(),
            "a seek while paused called the pause off"
        );
        assert!(!player.reload_hardware().await?.applied());
        player.stop().await;
        assert!(!player.is_paused());
        eventually(
            || player.audio_device().is_none(),
            "The deferred reload never ran",
        );
        player.await_hardware_ready().await;

        player.seek_to(Duration::from_millis(150)).await?;
        assert_eq!(player.pending_start(), Some(Duration::from_millis(150)));
        assert!(!player.is_paused(), "a stopped seek reads as paused");
        assert!(player.reload_hardware().await?.applied());
        Ok(())
    }

    /// A resume took the paused position before the join lock, and a reload
    /// that took the lock in between found the player idle: it reset the
    /// hardware, and the resume was refused with its position thrown away.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_resume_racing_a_reload_keeps_the_pause() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let _store = set_store_without_audio(&player)?;
        player.play().await?;
        elapsed_eventually(&player).await?;
        let paused = player.pause().await?;
        assert!(paused.is_some());

        // The reload and the resume both wait on the join lock, the reload
        // first.
        let join = player.join.lock().await;
        let p = player.clone();
        let reload =
            tokio::spawn(async move { p.reload_hardware().await.map_err(|e| e.to_string()) });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let p = player.clone();
        let resume = tokio::spawn(async move {
            p.play()
                .await
                .map(|s| s.map(|s| s.name().to_string()))
                .map_err(|e| e.to_string())
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(join);

        let reload = reload.await?;
        let resume = resume.await?;
        assert!(
            matches!(resume, Ok(Some(_))),
            "resume refused: {resume:?}; reload: {reload:?}"
        );
        assert!(player.audio_device().is_some(), "reloaded under the pause");
        player.stop().await;
        Ok(())
    }

    /// A MIDI controller listening for the pedalboard's footswitches.
    fn midi_controller() -> config::Controller {
        config::Controller::Midi(config::MidiController::new(
            config::midi::note_on(16, 0, 127),
            config::midi::note_on(16, 1, 127),
            config::midi::note_on(16, 2, 127),
            config::midi::note_on(16, 3, 127),
            config::midi::note_on(16, 4, 127),
            config::midi::note_on(16, 5, 127),
        ))
    }

    /// A player with a MIDI controller on the given MIDI device.
    fn midi_controller_config(midi: &str) -> config::Player {
        config::Player::new(
            vec![midi_controller()],
            Some(config::Audio::new("mock-device")),
            Some(config::Midi::new(midi, None)),
            None,
            HashMap::new(),
            "assets/songs",
        )
    }

    /// A player whose MIDI controller has started on the given MIDI device.
    async fn make_midi_controller_player(midi: &str) -> Result<Arc<Player>, Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let mut playlists = HashMap::new();
        playlists.insert("all_songs".to_string(), playlist::from_songs(songs)?);
        let player = Player::new(
            playlists,
            "all_songs".to_string(),
            &midi_controller_config(midi),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok(player)
    }

    /// Presses Next on the player's MIDI device, and waits for the playlist
    /// to move on. A mock event waits for a watcher to take it, forever if
    /// there is none, so it is sent from a thread of its own.
    fn press_next_on_midi_device(player: &Arc<Player>) -> Result<(), Box<dyn Error>> {
        use crate::config::midi::ToMidiEvent;

        let midi = player.midi_device().expect("midi device").to_mock()?;
        let before = player.get_playlist().current().unwrap().name().to_string();
        let mut next = Vec::new();
        config::midi::note_on(16, 2, 127)
            .to_midi_event()?
            .write(&mut next)?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            midi.mock_event(&next);
            let _ = tx.send(());
        });
        // As long as `eventually` below allows: a restarted controller can
        // take a while to start watching on a loaded runner.
        assert!(
            rx.recv_timeout(crate::testutil::PATIENCE).is_ok(),
            "nothing watches the player's MIDI device"
        );
        eventually(
            || player.get_playlist().current().unwrap().name() != before,
            "Next on the MIDI device was ignored",
        );
        Ok(())
    }

    /// A hardware reload opens the MIDI device anew, and an unchanged MIDI
    /// controller was kept listening to the old one, as gRPC and MCP
    /// reload: a pedalboard moved to another interface went unheard.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unchanged_midi_controller_follows_a_reloaded_midi_device(
    ) -> Result<(), Box<dyn Error>> {
        let player = make_midi_controller_player("mock-midi-a").await?;
        assert_eq!(player.controller_statuses()[0].status, "running");
        // The pedalboard moves to another MIDI interface; the controller is
        // unchanged.
        let dir = tempfile::tempdir()?;
        player.set_config_store(Arc::new(config::ConfigStore::new(
            midi_controller_config("mock-midi-b"),
            dir.path().join("config.yaml"),
        )));
        assert!(player.reload_hardware().await?.applied());
        player.await_hardware_ready().await;
        assert_eq!(player.controller_statuses()[0].status, "running");
        assert!(player
            .midi_device()
            .is_some_and(|d| d.to_string().contains("mock-midi-b")));
        press_next_on_midi_device(&player)
    }

    /// A restarted MIDI controller watches the device its predecessor did.
    /// The predecessor was aborted with its watch still on, the device
    /// refused a second, and the restarted controller heard nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_midi_controller_watches_its_device_again() -> Result<(), Box<dyn Error>> {
        let player = make_midi_controller_player("mock-midi-device").await?;
        player.restart_controllers_forced(vec![midi_controller()]);
        assert_eq!(player.controller_statuses()[0].status, "running");
        press_next_on_midi_device(&player)
    }
}
