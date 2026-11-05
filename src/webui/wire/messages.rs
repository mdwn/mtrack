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

//! The websocket's messages (`/ws`): the playback status, the stage's
//! state and metadata, log lines, waveforms and reload notices.

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
    State(StageState),
    Metadata(StageMetadata),
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
    /// The recent deferred hardware reloads refused, with why.
    pub hardware_reload_refused: Vec<RefusedReload>,
    /// Which process the generations are this one's: a restart counts from
    /// none again. A string, as it is more than a JSON number holds
    /// exactly.
    pub process_id: String,
    pub beat_grid: Option<PlaybackGrid>,
    pub available_sections: Vec<Section>,
    pub active_section: Option<ActiveSection>,
    pub looping: bool,
    /// The reactive section-loop state machine.
    pub reactive_loop_state: LoopState,
    /// Start position for the next play, set by seeking while stopped.
    pub pending_start_ms: Option<u64>,
    /// Pilot hints resolved to song times.
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
    /// whatever the mode, so the card can show it; `null` without one, or
    /// when the read did not answer.
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
    /// Output gain in dB.
    pub gain_db: f32,
    /// Whether the track is muted (independently of the gain).
    pub muted: bool,
}

/// A deferred hardware reload refused, and why.
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

/// The section loop in force, in song time.
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

/// A pilot hint resolved to song times.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ResolvedPilotHint {
    pub label: String,
    pub at_ms: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub has_audio: bool,
}

/// The tempo and metre at the playhead.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlaybackTempo {
    pub bpm: f64,
    /// `[numerator, denominator]`.
    pub time_signature: (u32, u32),
}

/// A song about to start by itself.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AutoPlay {
    pub song: String,
    pub starts_in_ms: u64,
}

/// A looping song fading out after a stop.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Ending {
    pub song: String,
    pub remaining_ms: u64,
}

/// Ambient light: its mode, its show and what it is doing.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AmbientState {
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
/// starts by itself after `auto_play_ms`, or (`null`) waits for Play. The
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
    pub auto_play_ms: Option<Option<u64>>,
}

/// The stage at one instant, by fixture: what the live `state` message and
/// a preview's evaluation both carry, so the stage views are fed the same
/// shape either way.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct StageValues {
    /// Channel bytes by fixture.
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

/// The `state` message: the stage now.
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

/// The `metadata` message: the venue's fixtures as the stage views need
/// them, sent to each client on connect.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StageMetadata {
    pub fixtures: BTreeMap<String, FixtureMetadata>,
    pub venue: Option<VenueMetadata>,
}

/// A fixture as the stage views draw it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureMetadata {
    pub tags: Vec<String>,
    #[serde(rename = "type")]
    pub fixture_type: String,
    /// Stage position in meters, when the venue places the fixture; the
    /// stage view draws a plot from it and falls back to the tag layout
    /// without.
    pub position: Option<Vec3>,
    /// Mounting rotation in degrees about X, Y, Z, when the venue states
    /// it.
    pub rotation: Option<Vec3>,
    /// Degrees the venue says this unit really throws, or `null` for its
    /// type's own; a drawing hint only.
    pub beam_angle: Option<f64>,
    /// The rig model's path under /api/lighting/assets/, when the type has
    /// one; the 3D view draws a generic body without.
    pub rig: Option<String>,
    /// What the fixture can do (`color`, `pan_tilt`, `strobe`, `cells`),
    /// named as the fit endpoint names them: a fixture with `pan_tilt` is
    /// a mover, whatever its type's channels look like.
    pub capabilities: Vec<String>,
    /// The cells of a pixel fixture, in the manufacturer's order: the plot
    /// draws one segment each.
    pub cells: Vec<CellPlacement>,
}

/// A cell of a pixel fixture, with its offset in the fixture's frame
/// (meters).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CellPlacement {
    pub name: String,
    pub offset: Vec3,
}

/// The current venue as the stage views need it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VenueMetadata {
    pub name: String,
    /// The configured venues directory, for saving edits back.
    pub dir: Option<String>,
    pub focus_points: BTreeMap<String, Vec3>,
    /// The scenery file under /api/lighting/assets/, when the venue came
    /// from an MVR; the 3D view draws it.
    pub scenery: Option<String>,
    /// Why the scenery is missing, when the venue should have had some.
    pub scenery_error: Option<String>,
}

/// A line of the player's log.
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

/// A track's waveform peaks.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TrackPeaks {
    pub name: String,
    pub peaks: Vec<f32>,
}

/// A light show reload's outcome.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShowReload {
    Ok,
    Error { error: String },
}
