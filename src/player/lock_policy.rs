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
//! surface: one table, which MCP's `call_tool` gate and the gRPC service's
//! lock guard (`controller::grpc::LockGuard`, for native gRPC and gRPC-Web,
//! on every service: an RPC is named by its method name alone, so no two
//! services may share one, which a test in `controller::grpc` holds) ask,
//! and one refusal, [`super::LockedError`].
//!
//! The web server has no table of its own: everything the web UI does is a
//! gRPC-Web call, held to the gRPC guard's. The three HTTP routes beside
//! the static page are not lock-checked, each for its own reason:
//! `PUT /uploads/{id}` and `GET /downloads/{id}` carry bytes for a transfer
//! id that `BeginUpload` and `ExportMvr` issued under this table (the id is
//! the permission), and `GET /lighting-assets/*` is a read of the asset
//! store, open while locked as every web `GET` always was (as is the
//! `GetLightingAsset` RPC).

/// The surface an operation is named on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// An MCP tool, by name.
    Mcp,
    /// A gRPC (and gRPC-Web) RPC, by its method name.
    Grpc,
}

use Surface::{Grpc, Mcp};

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
    // ConfigService's reads. Its writes (UpdateSamples, UpdateMetronome,
    // SaveProfileFile, DeleteProfileFile, and the audio probe and the
    // calibration calls, which open an audio device) stay locked out.
    (Grpc, "ListProfileFiles"),
    (Grpc, "GetProfileFile"),
    (Grpc, "ListAudioDevices"),
    (Grpc, "ListMidiDevices"),
    // The song reads: a song's config, its waveform, its tempo guess, its
    // files, and the project folder's listing. Creating, saving, deleting
    // and importing stay locked out.
    (Grpc, "GetSongConfig"),
    (Grpc, "GetSongWaveform"),
    (Grpc, "GuessSongTempo"),
    (Grpc, "ListSongFiles"),
    (Grpc, "BrowseDirectory"),
    // The Bluetooth controller's state is a read. Opening the pairing window
    // and forgetting a phone stay locked out: who may control the player is
    // not for a locked player to change.
    (Grpc, "GetBluetooth"),
    // The controller restart: open while locked, as it always was (it
    // changes nothing but restarting what the config already says).
    (Grpc, "RestartControllers"),
    // The lock itself: the phone's only way to unlock, and the web UI's
    // toggle. SavePlaylist and DeletePlaylist are playlist writes and stay
    // locked out.
    (Grpc, "SetLock"),
    // LightingService's reads, which include the validation and the offline
    // evaluation (they write nothing), and the release of a fixture test,
    // which only ever makes the lights go back to the show. Saving and
    // deleting a light show and starting a fixture test stay locked out.
    (Grpc, "GetLightShow"),
    (Grpc, "ValidateLightShow"),
    (Grpc, "GetLightingReadiness"),
    (Grpc, "GetLightingFit"),
    (Grpc, "EvaluateSongLighting"),
    (Grpc, "ListLightingGroups"),
    (Grpc, "StopFixtureTest"),
    (Grpc, "GetFixtureTestOptions"),
    // The fixture types' reads. Saving and deleting a type, and saving (or
    // planning) a fixture's settings, stay locked out, as the PUT, DELETE
    // and POST routes were.
    (Grpc, "ListFixtureTypes"),
    (Grpc, "GetFixtureType"),
    (Grpc, "GetFixtureTypeGdtfDetails"),
    (Grpc, "GetFixtureTypeSettings"),
    // The venues' reads, and the MVR export's summary (it writes nothing).
    // Saving, deleting, adding aim points and keeping an export stay locked
    // out, as the PUT, DELETE and POST routes were.
    (Grpc, "ListVenues"),
    (Grpc, "GetVenue"),
    (Grpc, "GetVenuePatch"),
    (Grpc, "GetVenueScene"),
    (Grpc, "GetMvrExportSummary"),
    // The file bytes. InspectMvr is the MVR inspector, a pure read that was
    // a POST. ExportMvr stages a download and writes nothing to the project,
    // as the GET it replaced. BeginUpload is open here, and refuses every
    // purpose but MVR_INSPECT itself while locked (the purpose is in the
    // request, not in the RPC's name): the claiming calls (AddTrackFiles,
    // AddSampleFile, ImportGdtf, ImportMvr) stay locked out.
    (Grpc, "BeginUpload"),
    (Grpc, "InspectMvr"),
    (Grpc, "ExportMvr"),
];

