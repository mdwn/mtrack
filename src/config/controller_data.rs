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

//! A profile's `controllers:` as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `controller`'s.

use super::midi_event::MidiEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_OSC_PORT: u16 = 43235;

pub const DEFAULT_MCP_PORT: u16 = 43237;

pub const DEFAULT_MCP_BIND_ADDRESS: &str = "127.0.0.1";

pub(super) fn default_osc_play() -> String {
    "/mtrack/play".to_string()
}

pub(super) fn default_osc_pause() -> String {
    "/mtrack/pause".to_string()
}

pub(super) fn default_osc_prev() -> String {
    "/mtrack/prev".to_string()
}

pub(super) fn default_osc_next() -> String {
    "/mtrack/next".to_string()
}

pub(super) fn default_osc_stop() -> String {
    "/mtrack/stop".to_string()
}

pub(super) fn default_osc_all_songs() -> String {
    "/mtrack/all_songs".to_string()
}

pub(super) fn default_osc_playlist() -> String {
    "/mtrack/playlist".to_string()
}

pub(super) fn default_osc_stop_samples() -> String {
    "/mtrack/samples/stop".to_string()
}

pub(super) fn default_osc_section_ack() -> String {
    "/mtrack/section_ack".to_string()
}

pub(super) fn default_osc_stop_section_loop() -> String {
    "/mtrack/stop_section_loop".to_string()
}

pub(super) fn default_osc_loop_section() -> String {
    "/mtrack/loop_section".to_string()
}

pub(super) fn default_osc_ambient_on() -> String {
    "/mtrack/ambient/on".to_string()
}

pub(super) fn default_osc_ambient_off() -> String {
    "/mtrack/ambient/off".to_string()
}

pub(super) fn default_osc_ambient_auto() -> String {
    "/mtrack/ambient/auto".to_string()
}

pub(super) fn default_osc_cancel_auto_play() -> String {
    "/mtrack/auto_play/cancel".to_string()
}

pub(super) fn default_osc_seek() -> String {
    "/mtrack/seek".to_string()
}

pub(super) fn default_osc_seek_section() -> String {
    "/mtrack/seek_section".to_string()
}

pub(super) fn default_osc_track_gain() -> String {
    "/mtrack/track/*/gain".to_string()
}

pub(super) fn default_osc_status() -> String {
    "/mtrack/status".to_string()
}

pub(super) fn default_osc_audio_health() -> String {
    "/mtrack/audio/health".to_string()
}

pub(super) fn default_osc_playlist_current() -> String {
    "/mtrack/playlist/current".to_string()
}

pub(super) fn default_osc_playlist_current_song() -> String {
    "/mtrack/playlist/current_song".to_string()
}

pub(super) fn default_osc_playlist_current_song_elapsed() -> String {
    "/mtrack/playlist/current_song/elapsed".to_string()
}

pub(super) fn default_osc_timeline() -> String {
    "/mtrack/timeline".to_string()
}

/// Allows users to specify various controllers.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Controller {
    Grpc(GrpcController),
    Mcp(McpController),
    Midi(MidiController),
    Multi(HashMap<String, Controller>),
    Osc(Box<OscController>),
}

/// The configuration that maps MIDI events to controller messages.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct MidiController {
    /// The MIDI event to look for to play the current song in the playlist.
    pub(super) play: MidiEvent,
    /// The MIDI event to look for to move the playlist to the previous item.
    pub(super) prev: MidiEvent,
    /// The MIDI event to look for to move the playlist to the next item.
    pub(super) next: MidiEvent,
    /// The MIDI event to look for to stop playback.
    pub(super) stop: MidiEvent,
    /// The MIDI event to look for to switch from the current playlist to an all songs playlist.
    pub(super) all_songs: MidiEvent,
    /// The MIDI event to look for to switch back to the current playlist.
    pub(super) playlist: MidiEvent,
    /// The MIDI event to acknowledge the current section (arm section loop).
    #[serde(default)]
    pub(super) section_ack: Option<MidiEvent>,
    /// The MIDI event to break out of the current section loop.
    #[serde(default)]
    pub(super) stop_section_loop: Option<MidiEvent>,
    /// The MIDI event to hold ambient light up (ambient mode `on`).
    #[serde(default)]
    pub(super) ambient_on: Option<MidiEvent>,
    /// The MIDI event to turn ambient light off (ambient mode `off`).
    #[serde(default)]
    pub(super) ambient_off: Option<MidiEvent>,
    /// The MIDI event to give ambient light back to the player (ambient
    /// mode `auto`).
    #[serde(default)]
    pub(super) ambient_auto: Option<MidiEvent>,
    /// The MIDI event to call off an auto play waiting to start.
    #[serde(default)]
    pub(super) cancel_auto_play: Option<MidiEvent>,
    /// Optional Morningstar controller integration for automatic preset naming.
    #[serde(default)]
    pub(super) morningstar: Option<MorningstarConfig>,
}

