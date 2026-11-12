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

//! The websocket's messages (`/ws`): the playback status, log lines,
//! waveforms and reload notices. (The stage is not here: it is the gRPC
//! stream `WatchStage`; its types below are what the web UI reads it into.)

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::audio::beat_grid::BeatGrid;
use crate::config::{FadeStatus, Section};
use crate::dmx::engine::AmbientMode;
use crate::lighting::geometry::Vec3;
use crate::playlist::AmbientSource;
use crate::state::PoseSnapshot;

/// A websocket message, by its `type`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SocketMessage {
    Playback(Box<Playback>),
    Logs {
        lines: Vec<LogLine>,
    },
    Waveform(Waveform),
    /// The config store changed: re-fetch what is shown of it.
    ConfigChanged,
    /// A song's light show was reloaded from disk, or failed to.
    Reload(ShowReload),
}

/// What the player is doing, about five times a second.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Playback {
    pub is_playing: bool,
    pub elapsed_ms: u64,
    pub song_name: String,
    pub song_duration_ms: u64,
    pub playlist_name: String,
    pub playlist_position: usize,
    /// A song an edit took off the list, playing on: the playlist sits
    /// before the entry at `playlist_position` rather than on it.
    pub playlist_off_list: bool,
    /// Whether Previous has no entry to move back to, and Next none to move
    /// on to (`Playlist::can_move`).
    pub playlist_at_first: bool,
    pub playlist_at_last: bool,
    pub playlist_songs: Vec<String>,
    pub tracks: Vec<TrackInfo>,
    pub available_playlists: Vec<String>,
    pub persisted_playlist_name: String,
    pub locked: bool,
    /// The last deferred hardware reload generation started (or refused):
    /// a config save's deferral is over once this reaches the generation
    /// its reply named. What each device then does is the hardware
    /// status's to say.
    pub hardware_reload_started: u64,
    pub hardware_reload_refused: Vec<RefusedReload>,
    /// Which process the generations are this one's: a restart counts from
    /// none again. A string, as it is more than a JSON number holds
    /// exactly.
    pub process_id: String,
    pub beat_grid: Option<PlaybackGrid>,
    pub available_sections: Vec<Section>,
    pub active_section: Option<ActiveSection>,
    pub looping: bool,
    pub reactive_loop_state: LoopState,
    /// Start position for the next play, set by seeking while stopped.
    pub pending_start_ms: Option<u64>,
    pub pilot_hints: Vec<ResolvedPilotHint>,
    /// The tempo and metre from the song's tempo map, at the playhead.
    pub tempo: Option<PlaybackTempo>,
    /// A song about to start by itself, its playlist entry's auto-play gap
    /// running.
    pub auto_play: Option<AutoPlay>,
    /// A looping song fading out after a stop, with nothing after it; a
    /// second stop cuts it.
    pub ending: Option<Ending>,
    /// The show the rig plays while no song is: present with a DMX engine,
    /// whatever the mode, so the card can show it, or that the read did not
    /// answer; `null` without one.
    pub ambient: Option<AmbientState>,
    /// What comes after the current song and how it arrives; `null` with no
    /// current song.
    pub next: Option<NextUp>,
}

/// An output track, with its mixer settings.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub name: String,
    pub output_channels: Vec<u16>,
    /// Output gain in dB, widened from the mixer's f32 as the JSON has
    /// always carried it.
    pub gain_db: f64,
    /// Whether the track is muted (independently of the gain).
    pub muted: bool,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RefusedReload {
    pub generation: u64,
    pub error: String,
}

/// The player's beat grid, with the accent levels the metronome plays.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlaybackGrid {
    #[serde(flatten)]
    pub grid: BeatGrid,
    /// Per-beat accent levels (0 silent, 1 normal, 2 half, 3 accent):
    /// what the audio metronome plays, for the visual one. Present when the
    /// song has a metronome.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_levels: Option<Vec<u8>>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ActiveSection {
    pub name: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The reactive section loop's state, with the section it is about.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LoopState {
    Idle,
    SectionOffered { section_name: String },
    LoopArmed { section_name: String },
    Looping { section_name: String },
    BreakRequested { section_name: String },
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ResolvedPilotHint {
    pub label: String,
    pub at_ms: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub has_audio: bool,
}

/// A tempo and metre: at the playhead, or a tempo block's defaults.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(into_wasm_abi))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlaybackTempo {
    pub bpm: f64,
    /// `[numerator, denominator]`.
    pub time_signature: (u32, u32),
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AutoPlay {
    pub song: String,
    pub starts_in_ms: u64,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Ending {
    pub song: String,
    pub remaining_ms: u64,
}

/// Ambient light as the status reports it, on every surface: the web UI's
/// playback message and MCP's `status` serialize it, gRPC's `Status` maps
/// it. Built by `DmxRead::<AmbientStatus>::view` alone.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum AmbientState {
    /// The status read the DMX engine.
    Read(AmbientReading),
    /// The status's read of the DMX engine did not answer in time, or
    /// failed: not the same as no DMX engine (no ambient at all).
    Unread(AmbientUnread),
}

/// Ambient light whose status read did not answer.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AmbientUnread {
    /// Always `false`.
    pub available: bool,
    /// Why there is nothing to report.
    pub reason: String,
}