/// Whether `operation`, as `surface` names it, stays open while the player
/// is locked.
pub fn open_while_locked(surface: Surface, operation: &str) -> bool {
    OPEN_WHILE_LOCKED
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
    fn the_table_names_operations_per_surface() {
        // Names are per surface.
        assert!(open_while_locked(Grpc, "Play"));
        assert!(!open_while_locked(Mcp, "Play"));
        assert!(!open_while_locked(Grpc, "UpdateAudio"));
        // The phone's status stream and its Wi-Fi lookup are reads.
        assert!(open_while_locked(Grpc, "WatchStatus"));
        assert!(open_while_locked(Grpc, "Watch"));
        assert!(open_while_locked(Grpc, "GetNetworkInfo"));
        // ConfigService: its reads are open, its writes are not.
        assert!(open_while_locked(Grpc, "ListProfileFiles"));
        assert!(open_while_locked(Grpc, "GetProfileFile"));
        assert!(!open_while_locked(Grpc, "SaveProfileFile"));
        assert!(!open_while_locked(Grpc, "DeleteProfileFile"));
        assert!(!open_while_locked(Grpc, "UpdateSamples"));
        assert!(!open_while_locked(Grpc, "UpdateMetronome"));
        // The device lists and the controller restart are open; the probe
        // and the calibration calls open an audio device and are not.
        for open in ["ListAudioDevices", "ListMidiDevices", "RestartControllers"] {
            assert!(open_while_locked(Grpc, open), "{open}");
        }
        // The song calls: the reads are open, the writes are not.
        for open in [
            "GetSongConfig",
            "GetSongWaveform",
            "GuessSongTempo",
            "ListSongFiles",
            "BrowseDirectory",
        ] {
            assert!(open_while_locked(Grpc, open), "{open}");
        }
        for locked in [
            "CreateSong",
            "SaveSongConfig",
            "DeleteSong",
            "ImportSongFile",
            "CreateSongInDirectory",
            "BulkImportSongs",
        ] {
            assert!(!open_while_locked(Grpc, locked), "{locked}");
        }
        // LightingService: the reads, the validation, the evaluation and the
        // release of a fixture test are open; writing a show and starting a
        // test are not.
        for open in [
            "GetLightShow",
            "ValidateLightShow",
            "GetLightingReadiness",
            "GetLightingFit",
            "EvaluateSongLighting",
            "ListLightingGroups",
            "StopFixtureTest",
            "GetFixtureTestOptions",
            "ListFixtureTypes",
            "GetFixtureType",
            "GetFixtureTypeGdtfDetails",
            "GetFixtureTypeSettings",
            "ListVenues",
            "GetVenue",
            "GetVenuePatch",
            "GetVenueScene",
            "GetMvrExportSummary",
            "BeginUpload",
            "InspectMvr",
            "ExportMvr",
        ] {
            assert!(open_while_locked(Grpc, open), "{open}");
        }
        for locked in [
            "SaveLightShow",
            "DeleteLightShow",
            "StartFixtureTest",
            "SaveFixtureType",
            "DeleteFixtureType",
            "SaveFixtureTypeSettings",
            "SaveVenue",
            "DeleteVenue",
            "AddVenueAimPoints",
            "KeepMvrExport",
            "AddTrackFiles",
            "AddSampleFile",
            "ImportGdtf",
            "ImportMvr",
        ] {
            assert!(!open_while_locked(Grpc, locked), "{locked}");
        }
        // Bluetooth: the status is a read; pairing and forgetting are not.
        assert!(open_while_locked(Grpc, "GetBluetooth"));
        for locked in ["SetBluetoothPairing", "ForgetBluetoothDevice"] {
            assert!(!open_while_locked(Grpc, locked), "{locked}");
        }
        for locked in [
            "ProbeAudioDevice",
            "StartCalibration",
            "CaptureCalibration",
            "StopCalibration",
            "CancelCalibration",
        ] {
            assert!(!open_while_locked(Grpc, locked), "{locked}");
        }
    }
}