/// The configuration for the multitrack player gRPC server.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct GrpcController {
    /// The port to listen on.
    #[serde(default)]
    pub(super) port: Option<u16>,
}

pub(super) fn default_mcp_bind_address() -> String {
    DEFAULT_MCP_BIND_ADDRESS.to_string()
}

/// The configuration for the mtrack MCP (Model Context Protocol) server.
///
pub const DEFAULT_MCP_IDLE_TIMEOUT_SECS: u64 = 14400;

pub(super) fn default_mcp_idle_timeout_secs() -> Option<u64> {
    Some(DEFAULT_MCP_IDLE_TIMEOUT_SECS)
}

/// Exposes mtrack to MCP-compatible clients (such as Claude Desktop or Claude
/// Code) over a streamable HTTP transport. The server is bound to localhost
/// by default — set `bind_address: "0.0.0.0"` to expose it on the network.
/// When binding beyond localhost you should also set `bearer_token` so the
/// MCP surface is not exposed unauthenticated.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Deserialize, Serialize)]
pub struct McpController {
    /// The port to listen on. Defaults to `DEFAULT_MCP_PORT`.
    #[serde(default)]
    pub(super) port: Option<u16>,
    /// The bind address. Defaults to localhost-only.
    #[serde(default = "default_mcp_bind_address")]
    pub(super) bind_address: String,
    /// Optional bearer token. When set, every request must carry an
    /// `Authorization: Bearer <token>` header; mismatched or missing tokens
    /// return HTTP 401. Recommended whenever `bind_address` is not localhost.
    #[serde(default)]
    pub(super) bearer_token: Option<String>,
    /// Idle timeout in seconds. A session whose inbound *and* outbound traffic
    /// is quiet for this long is closed and its server-side resources released.
    /// Defaults to 14400 (4 hours). Set to `null` to disable eviction.
    ///
    /// Server-pushed SSE notifications count as activity (see the response-body
    /// wrapper in `controller/mcp.rs`), so a passive subscriber that's actively
    /// receiving pushes stays alive without needing to send keepalive pings.
    #[serde(default = "default_mcp_idle_timeout_secs")]
    pub(super) idle_session_timeout_secs: Option<u64>,
    /// `Host` header values the server accepts, as DNS-rebinding protection.
    /// Each entry is a host or `host:port` (`"mtrack.local"`,
    /// `"192.168.1.20:43237"`). An empty list accepts any `Host`.
    ///
    /// When unset, a loopback `bind_address` accepts only `localhost`,
    /// `127.0.0.1` and `::1`, and any other `bind_address` accepts every
    /// `Host` — a server deliberately exposed on the network has to answer
    /// to whatever name or address clients reach it by, and `bearer_token`
    /// is what keeps a rebinding attacker's browser out.
    #[serde(default)]
    pub(super) allowed_hosts: Option<Vec<String>>,
}