/// Ambient light: its mode, its show and what it is doing.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AmbientReading {
    pub mode: AmbientMode,
    /// Whose settings are up: the profile's, the playlist's, or the
    /// entry's (the song about to play).
    pub source: Option<AmbientSource>,
    /// The show, `null` for none.
    pub show: Option<String>,
    /// `playing`, `pending` (its delay running), `off`, or `unavailable`
    /// (on, with no show to play).
    pub state: String,
}

/// The song after the current one. A looping song that is playing goes on
/// only by a loop break, with the playlist's fades; otherwise the next song
/// starts by itself after `auto_play_ms` (exact, not rounded: the web UI
/// writes it as the entry's gap is written, `gap_text`), or (`null`) waits
/// for Play. The
/// fields a case does not decide are absent.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct NextUp {
    /// `null` at the end of the playlist.
    pub song: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breaks_loop: Option<bool>,
    /// At the end of the playlist, a looping song playing: Next (or
    /// Previous on the first entry) ends it over `fade_out` rather than
    /// moving on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_loop: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<Option<FadeStatus>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<Option<FadeStatus>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_play_ms: Option<Option<f64>>,
}

/// The stage at one instant, by fixture: what the live `state` message and
/// a preview's evaluation both carry, so the stage views are fed the same
/// shape either way.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct StageValues {
    pub fixtures: BTreeMap<String, HashMap<String, u8>>,
    /// Where each mover points.
    pub poses: BTreeMap<String, PoseSnapshot>,
    /// Per-cell bytes by fixture then cell, only for fixtures a per-cell
    /// effect is driving (design §17.4); a fixture absent here shows its
    /// own channels on every cell.
    pub cells: BTreeMap<String, BTreeMap<String, HashMap<String, u8>>>,
    /// The rate each strobing fixture flashes at, in hertz: a map of its
    /// own, not a channel, so the byte maps stay bytes. A fixture that is
    /// not strobing is absent.
    pub strobe_rates: BTreeMap<String, f64>,
}

/// The stage now: what a frame of the gRPC stage stream says, read against
/// its layout.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StageState {
    #[serde(flatten)]
    pub frame: StageValues,
    pub active_effects: Vec<String>,
    /// Venue fixtures a fixture test's output covers: their channels,
    /// cells and poses are the test's bytes, what leaves for olad.
    pub under_test: Vec<String>,
}

/// What is on the stage: the venue's fixtures as the stage views need them,
/// which is what a layout of the gRPC stage stream says.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StageMetadata {
    pub fixtures: BTreeMap<String, FixtureMetadata>,
    pub venue: Option<StageVenue>,
}

/// A fixture as the stage views draw it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureMetadata {
    #[serde(flatten)]
    pub placement: FixturePlacement,
    /// What the fixture can do (`color`, `pan_tilt`, `strobe`, `cells`),
    /// named as the fit endpoint names them: a fixture with `pan_tilt` is
    /// a mover, whatever its type's channels look like.
    pub capabilities: Vec<crate::lighting::fit::Capability>,
    /// The cells of a pixel fixture, in the manufacturer's order: the plot
    /// draws one segment each.
    pub cells: Vec<CellPlacement>,
}

/// Where and how a fixture hangs, as the venue says.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixturePlacement {
    pub tags: Vec<String>,
    #[serde(rename = "type")]
    pub fixture_type: String,
    /// Stage position in meters, when the venue places the fixture; the
    /// stage view draws a plot from it and falls back to the tag layout
    /// without.
    pub position: Option<Vec3>,
    /// Mounting rotation in degrees about X, Y, Z.
    pub rotation: Option<Vec3>,
    /// Degrees the venue says this unit really throws, or `null` for its
    /// type's own; a drawing hint only.
    pub beam_angle: Option<f64>,
    /// The rig model's path under /api/lighting/assets/, when the type has
    /// one; the 3D view draws a generic body without.
    pub rig: Option<String>,
}

/// A cell of a pixel fixture, with its offset in the fixture's frame
/// (meters).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CellPlacement {
    pub name: String,
    pub offset: Vec3,
}

/// The current venue as the stage views need it: its points and scenery.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StageVenue {
    pub name: String,
    pub focus_points: BTreeMap<String, Vec3>,
    /// The scenery file under /api/lighting/assets/, when the venue came
    /// from an MVR; the 3D view draws it.
    pub scenery: Option<String>,
    /// Why the scenery is missing, when the venue should have had some.
    pub scenery_error: Option<String>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LogLine {
    pub level: String,
    pub target: String,
    pub message: String,
}

/// The current song's waveform peaks, by track.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Waveform {
    pub song_name: String,
    pub tracks: Vec<TrackPeaks>,
}

impl Waveform {
    /// A song's waveform from its tracks' peaks, widened from f32 as the
    /// JSON has always carried them.
    pub fn new(song_name: String, peaks: impl IntoIterator<Item = (String, Vec<f32>)>) -> Self {
        Waveform {
            song_name,
            tracks: peaks
                .into_iter()
                .map(|(name, peaks)| TrackPeaks {
                    name,
                    peaks: peaks.into_iter().map(f64::from).collect(),
                })
                .collect(),
        }
    }
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TrackPeaks {
    pub name: String,
    pub peaks: Vec<f64>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShowReload {
    Ok,
    Error { error: String },
}
