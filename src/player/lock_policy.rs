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

//! Which operations stay open while the player is locked, on every control
//! surface: one table, which MCP's `call_tool` gate, the gRPC service's
//! lock guard (`controller::grpc::LockGuard`, for native gRPC and gRPC-Web)
//! and the web UI's lock middleware all ask, and one refusal,
//! [`super::LockedError`].

/// The surface an operation is named on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// An MCP tool, by name.
    Mcp,
    /// A gRPC (and gRPC-Web) RPC, by its method name.
    Grpc,
    /// A web API route, as `METHOD /pattern` with the route's own pattern
    /// (never the request path) and no `/api` prefix. Every `GET` is a read
    /// and open.
    Web,
}

use Surface::{Grpc, Mcp, Web};

/// The operations that stay open while the player is locked: reads,
/// validation and offline evaluation, and the playback controls
/// (transport, playlist switch, samples, ambient light, section loops,
/// auto-play, track gains) and the lock itself. Every other operation writes config or project
/// files and is refused while locked, so a new one is locked out until it is
/// listed here.
pub const OPEN_WHILE_LOCKED: &[(Surface, &str)] = &[
    // MCP: reads.
    (Mcp, "status"),
    (Mcp, "host_info"),
    (Mcp, "list_songs"),
    (Mcp, "list_playlists"),
    (Mcp, "get_cues"),
    (Mcp, "get_active_effects"),
    (Mcp, "get_fixture_state"),
    (Mcp, "get_config"),
    (Mcp, "read_song"),
    (Mcp, "song_details"),
    (Mcp, "song_beat_grid"),
    (Mcp, "read_playlist"),
    (Mcp, "lighting_dsl_reference"),
    (Mcp, "validate_lighting"),
    (Mcp, "evaluate_show"),
    (Mcp, "analyze_show"),
    (Mcp, "diff_shows"),
    (Mcp, "list_venues"),
    (Mcp, "suggest_group_tags"),
    (Mcp, "list_groups"),
    (Mcp, "list_fixture_types"),
    (Mcp, "list_gdtf_modes"),
    (Mcp, "inspect_mvr"),
    (Mcp, "read_song_lighting"),
    (Mcp, "list_venue_files"),
    (Mcp, "read_venue"),
    (Mcp, "list_fixture_type_files"),
    (Mcp, "read_fixture_type"),
    // MCP: playback controls.
    (Mcp, "play"),
    (Mcp, "play_from"),
    (Mcp, "play_song_from"),
    (Mcp, "seek"),
    (Mcp, "seek_to_section"),
    (Mcp, "stop"),
    (Mcp, "next"),
    (Mcp, "previous"),
    (Mcp, "switch_playlist"),
    (Mcp, "stop_samples"),
    (Mcp, "set_ambient_mode"),
    (Mcp, "cancel_auto_play"),
    (Mcp, "loop_section"),
    (Mcp, "stop_section_loop"),
    (Mcp, "section_ack"),
    // gRPC: reads and playback controls.
    (Grpc, "Play"),
    (Grpc, "PlayFrom"),
    (Grpc, "PlaySongFrom"),
    (Grpc, "Seek"),
    (Grpc, "SeekToSection"),
    (Grpc, "Previous"),
    (Grpc, "Next"),
    (Grpc, "NavigateToSong"),
    (Grpc, "Stop"),
    (Grpc, "SwitchToPlaylist"),
    (Grpc, "Status"),
    (Grpc, "GetCues"),
    (Grpc, "GetActiveEffects"),
    (Grpc, "StopSamples"),
    (Grpc, "GetConfig"),
    (Grpc, "LoopSection"),
    (Grpc, "StopSectionLoop"),
    (Grpc, "SectionAck"),
    (Grpc, "SetTrackGain"),
    (Grpc, "SetTrackMute"),
    (Grpc, "SetAmbientMode"),
    (Grpc, "CancelAutoPlay"),
    (Grpc, "GetTrackGains"),
    (Grpc, "ListPlaylists"),
    (Grpc, "GetPlaylist"),
    (Grpc, "ListLightShows"),
    (Grpc, "ListSongs"),
    (Grpc, "GetSong"),
    (Grpc, "WatchStage"),
    (Grpc, "GetLightingAsset"),
    (Grpc, "WatchStatus"),
    (Grpc, "Watch"),
    (Grpc, "GetNetworkInfo"),
    // The lock itself: the phone's only way to unlock. SavePlaylist and
    // DeletePlaylist are playlist writes and stay locked out, as the web
    // UI's are.
    (Grpc, "SetLock"),
    // Web: the lock toggle, validation, playlist activation (a playback
    // control, not an edit), the controller restart, and the pure reads that
    // happen to be POSTs (a preview evaluation and the upload inspectors,
    // which write nothing).
    (Web, "PUT /lock"),
    (Web, "POST /lighting/validate"),
    (Web, "POST /playlists/{name}/activate"),
    (Web, "POST /controllers/restart"),
    (Web, "POST /lighting/evaluate"),
    (Web, "POST /lighting/mvr/inspect"),
    // Releasing a fixture test only ever makes the lights go back to the show.
    (Web, "DELETE /lighting/fixture-test"),
];

/// Whether `operation`, as `surface` names it, stays open while the player
/// is locked.
pub fn open_while_locked(surface: Surface, operation: &str) -> bool {
    (surface == Web && operation.starts_with("GET "))
        || OPEN_WHILE_LOCKED
            .iter()
            .any(|(s, name)| *s == surface && *name == operation)
}

/// The operations `surface` leaves open while locked, for its tests.
pub fn open_on(surface: Surface) -> impl Iterator<Item = &'static str> {
    OPEN_WHILE_LOCKED
        .iter()
        .filter(move |(s, _)| *s == surface)
        .map(|(_, name)| *name)
}

impl super::Player {
    /// Refuses `operation` with [`super::LockedError`] while the player is
    /// locked, unless [`OPEN_WHILE_LOCKED`] lists it for `surface`.
    pub fn check_lock(&self, surface: Surface, operation: &str) -> Result<(), super::LockedError> {
        if open_while_locked(surface, operation) {
            Ok(())
        } else {
            self.require_unlocked()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_gets_are_open_and_other_methods_are_listed() {
        assert!(open_while_locked(Web, "GET /songs"));
        assert!(open_while_locked(Web, "PUT /lock"));
        assert!(!open_while_locked(Web, "PUT /playlists/{name}"));
        // Names are per surface.
        assert!(open_while_locked(Grpc, "Play"));
        assert!(!open_while_locked(Mcp, "Play"));
        assert!(!open_while_locked(Grpc, "UpdateAudio"));
        // The phone's status stream and its Wi-Fi lookup are reads.
        assert!(open_while_locked(Grpc, "WatchStatus"));
        assert!(open_while_locked(Grpc, "Watch"));
        assert!(open_while_locked(Grpc, "GetNetworkInfo"));
    }
}