/// The configuration for the multitrack player OSC server.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Deserialize, Serialize)]
pub struct OscController {
    /// The port to listen on.
    #[serde(default = "default_osc_port")]
    pub(super) port: u16,
    /// The broadcast addresses including the port.
    #[serde(default)]
    pub(super) broadcast_addresses: Vec<String>,
    /// The OSC address to look for to play the current song in the playlist.
    #[serde(default = "default_osc_play")]
    pub(super) play: String,
    /// The OSC address to pause playback while preserving the current position.
    #[serde(default = "default_osc_pause")]
    pub(super) pause: String,
    /// The OSC address to look for to move the playlist to the previous item.
    #[serde(default = "default_osc_prev")]
    pub(super) prev: String,
    /// The OSC address to look for to move the playlist to the next item.
    #[serde(default = "default_osc_next")]
    pub(super) next: String,
    /// The OSC address to look for to stop playback.
    #[serde(default = "default_osc_stop")]
    pub(super) stop: String,
    /// The OSC address to look for to switch from the current playlist to an all songs playlist.
    #[serde(default = "default_osc_all_songs")]
    pub(super) all_songs: String,
    /// The OSC address to look for to switch back to the current playlist.
    #[serde(default = "default_osc_playlist")]
    pub(super) playlist: String,
    /// The OSC address to look for to stop all triggered samples.
    #[serde(default = "default_osc_stop_samples")]
    pub(super) stop_samples: String,
    /// The OSC address to acknowledge the current section (arm section loop).
    #[serde(default = "default_osc_section_ack")]
    pub(super) section_ack: String,
    /// The OSC address to break out of the current section loop.
    #[serde(default = "default_osc_stop_section_loop")]
    pub(super) stop_section_loop: String,
    /// The OSC address to loop a specific section by name (takes a string arg).
    #[serde(default = "default_osc_loop_section")]
    pub(super) loop_section: String,
    /// The OSC address to seek within the current song (takes a numeric
    /// seconds arg).
    #[serde(default = "default_osc_seek")]
    pub(super) seek: String,
    /// The OSC address to seek to the start of a named section (takes a
    /// string arg).
    #[serde(default = "default_osc_seek_section")]
    pub(super) seek_section: String,
    /// The OSC address to hold ambient light up (ambient mode `on`).
    #[serde(default = "default_osc_ambient_on")]
    pub(super) ambient_on: String,
    /// The OSC address to turn ambient light off (ambient mode `off`).
    #[serde(default = "default_osc_ambient_off")]
    pub(super) ambient_off: String,
    /// The OSC address to give ambient light back to the player (ambient
    /// mode `auto`).
    #[serde(default = "default_osc_ambient_auto")]
    pub(super) ambient_auto: String,
    /// The OSC address to call off an auto play waiting to start.
    #[serde(default = "default_osc_cancel_auto_play")]
    pub(super) cancel_auto_play: String,
    /// The OSC address pattern to set an output track's gain in dB (takes a
    /// float arg). The `*` segment is the track name; the same pattern (with
    /// the name substituted) is used for gain feedback broadcasts.
    #[serde(default = "default_osc_track_gain")]
    pub(super) track_gain: String,
    /// The OSC address to broadcast to display the current player status.
    #[serde(default = "default_osc_status")]
    pub(super) status: String,
    /// The OSC address to broadcast the audio output's health verdict:
    /// "healthy", "recovering", "stalled" or "never_started".
    ///
    /// Separate from `status`, which reports what the *player* is doing. A
    /// control surface wants both: the player can be playing while the audio
    /// output is stalled, and that combination is the one worth a red light.
    #[serde(default = "default_osc_audio_health")]
    pub(super) audio_health: String,
    /// The OSC address to broadcast the current playlist songs.
    #[serde(default = "default_osc_playlist_current")]
    pub(super) playlist_current: String,
    /// The OSC address to broadcast to display the current song on the playlist.
    #[serde(default = "default_osc_playlist_current_song")]
    pub(super) playlist_current_song: String,
    /// The OSC address to broadcast to display the current song elapsed duration.
    #[serde(default = "default_osc_playlist_current_song_elapsed")]
    pub(super) playlist_current_song_elapsed: String,
    /// The OSC address to broadcast numeric playback progress and the current
    /// song's dynamically named section boundaries.
    #[serde(default = "default_osc_timeline")]
    pub(super) timeline: String,
}

pub(super) fn default_osc_port() -> u16 {
    DEFAULT_OSC_PORT
}

/// Configuration for Morningstar MIDI controller integration.
///
/// When present, mtrack will send SysEx messages to update the current bank
/// name on the controller whenever the current song changes.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MorningstarConfig {
    /// The Morningstar controller model.
    pub(super) model: MorningstarModel,
    /// Whether to save the name to flash (true) or keep it temporary (false).
    #[serde(default)]
    pub(super) save: bool,
}

/// Morningstar controller model.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum MorningstarModel {
    MC3,
    MC6,
    MC8,
    #[serde(rename = "mc6pro")]
    MC6Pro,
    #[serde(rename = "mc8pro")]
    MC8Pro,
    #[serde(rename = "mc4pro")]
    MC4Pro,
    Custom(CustomModel),
}

/// Custom Morningstar model with explicit device ID.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CustomModel {
    pub model_id: u8,
}
