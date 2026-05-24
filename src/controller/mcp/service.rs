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
//! MCP server handler. Tool methods are thin adapters over [`Player`] and
//! [`ConfigStore`]; they translate inputs/outputs to JSON and propagate
//! domain errors as [`McpError`] values.
//!
//! **Error response shape convention.** Tool methods surface failures by
//! returning `Err(McpError)`, which the rmcp layer encodes as a top-level
//! JSON-RPC `error` object on the response:
//!
//!   * `McpError::invalid_params(...)` — bad caller input (missing arg,
//!     unknown song, non-unique patch, malformed JSON, unsupported resource
//!     URI, path-traversal attempt, invalid YAML/DSL).
//!   * `McpError::internal_error(...)` — backend failures the caller can't
//!     control (filesystem I/O errors, [`ConfigStore`] errors, player errors
//!     from `play` / `loop_section` / etc., absent [`ConfigStore`]).
//!
//! The one deliberate exception is `validate_lighting`. It always returns
//! a successful tool result; the verdict is encoded as
//! `{"ok": bool, "error"?: string, "shows"?: [...]}` so callers chained on it
//! (typically just before `write_*_lighting`) can branch on the result
//! without exception-handling. The `write_*_lighting` and `patch_*_lighting`
//! tools, by contrast, *do* return `McpError::invalid_params` on invalid
//! input because for them validation failure is a hard failure.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Implementation, ListResourcesResult, PaginatedRequestParams,
        ProtocolVersion, RawResource, ReadResourceRequestParams, ReadResourceResult, Resource,
        ResourceContents, ResourceUpdatedNotificationParam, ServerCapabilities, ServerInfo,
        SubscribeRequestParams, UnsubscribeRequestParams,
    },
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, Peer, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::task::AbortHandle;

use crate::player::Player;

/// Resource URI exposing the live player status as JSON. Clients can read it
/// directly or subscribe for `notifications/resources/updated` pushes on each
/// state change.
const RESOURCE_STATUS_URI: &str = "mtrack://status";
/// Resource URI exposing the current configuration YAML plus checksum.
const RESOURCE_CONFIG_URI: &str = "mtrack://config";

/// MCP server handler holding shared state for tool calls.
///
/// The `tool_router` field is populated by the `#[tool_router]` macro and read
/// indirectly through the generated `ServerHandler` impl.
#[allow(dead_code)]
#[derive(Clone)]
pub struct McpServer {
    player: Arc<Player>,
    tool_router: ToolRouter<McpServer>,
    /// Background tasks pushing resource-updated notifications for this
    /// session, keyed by resource URI. The handles abort on `Drop` to clean
    /// up cleanly when the session ends.
    subscriptions: Arc<Mutex<HashMap<String, SubscriptionHandle>>>,
}

/// Wraps a tokio `AbortHandle` so we drop subscriptions deterministically when
/// the `McpServer` (and therefore the session) is dropped.
struct SubscriptionHandle(AbortHandle);

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlayFromArgs {
    /// Time to start playback from, formatted as `mm:ss.mmm` or `Ns` (e.g. `1:23.456`, `45.5s`).
    pub start_time: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlaySongFromArgs {
    /// Name of the song in the active playlist (or all-songs registry).
    pub song_name: String,
    /// Optional time to start playback from (`mm:ss.mmm` or `Ns`). Defaults to song start.
    #[serde(default)]
    pub start_time: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SwitchPlaylistArgs {
    /// Name of the playlist to switch to.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoopSectionArgs {
    /// Name of the section to loop. Must match a section defined on the current song.
    pub section_name: String,
}

/// Common shape for any subsection-update tool. The `body` is the JSON form of
/// the subsection (e.g. an `Audio` block, an array of controllers, a profile).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateConfigArgs {
    /// JSON payload for the subsection being updated. Use `null` to clear an
    /// optional subsection (e.g. `audio: null`).
    pub body: serde_json::Value,
    /// Checksum returned by the most recent `get_config` call. The update is
    /// rejected if the on-disk config has changed since.
    pub expected_checksum: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProfileIndexArgs {
    /// Index of the profile in the `profiles` list.
    pub index: u32,
    /// JSON payload for the profile.
    pub body: serde_json::Value,
    /// Expected checksum from the last `get_config` call.
    pub expected_checksum: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveProfileArgs {
    /// Index of the profile to remove.
    pub index: u32,
    /// Expected checksum from the last `get_config` call.
    pub expected_checksum: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SongNameArgs {
    /// Name of the song as listed by `list_songs`.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteSongArgs {
    /// Name of the song. If the song doesn't yet exist, a new directory is
    /// created under the songs root with this name (after sanity-checking).
    pub name: String,
    /// Full YAML body to write to `song.yaml`. Must parse as a `config::Song`.
    pub yaml: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlaylistNameArgs {
    /// Optional playlist name. If omitted, returns the top-level `playlist:`
    /// file referenced from the mtrack config.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WritePlaylistArgs {
    /// Playlist name. Required when writing into the `playlists_dir`. If only
    /// the top-level `playlist:` file is configured and `name` is omitted, that
    /// file is replaced.
    #[serde(default)]
    pub name: Option<String>,
    /// Full YAML body to write. Must parse as a `config::Playlist`.
    pub yaml: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateLightingArgs {
    /// `.light` DSL source to validate.
    pub source: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SongLightingArgs {
    /// Song name as listed by `list_songs`.
    pub song: String,
    /// Lighting file basename, e.g. `main.light`. Must end with `.light`.
    pub file: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteSongLightingArgs {
    /// Song name as listed by `list_songs`.
    pub song: String,
    /// Lighting file basename to create or replace (e.g. `main.light`). Must
    /// end with `.light` and contain no path separators.
    pub file: String,
    /// Full `.light` DSL source. The server validates this with the lighting
    /// parser before writing.
    pub source: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LightingFileArgs {
    /// Basename of a `.light` file (no path separators).
    pub file: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteLightingFileArgs {
    /// Basename of the `.light` file to write (no path separators).
    pub file: String,
    /// Full `.light` DSL source. Validated before being written.
    pub source: String,
}

/// String replacement parameters shared by every patch tool. Matches the
/// `Edit`-tool ergonomic familiar from Claude Code: by default the change
/// applies only when `old_string` occurs exactly once, forcing the caller to
/// expand context until the match is unambiguous; `replace_all` opts into
/// blanket replacement.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchFields {
    /// Exact text to replace. Must occur in the file.
    pub old_string: String,
    /// Replacement text.
    pub new_string: String,
    /// If true, replace every occurrence. If false (default), the tool errors
    /// when `old_string` occurs more than once.
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchSongArgs {
    /// Name of the song as listed by `list_songs`.
    pub name: String,
    #[serde(flatten)]
    pub patch: PatchFields,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchPlaylistArgs {
    /// Optional playlist name. With `null`/omitted, edits the top-level
    /// `playlist:` file.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub patch: PatchFields,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchSongLightingArgs {
    /// Name of the song as listed by `list_songs`.
    pub song: String,
    /// Lighting file basename, e.g. `main.light`.
    pub file: String,
    #[serde(flatten)]
    pub patch: PatchFields,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchLightingFileArgs {
    /// Basename of the `.light` file under the configured directory.
    pub file: String,
    #[serde(flatten)]
    pub patch: PatchFields,
}

#[tool_router]
impl McpServer {
    pub fn new(player: Arc<Player>) -> Self {
        Self {
            player,
            tool_router: Self::tool_router(),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tool(description = "Return the current playback status: active playlist, \
        current song, whether a song is playing, and elapsed time.")]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        Ok(ok_json(self.status_snapshot().await?))
    }

    #[tool(description = "Return runtime identity for the connected mtrack \
        instance: resolved hostname, active hardware profile name, and the \
        live state (`connected`/`not_connected`/...) and device name of each \
        subsystem (audio, MIDI, DMX, trigger). Use this to confirm which host \
        a configuration's `profiles` list selected and what hardware is \
        actually wired up.")]
    async fn host_info(&self) -> Result<CallToolResult, McpError> {
        let snapshot = self.player.hardware_status();
        let json = serde_json::to_value(&snapshot).map_err(|e| {
            McpError::internal_error(format!("failed to serialize hardware status: {e}"), None)
        })?;
        Ok(ok_json(json))
    }

    #[tool(description = "List all songs loaded in the song repository. Returns name, \
        duration, and track count for each song.")]
    async fn list_songs(&self) -> Result<CallToolResult, McpError> {
        let songs = self.player.songs();
        let entries: Vec<Value> = songs
            .sorted_list()
            .iter()
            .map(|s| song_summary(s))
            .collect();
        Ok(ok_json(json!({
            "count": entries.len(),
            "songs": entries,
        })))
    }

    #[tool(description = "List the names of all configured playlists.")]
    async fn list_playlists(&self) -> Result<CallToolResult, McpError> {
        let names = self.player.list_playlists();
        Ok(ok_json(json!({ "playlists": names })))
    }

    #[tool(description = "Return the cue list (time + index) for the lighting timeline \
        of the song currently loaded by the player.")]
    async fn get_cues(&self) -> Result<CallToolResult, McpError> {
        let cues: Vec<Value> = self
            .player
            .get_cues()
            .into_iter()
            .map(|(time, index)| {
                json!({
                    "index": index,
                    "time": format_duration(time),
                })
            })
            .collect();
        Ok(ok_json(json!({ "cues": cues })))
    }

    #[tool(description = "Return a human-readable summary of all lighting effects \
        currently active on the player.")]
    async fn get_active_effects(&self) -> Result<CallToolResult, McpError> {
        let summary = self
            .player
            .format_active_effects()
            .unwrap_or_else(|| "(no effect engine configured)".to_string());
        Ok(CallToolResult::success(vec![Content::text(summary)]))
    }

    // ---- Playback control ----

    #[tool(description = "Start playback of the current song in the active playlist.")]
    async fn play(&self) -> Result<CallToolResult, McpError> {
        let song = self.player.play().await.map_err(internal_err)?;
        Ok(ok_json(json!({
            "now_playing": song.as_ref().map(|s| song_summary(s)),
        })))
    }

    #[tool(description = "Start playback of the current song from a specific time \
        (e.g. `1:23.456` or `45.5s`).")]
    async fn play_from(
        &self,
        Parameters(args): Parameters<PlayFromArgs>,
    ) -> Result<CallToolResult, McpError> {
        let start = parse_duration(&args.start_time)?;
        let song = self.player.play_from(start).await.map_err(internal_err)?;
        Ok(ok_json(json!({
            "now_playing": song.as_ref().map(|s| song_summary(s)),
            "start_time": format_duration(start),
        })))
    }

    #[tool(description = "Play a named song from a specific time. Switches the current \
        playlist position to that song.")]
    async fn play_song_from(
        &self,
        Parameters(args): Parameters<PlaySongFromArgs>,
    ) -> Result<CallToolResult, McpError> {
        let start = match args.start_time.as_deref() {
            Some(s) => parse_duration(s)?,
            None => std::time::Duration::ZERO,
        };
        let song = self
            .player
            .play_song_from(&args.song_name, start)
            .await
            .map_err(internal_err)?;
        Ok(ok_json(json!({
            "now_playing": song.as_ref().map(|s| song_summary(s)),
            "start_time": format_duration(start),
        })))
    }

    #[tool(description = "Stop playback of the currently playing song.")]
    async fn stop(&self) -> Result<CallToolResult, McpError> {
        let song = self.player.stop().await;
        Ok(ok_json(json!({
            "stopped": song.as_ref().map(|s| song_summary(s)),
        })))
    }

    #[tool(description = "Advance the active playlist to the next song. Returns the \
        new current song.")]
    async fn next(&self) -> Result<CallToolResult, McpError> {
        let song = self.player.next().await;
        Ok(ok_json(json!({
            "current_song": song.as_ref().map(|s| song_summary(s)),
        })))
    }

    #[tool(description = "Move the active playlist to the previous song. Returns the \
        new current song.")]
    async fn previous(&self) -> Result<CallToolResult, McpError> {
        let song = self.player.prev().await;
        Ok(ok_json(json!({
            "current_song": song.as_ref().map(|s| song_summary(s)),
        })))
    }

    #[tool(description = "Switch the player to a different playlist by name.")]
    async fn switch_playlist(
        &self,
        Parameters(args): Parameters<SwitchPlaylistArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.player
            .switch_to_playlist(&args.name)
            .await
            .map_err(|e| McpError::invalid_params(e, None))?;
        Ok(ok_json(json!({ "active_playlist": args.name })))
    }

    #[tool(description = "Stop all currently playing triggered samples.")]
    async fn stop_samples(&self) -> Result<CallToolResult, McpError> {
        self.player.stop_samples();
        Ok(ok_json(json!({ "ok": true })))
    }

    #[tool(description = "Activate section looping for a named section on the current song.")]
    async fn loop_section(
        &self,
        Parameters(args): Parameters<LoopSectionArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.player
            .loop_section(&args.section_name)
            .await
            .map_err(internal_err)?;
        Ok(ok_json(json!({ "looping": args.section_name })))
    }

    #[tool(description = "Deactivate section looping. The current iteration finishes \
        and playback continues past the section.")]
    async fn stop_section_loop(&self) -> Result<CallToolResult, McpError> {
        self.player.stop_section_loop();
        Ok(ok_json(json!({ "ok": true })))
    }

    #[tool(description = "Acknowledge the current section in reactive looping mode, \
        arming the loop so it engages at the section end.")]
    async fn section_ack(&self) -> Result<CallToolResult, McpError> {
        self.player.section_ack().await.map_err(internal_err)?;
        Ok(ok_json(json!({ "ok": true })))
    }

    // ---- Configuration ----

    #[tool(description = "Return the full mtrack configuration as YAML, plus a \
        checksum to pass to subsequent update tools for optimistic concurrency.")]
    async fn get_config(&self) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let (yaml, checksum) = store.read_yaml().await.map_err(internal_err)?;
        Ok(ok_json(json!({
            "yaml": yaml,
            "checksum": checksum,
        })))
    }

    #[tool(description = "Update the `audio` subsection of the configuration. The \
        `body` field accepts the same JSON structure as the `audio:` YAML block, \
        or `null` to remove it.")]
    async fn update_audio(
        &self,
        Parameters(args): Parameters<UpdateConfigArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let audio: Option<crate::config::Audio> = parse_optional(args.body)?;
        let snapshot = store
            .update_audio(audio, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    #[tool(description = "Update the `midi` subsection of the configuration. Pass \
        `null` to remove it.")]
    async fn update_midi(
        &self,
        Parameters(args): Parameters<UpdateConfigArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let midi: Option<crate::config::Midi> = parse_optional(args.body)?;
        let snapshot = store
            .update_midi(midi, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    #[tool(description = "Update the `dmx` subsection of the configuration. Pass \
        `null` to remove it.")]
    async fn update_dmx(
        &self,
        Parameters(args): Parameters<UpdateConfigArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let dmx: Option<crate::config::Dmx> = parse_optional(args.body)?;
        let snapshot = store
            .update_dmx(dmx, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    #[tool(description = "Replace the entire `controllers` list. Pass an array of \
        controller definitions, each tagged with a `kind` discriminator \
        (`grpc`, `mcp`, `osc`, `midi`).")]
    async fn update_controllers(
        &self,
        Parameters(args): Parameters<UpdateConfigArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let controllers: Vec<crate::config::Controller> = serde_json::from_value(args.body)
            .map_err(|e| McpError::invalid_params(format!("invalid controllers: {e}"), None))?;
        let snapshot = store
            .update_controllers(controllers, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    #[tool(description = "Append a new hardware profile to the `profiles` list.")]
    async fn add_profile(
        &self,
        Parameters(args): Parameters<UpdateConfigArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let profile: crate::config::Profile = serde_json::from_value(args.body)
            .map_err(|e| McpError::invalid_params(format!("invalid profile: {e}"), None))?;
        let snapshot = store
            .add_profile(profile, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    #[tool(description = "Replace the profile at a given index in the `profiles` list.")]
    async fn update_profile(
        &self,
        Parameters(args): Parameters<ProfileIndexArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let profile: crate::config::Profile = serde_json::from_value(args.body)
            .map_err(|e| McpError::invalid_params(format!("invalid profile: {e}"), None))?;
        let snapshot = store
            .update_profile(args.index as usize, profile, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    #[tool(description = "Remove the profile at a given index in the `profiles` list.")]
    async fn remove_profile(
        &self,
        Parameters(args): Parameters<RemoveProfileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.config_store()?;
        let snapshot = store
            .remove_profile(args.index as usize, &args.expected_checksum)
            .await
            .map_err(internal_err)?;
        Ok(snapshot_response(&snapshot))
    }

    // ---- Song / playlist file editing ----

    #[tool(description = "Read the raw `song.yaml` for a song. Works for both \
        directory-layout songs (`<dir>/song.yaml`) and flat-layout songs \
        (`<dir>/<name>.yaml`) — the exact source file recorded at load time \
        is used.")]
    async fn read_song(
        &self,
        Parameters(args): Parameters<SongNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        let song = self
            .player
            .songs()
            .get(&args.name)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let path = song
            .config_path()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| song.base_path().join("song.yaml"));
        let body = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpError::internal_error(format!("failed to read {}: {e}", path.display()), None)
        })?;
        Ok(ok_json(json!({
            "name": song.name(),
            "path": path.display().to_string(),
            "yaml": body,
        })))
    }

    #[tool(description = "Return detailed metadata for a loaded song: track \
        names with source file + channel, sections, lighting show references, \
        MIDI playback presence, loop flag, and (when the song's click track \
        has been analyzed) the full beat grid, the dominant BPM, and a \
        `tempo_segments` list that breaks the song into runs of roughly \
        constant tempo (each with `start_seconds`, `end_seconds`, \
        `beat_count`, `bpm`). The segments are useful for songs with \
        half-time bridges or shifting feels, where the single `bpm` field \
        only reports the dominant tempo.")]
    async fn song_details(
        &self,
        Parameters(args): Parameters<SongNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        let song = self
            .player
            .songs()
            .get(&args.name)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let tracks: Vec<Value> = song
            .tracks()
            .iter()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "file": t.file().display().to_string(),
                    "file_channel": t.file_channel(),
                })
            })
            .collect();

        let sections: Vec<Value> = song
            .sections()
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "start_measure": s.start_measure,
                    "end_measure": s.end_measure,
                })
            })
            .collect();

        let beat_grid = song.beat_grid().map(|g| {
            json!({
                "beat_count": g.beats.len(),
                "measure_count": g.measure_starts.len(),
                "beats": g.beats,
                "measure_starts": g.measure_starts,
            })
        });
        let bpm = song
            .beat_grid()
            .and_then(|g| compute_bpm_from_beats(&g.beats));
        let tempo_segments = song
            .beat_grid()
            .map(|g| compute_tempo_segments(&g.beats))
            .unwrap_or_default();

        let light_shows: Vec<Value> = song
            .light_shows()
            .iter()
            .map(|ls| {
                json!({
                    "universe": ls.universe_name(),
                    "file": ls.dmx_file_path().display().to_string(),
                })
            })
            .collect();
        let dsl_lighting_shows: Vec<Value> = song
            .dsl_lighting_shows()
            .iter()
            .map(|s| {
                let show_names: Vec<&String> = s.shows().keys().collect();
                json!({
                    "file": s.file_path().display().to_string(),
                    "shows": show_names,
                })
            })
            .collect();

        Ok(ok_json(json!({
            "name": song.name(),
            "base_path": song.base_path().display().to_string(),
            "config_path": song.config_path().map(|p| p.display().to_string()),
            "duration": format_duration(song.duration()),
            "duration_seconds": song.duration().as_secs_f64(),
            "num_channels": song.num_channels(),
            "loop_playback": song.loop_playback(),
            "has_midi_playback": song.midi_playback().is_some(),
            "tracks": tracks,
            "sections": sections,
            "beat_grid": beat_grid,
            "bpm": bpm,
            "tempo_segments": tempo_segments,
            "light_shows": light_shows,
            "dsl_lighting_shows": dsl_lighting_shows,
        })))
    }

    #[tool(description = "Write a song's `song.yaml`. Validates the YAML against \
        `config::Song` before writing. If the song doesn't exist yet, creates a \
        new song directory under the songs root.")]
    async fn write_song(
        &self,
        Parameters(args): Parameters<WriteSongArgs>,
    ) -> Result<CallToolResult, McpError> {
        // Validate the YAML parses as a song config before we touch disk.
        let _: crate::config::Song = serde_yaml_from_str(&args.yaml)?;

        let songs_root = self.songs_root_verified().await?;
        let song_dir = match self.player.songs().get(&args.name) {
            Ok(existing) => crate::webui::safe_path::SafePath::resolve(
                existing.base_path(),
                &songs_root,
            )
            .map_err(safepath_err)?,
            Err(_) => {
                crate::webui::safe_path::SafePath::validate_name(&args.name)
                    .map_err(safepath_err)?;
                crate::webui::safe_path::SafePath::create_dir(
                    &songs_root.as_safe_path(),
                    &args.name,
                    &songs_root,
                )
                .map_err(safepath_err)?
            }
        };
        let yaml_path = song_dir.join_filename("song.yaml");
        atomic_write_string(&yaml_path, &args.yaml).await?;
        // Rescan the songs directory so list_songs / read_song see the new or
        // updated song without requiring an mtrack restart.
        self.reload_songs_from_config().await?;
        Ok(ok_json(json!({
            "path": yaml_path.display().to_string(),
            "bytes": args.yaml.len(),
        })))
    }

    #[tool(description = "Read a playlist YAML file. With no `name`, reads the \
        top-level `playlist:` file from the mtrack config. With a `name`, reads \
        `<playlists_dir>/<name>.yaml`.")]
    async fn read_playlist(
        &self,
        Parameters(args): Parameters<PlaylistNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = self.resolve_playlist_path(args.name.as_deref()).await?;
        let body = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpError::internal_error(format!("failed to read {}: {e}", path.display()), None)
        })?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "yaml": body,
        })))
    }

    #[tool(description = "Write a playlist YAML file. Validates the YAML against \
        `config::Playlist` before writing. With no `name`, writes the top-level \
        playlist file. With a `name`, writes `<playlists_dir>/<name>.yaml` and \
        creates the file if missing.")]
    async fn write_playlist(
        &self,
        Parameters(args): Parameters<WritePlaylistArgs>,
    ) -> Result<CallToolResult, McpError> {
        let _: crate::config::Playlist = serde_yaml_from_str(&args.yaml)?;
        let path = self.resolve_playlist_path(args.name.as_deref()).await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                McpError::internal_error(format!("failed to create {}: {e}", parent.display()), None)
            })?;
        }
        atomic_write_string(&path, &args.yaml).await?;
        // Rebuild the player's playlist set so `list_playlists` /
        // `switch_playlist` see the new file without requiring a restart.
        self.reload_songs_from_config().await?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "bytes": args.yaml.len(),
        })))
    }

    // ---- Lighting ----

    #[tool(description = "Return a concise reference for the mtrack lighting DSL. \
        ALWAYS read this before generating `.light` files.")]
    async fn lighting_dsl_reference(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            include_str!("dsl_reference.md").to_string(),
        )]))
    }

    #[tool(description = "Parse a `.light` DSL source and report any syntax or \
        semantic errors. Returns the parsed show names and a summary of cues if \
        valid; an error message otherwise.")]
    async fn validate_lighting(
        &self,
        Parameters(args): Parameters<ValidateLightingArgs>,
    ) -> Result<CallToolResult, McpError> {
        match crate::lighting::parser::parse_light_shows(&args.source) {
            Ok(shows) => {
                let summary: Vec<Value> = shows
                    .iter()
                    .map(|(name, show)| {
                        json!({
                            "name": name,
                            "cues": show.cues.len(),
                        })
                    })
                    .collect();
                Ok(ok_json(json!({
                    "ok": true,
                    "shows": summary,
                })))
            }
            Err(e) => Ok(ok_json(json!({
                "ok": false,
                "error": e.to_string(),
            }))),
        }
    }

    #[tool(description = "List the venues known to the running DMX engine. Each \
        venue lists its groups and fixture count.")]
    async fn list_venues(&self) -> Result<CallToolResult, McpError> {
        let dmx = match self.player.dmx_engine() {
            Some(d) => d,
            None => return Ok(ok_json(json!({ "venues": [] }))),
        };
        let system = match dmx.broadcast_handles().lighting_system {
            Some(s) => s,
            None => return Ok(ok_json(json!({ "venues": [] }))),
        };
        let guard = system.lock();
        let venues: Vec<Value> = guard
            .venues_iter()
            .map(|(name, venue)| {
                let groups: Vec<&String> = venue.groups().keys().collect();
                json!({
                    "name": name,
                    "fixtures": venue.fixtures().len(),
                    "groups": groups,
                })
            })
            .collect();
        Ok(ok_json(json!({
            "current_venue": guard.current_venue(),
            "venues": venues,
        })))
    }

    #[tool(description = "List every group name valid as a cue target. Returns \
        both venue-defined groups (explicit member lists from `venue \"...\" { … }` \
        blocks) and logical groups (tag/constraint-based, declared under \
        `dmx.lighting.groups` in the player config), each tagged with its \
        `source`. Logical groups include their constraints and, when a venue \
        is loaded, the fixtures they currently resolve to.")]
    async fn list_groups(&self) -> Result<CallToolResult, McpError> {
        let dmx = match self.player.dmx_engine() {
            Some(d) => d,
            None => return Ok(ok_json(json!({ "groups": [] }))),
        };
        let system = match dmx.broadcast_handles().lighting_system {
            Some(s) => s,
            None => return Ok(ok_json(json!({ "groups": [] }))),
        };

        // First pass: snapshot venue groups + logical group definitions while
        // holding the guard immutably. `resolve_logical_group_graceful` takes
        // `&mut self`, so we drop and re-acquire for the resolution pass to
        // avoid borrow-checker contortions.
        let snapshot: GroupSnapshot = {
            let guard = system.lock();
            let venue = guard.current_venue().map(str::to_owned);
            let venue_groups = guard
                .get_current_venue()
                .map(|v| {
                    v.groups()
                        .iter()
                        .map(|(name, group)| {
                            json!({
                                "name": name,
                                "source": "venue",
                                "fixtures": group.fixtures(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let logical_defs = guard
                .logical_groups_iter()
                .map(|(name, group)| {
                    let constraints = group
                        .constraints()
                        .iter()
                        .map(|c| format!("{c:?}"))
                        .collect();
                    (name.clone(), constraints)
                })
                .collect();
            GroupSnapshot {
                venue,
                venue_groups,
                logical_defs,
            }
        };

        // Second pass: resolve each logical group against the current venue
        // so callers see what fixtures the group would actually target right
        // now. Resolution mutates the cache, hence the mutable lock.
        let mut groups = snapshot.venue_groups;
        {
            let mut guard = system.lock();
            for (name, constraints) in snapshot.logical_defs {
                let fixtures = guard.resolve_logical_group_graceful(&name);
                groups.push(json!({
                    "name": name,
                    "source": "logical",
                    "constraints": constraints,
                    "fixtures": fixtures,
                }));
            }
        }

        Ok(ok_json(json!({
            "venue": snapshot.venue,
            "groups": groups,
        })))
    }

    #[tool(description = "List the fixture types known to the lighting engine.")]
    async fn list_fixture_types(&self) -> Result<CallToolResult, McpError> {
        let dmx = match self.player.dmx_engine() {
            Some(d) => d,
            None => return Ok(ok_json(json!({ "fixture_types": [] }))),
        };
        let system = match dmx.broadcast_handles().lighting_system {
            Some(s) => s,
            None => return Ok(ok_json(json!({ "fixture_types": [] }))),
        };
        let guard = system.lock();
        let types: Vec<Value> = guard
            .fixture_types_iter()
            .map(|(name, ft)| {
                json!({
                    "name": name,
                    "channels": ft.channels(),
                })
            })
            .collect();
        Ok(ok_json(json!({ "fixture_types": types })))
    }

    #[tool(description = "Read a lighting `.light` file from a song's lighting \
        directory. The file must be located under `<song>/lighting/`.")]
    async fn read_song_lighting(
        &self,
        Parameters(args): Parameters<SongLightingArgs>,
    ) -> Result<CallToolResult, McpError> {
        let song = self
            .player
            .songs()
            .get(&args.song)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        validate_lighting_filename(&args.file)?;
        let path = song.base_path().join("lighting").join(&args.file);
        let body = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpError::internal_error(format!("failed to read {}: {e}", path.display()), None)
        })?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "source": body,
        })))
    }

    #[tool(description = "List the venue `.light` files in the configured \
        venues directory. Returns the resolved directory and a list of basenames.")]
    async fn list_venue_files(&self) -> Result<CallToolResult, McpError> {
        let dir = self.resolve_lighting_dir(LightingDirKind::Venues).await?;
        let entries = list_light_files(&dir).await?;
        Ok(ok_json(json!({
            "dir": dir.display().to_string(),
            "files": entries,
        })))
    }

    #[tool(description = "Read a venue `.light` file by basename.")]
    async fn read_venue(
        &self,
        Parameters(args): Parameters<LightingFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = self
            .resolve_lighting_file(LightingDirKind::Venues, &args.file)
            .await?;
        let body = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpError::internal_error(format!("failed to read {}: {e}", path.display()), None)
        })?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "source": body,
        })))
    }

    #[tool(description = "Validate and write a venue `.light` file into the \
        configured venues directory. The DSL is parsed with the in-tree venue \
        parser; on failure the file is not written.")]
    async fn write_venue(
        &self,
        Parameters(args): Parameters<WriteLightingFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        crate::lighting::parser::parse_venues(&args.source)
            .map_err(|e| McpError::invalid_params(format!("invalid venue: {e}"), None))?;
        let path = self
            .resolve_lighting_file(LightingDirKind::Venues, &args.file)
            .await?;
        atomic_write_string(&path, &args.source).await?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "bytes": args.source.len(),
        })))
    }

    #[tool(description = "List the fixture-type `.light` files in the \
        configured fixture types directory.")]
    async fn list_fixture_type_files(&self) -> Result<CallToolResult, McpError> {
        let dir = self.resolve_lighting_dir(LightingDirKind::FixtureTypes).await?;
        let entries = list_light_files(&dir).await?;
        Ok(ok_json(json!({
            "dir": dir.display().to_string(),
            "files": entries,
        })))
    }

    #[tool(description = "Read a fixture-type `.light` file by basename.")]
    async fn read_fixture_type(
        &self,
        Parameters(args): Parameters<LightingFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = self
            .resolve_lighting_file(LightingDirKind::FixtureTypes, &args.file)
            .await?;
        let body = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpError::internal_error(format!("failed to read {}: {e}", path.display()), None)
        })?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "source": body,
        })))
    }

    #[tool(description = "Validate and write a fixture-type `.light` file into \
        the configured fixture types directory. The DSL is parsed first; on \
        failure the file is not written.")]
    async fn write_fixture_type(
        &self,
        Parameters(args): Parameters<WriteLightingFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        crate::lighting::parser::parse_fixture_types(&args.source)
            .map_err(|e| McpError::invalid_params(format!("invalid fixture type: {e}"), None))?;
        let path = self
            .resolve_lighting_file(LightingDirKind::FixtureTypes, &args.file)
            .await?;
        atomic_write_string(&path, &args.source).await?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "bytes": args.source.len(),
        })))
    }

    #[tool(description = "Validate and write a lighting `.light` file into a \
        song's `lighting/` directory. The DSL is parsed first; on failure, the \
        file is not written.")]
    async fn write_song_lighting(
        &self,
        Parameters(args): Parameters<WriteSongLightingArgs>,
    ) -> Result<CallToolResult, McpError> {
        crate::lighting::parser::parse_light_shows(&args.source)
            .map_err(|e| McpError::invalid_params(format!("invalid .light source: {e}"), None))?;
        validate_lighting_filename(&args.file)?;
        let song = self
            .player
            .songs()
            .get(&args.song)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let lighting_dir = song.base_path().join("lighting");
        tokio::fs::create_dir_all(&lighting_dir).await.map_err(|e| {
            McpError::internal_error(
                format!("failed to create {}: {e}", lighting_dir.display()),
                None,
            )
        })?;
        let path = lighting_dir.join(&args.file);
        atomic_write_string(&path, &args.source).await?;
        Ok(ok_json(json!({
            "path": path.display().to_string(),
            "bytes": args.source.len(),
        })))
    }

    // ---- Patch (string-replace) tools ----

    #[tool(description = "Patch a song's `song.yaml` with a string replacement. \
        `old_string` must occur in the file (and must be unique unless \
        `replace_all` is true). The result is parse-validated as `config::Song` \
        before being written.")]
    async fn patch_song(
        &self,
        Parameters(args): Parameters<PatchSongArgs>,
    ) -> Result<CallToolResult, McpError> {
        let song = self
            .player
            .songs()
            .get(&args.name)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let path = song
            .config_path()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| song.base_path().join("song.yaml"));
        let original = read_text(&path).await?;
        let updated = apply_patch(&original, &args.patch)?;
        let _: crate::config::Song = serde_yaml_from_str(&updated)?;
        atomic_write_string(&path, &updated).await?;
        self.reload_songs_from_config().await?;
        Ok(patch_response(&path, &original, &updated))
    }

    #[tool(description = "Patch a playlist YAML file with a string replacement. \
        The result is parse-validated as `config::Playlist` before being \
        written. With no `name`, edits the top-level playlist file.")]
    async fn patch_playlist(
        &self,
        Parameters(args): Parameters<PatchPlaylistArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = self.resolve_playlist_path(args.name.as_deref()).await?;
        let original = read_text(&path).await?;
        let updated = apply_patch(&original, &args.patch)?;
        let _: crate::config::Playlist = serde_yaml_from_str(&updated)?;
        atomic_write_string(&path, &updated).await?;
        self.reload_songs_from_config().await?;
        Ok(patch_response(&path, &original, &updated))
    }

    #[tool(description = "Patch a `.light` file in a song's `lighting/` \
        directory with a string replacement. The result is parsed with the \
        lighting parser before being written.")]
    async fn patch_song_lighting(
        &self,
        Parameters(args): Parameters<PatchSongLightingArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_lighting_filename(&args.file)?;
        let song = self
            .player
            .songs()
            .get(&args.song)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let path = song.base_path().join("lighting").join(&args.file);
        let original = read_text(&path).await?;
        let updated = apply_patch(&original, &args.patch)?;
        crate::lighting::parser::parse_light_shows(&updated)
            .map_err(|e| McpError::invalid_params(format!("patched .light is invalid: {e}"), None))?;
        atomic_write_string(&path, &updated).await?;
        Ok(patch_response(&path, &original, &updated))
    }

    #[tool(description = "Patch a venue `.light` file with a string replacement. \
        The result is parsed with the venue parser before being written.")]
    async fn patch_venue(
        &self,
        Parameters(args): Parameters<PatchLightingFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = self
            .resolve_lighting_file(LightingDirKind::Venues, &args.file)
            .await?;
        let original = read_text(&path).await?;
        let updated = apply_patch(&original, &args.patch)?;
        crate::lighting::parser::parse_venues(&updated)
            .map_err(|e| McpError::invalid_params(format!("patched venue is invalid: {e}"), None))?;
        atomic_write_string(&path, &updated).await?;
        Ok(patch_response(&path, &original, &updated))
    }

    #[tool(description = "Patch a fixture-type `.light` file with a string \
        replacement. The result is parsed with the fixture-type parser before \
        being written.")]
    async fn patch_fixture_type(
        &self,
        Parameters(args): Parameters<PatchLightingFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let path = self
            .resolve_lighting_file(LightingDirKind::FixtureTypes, &args.file)
            .await?;
        let original = read_text(&path).await?;
        let updated = apply_patch(&original, &args.patch)?;
        crate::lighting::parser::parse_fixture_types(&updated).map_err(|e| {
            McpError::invalid_params(format!("patched fixture type is invalid: {e}"), None)
        })?;
        atomic_write_string(&path, &updated).await?;
        Ok(patch_response(&path, &original, &updated))
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "mtrack MCP server. Inspect and control the running multitrack player: \
             query playback status, list songs, edit playlists, and create lighting shows. \
             Use the `lighting_dsl_reference` tool before generating .light files. \
             Subscribe to `mtrack://status` or `mtrack://config` for resource-updated \
             notifications when playback state or configuration changes."
                .to_string(),
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                Resource {
                    raw: RawResource {
                        uri: RESOURCE_STATUS_URI.to_string(),
                        name: "Player Status".to_string(),
                        title: None,
                        description: Some(
                            "Live JSON snapshot of the player: active playlist, \
                             current song, playing flag, elapsed time."
                                .to_string(),
                        ),
                        mime_type: Some("application/json".to_string()),
                        size: None,
                        icons: None,
                        meta: None,
                    },
                    annotations: None,
                },
                Resource {
                    raw: RawResource {
                        uri: RESOURCE_CONFIG_URI.to_string(),
                        name: "Mtrack Configuration".to_string(),
                        title: None,
                        description: Some(
                            "Full mtrack configuration as YAML, with the current \
                             checksum for optimistic-concurrency edits."
                                .to_string(),
                        ),
                        mime_type: Some("application/x-yaml".to_string()),
                        size: None,
                        icons: None,
                        meta: None,
                    },
                    annotations: None,
                },
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match request.uri.as_str() {
            RESOURCE_STATUS_URI => {
                let json = self.status_snapshot().await?;
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string()),
                    request.uri,
                )]))
            }
            RESOURCE_CONFIG_URI => {
                let store = self.config_store()?;
                let (yaml, checksum) = store.read_yaml().await.map_err(internal_err)?;
                let envelope = json!({"yaml": yaml, "checksum": checksum});
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    serde_json::to_string_pretty(&envelope)
                        .unwrap_or_else(|_| envelope.to_string()),
                    request.uri,
                )]))
            }
            other => Err(McpError::resource_not_found(
                "unknown resource",
                Some(json!({"uri": other})),
            )),
        }
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let uri = request.uri.clone();
        // Idempotent — re-subscribing replaces any existing watcher.
        if self.subscriptions.lock().contains_key(&uri) {
            return Ok(());
        }
        let handle = match uri.as_str() {
            RESOURCE_STATUS_URI => self.spawn_status_subscription(ctx.peer.clone())?,
            RESOURCE_CONFIG_URI => self.spawn_config_subscription(ctx.peer.clone())?,
            other => {
                return Err(McpError::invalid_params(
                    format!("unknown resource: {other}"),
                    None,
                ));
            }
        };
        self.subscriptions
            .lock()
            .insert(uri, SubscriptionHandle(handle));
        Ok(())
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.subscriptions.lock().remove(&request.uri);
        Ok(())
    }
}

/// Returns a successful tool result wrapping a JSON value as pretty-printed text.
pub(crate) fn ok_json(value: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    CallToolResult::success(vec![Content::text(text)])
}

/// Derives BPM from a click-track beat grid using the median inter-beat
/// interval. Median is robust to tempo changes and bad first/last beats;
/// when a song has a single uniform tempo it matches the mean.
pub(crate) fn compute_bpm_from_beats(beats: &[f64]) -> Option<f64> {
    if beats.len() < 2 {
        return None;
    }
    let mut intervals: Vec<f64> =
        beats.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = intervals[intervals.len() / 2];
    if median > 0.0 {
        Some(60.0 / median)
    } else {
        None
    }
}

/// Width (in intervals) of the median window used to smooth out individual
/// jittery beats when detecting tempo changes. 5 = consider 2 before / 2
/// after the focal interval.
const TEMPO_SMOOTHING_WINDOW: usize = 5;
/// Minimum length, in intervals, for a tempo segment to survive the noise
/// filter. 8 intervals ≈ 2 measures of 4/4 — short enough to catch real
/// musical sections, long enough to drop click-track flickers.
const TEMPO_MIN_SEGMENT_INTERVALS: usize = 8;

/// Splits a beat grid into contiguous tempo segments. Within each segment
/// the BPM is roughly constant (rounded to integer); between segments the
/// BPM changes by at least 1. Short noise segments are folded into their
/// larger neighbours and equal-BPM adjacents are re-merged afterwards so
/// the output is a stable, low-cardinality summary of the song's
/// arrangement (e.g. an 8-minute song with a half-time bridge yields
/// three segments rather than a sea of single-beat fluctuations).
pub(crate) fn compute_tempo_segments(beats: &[f64]) -> Vec<Value> {
    if beats.len() < TEMPO_SMOOTHING_WINDOW + 2 {
        return Vec::new();
    }
    let intervals: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
    if intervals.is_empty() {
        return Vec::new();
    }

    let half = TEMPO_SMOOTHING_WINDOW / 2;
    let smoothed: Vec<f64> = (0..intervals.len())
        .map(|i| {
            let start = i.saturating_sub(half);
            let end = (i + half + 1).min(intervals.len());
            let mut window: Vec<f64> = intervals[start..end]
                .iter()
                .copied()
                .filter(|d| *d > 0.0)
                .collect();
            if window.is_empty() {
                return f64::NAN;
            }
            window.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = window[window.len() / 2];
            (60.0 / median).round()
        })
        .collect();

    // Run-length encode the smoothed BPM trace.
    let mut segs: Vec<(usize, usize, f64)> = Vec::new(); // (start_interval, end_interval_inclusive, bpm)
    let mut run_start = 0usize;
    let mut current = smoothed[0];
    for (i, &bpm) in smoothed.iter().enumerate().skip(1) {
        if (bpm - current).abs() >= 0.5 {
            segs.push((run_start, i - 1, current));
            run_start = i;
            current = bpm;
        }
    }
    segs.push((run_start, smoothed.len() - 1, current));

    // Noise filter: repeatedly fold the shortest sub-threshold segment into
    // its larger neighbour, then re-merge any adjacent equal-BPM runs that
    // result. Terminates when no sub-threshold segment remains (or there's
    // only one segment left).
    loop {
        let short = segs
            .iter()
            .enumerate()
            .filter(|(_, (s, e, _))| e - s + 1 < TEMPO_MIN_SEGMENT_INTERVALS)
            .min_by_key(|(_, (s, e, _))| e - s + 1)
            .map(|(idx, _)| idx);
        let Some(idx) = short else { break };
        if segs.len() == 1 {
            break;
        }
        let left_len = if idx > 0 {
            segs[idx - 1].1 - segs[idx - 1].0 + 1
        } else {
            0
        };
        let right_len = if idx + 1 < segs.len() {
            segs[idx + 1].1 - segs[idx + 1].0 + 1
        } else {
            0
        };
        let merge_left = idx > 0 && (idx + 1 >= segs.len() || left_len >= right_len);
        if merge_left {
            segs[idx - 1].1 = segs[idx].1;
            segs.remove(idx);
        } else {
            segs[idx + 1].0 = segs[idx].0;
            segs.remove(idx);
        }
        // Re-merge any newly-adjacent equal-BPM runs.
        let mut i = 0;
        while i + 1 < segs.len() {
            if (segs[i].2 - segs[i + 1].2).abs() < 0.5 {
                segs[i].1 = segs[i + 1].1;
                segs.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    segs.into_iter()
        .map(|(s, e, bpm)| {
            // Interval s lies between beats s and s+1; interval e lies
            // between beats e and e+1. So the segment spans beats s..=e+1.
            let start_beat = s;
            let end_beat = e + 1;
            json!({
                "start_seconds": beats[start_beat],
                "end_seconds": beats[end_beat],
                "beat_count": end_beat - start_beat + 1,
                "bpm": bpm,
            })
        })
        .collect()
}

/// Formats a [`std::time::Duration`] as `M:SS.mmm`.
pub(crate) fn format_duration(d: std::time::Duration) -> String {
    let total = d.as_secs_f64();
    let minutes = (total / 60.0).floor() as u64;
    let seconds = total - (minutes as f64) * 60.0;
    format!("{}:{:06.3}", minutes, seconds)
}

/// Builds a compact summary value for a song.
pub(crate) fn song_summary(song: &crate::songs::Song) -> Value {
    let tracks: Vec<&str> = song.tracks().iter().map(|t| t.name()).collect();
    let sections: Vec<Value> = song
        .sections()
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "start_measure": s.start_measure,
                "end_measure": s.end_measure,
            })
        })
        .collect();
    json!({
        "name": song.name(),
        "duration": format_duration(song.duration()),
        "tracks": tracks,
        "sections": sections,
    })
}

/// Parses an `mm:ss.mmm` or `Ns` / `N.Ns` time string into a [`Duration`].
pub(crate) fn parse_duration(value: &str) -> Result<std::time::Duration, McpError> {
    let trimmed = value.trim();
    let seconds = if let Some(stripped) = trimmed
        .strip_suffix('s')
        .or_else(|| trimmed.strip_suffix('S'))
    {
        if stripped.contains(':') {
            parse_mm_ss(stripped)?
        } else {
            stripped
                .parse::<f64>()
                .map_err(|e| McpError::invalid_params(format!("invalid seconds: {e}"), None))?
        }
    } else if trimmed.contains(':') {
        parse_mm_ss(trimmed)?
    } else {
        trimmed
            .parse::<f64>()
            .map_err(|e| McpError::invalid_params(format!("invalid duration: {e}"), None))?
    };
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(McpError::invalid_params(
            "duration must be a finite non-negative number",
            None,
        ));
    }
    Ok(std::time::Duration::from_secs_f64(seconds))
}

fn parse_mm_ss(s: &str) -> Result<f64, McpError> {
    let (mm, ss) = s
        .split_once(':')
        .ok_or_else(|| McpError::invalid_params("expected mm:ss.mmm", None))?;
    let minutes: u64 = mm
        .parse()
        .map_err(|e| McpError::invalid_params(format!("invalid minutes: {e}"), None))?;
    let seconds: f64 = ss
        .parse()
        .map_err(|e| McpError::invalid_params(format!("invalid seconds: {e}"), None))?;
    Ok((minutes as f64) * 60.0 + seconds)
}

/// Wraps a domain error into [`McpError::internal_error`].
pub(crate) fn internal_err<E: std::fmt::Display>(e: E) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

impl McpServer {
    /// Builds the JSON snapshot returned by `tools/call status` and by
    /// reading the `mtrack://status` resource. Kept in one place so the two
    /// surfaces never drift.
    pub(crate) async fn status_snapshot(&self) -> Result<Value, McpError> {
        let playlist = self.player.get_playlist();
        let current = playlist.current();
        let playing = self.player.is_playing().await;
        let elapsed = self
            .player
            .elapsed()
            .await
            .map_err(internal_err)?
            .unwrap_or_default();
        Ok(json!({
            "playlist_name": playlist.name(),
            "current_song": current.as_ref().map(|s| song_summary(s)),
            "playing": playing,
            "elapsed": format_duration(elapsed),
        }))
    }

    /// Watches the player's state-snapshot channel and pushes a
    /// `notifications/resources/updated` to the subscribed peer on each change.
    fn spawn_status_subscription(
        &self,
        peer: Peer<RoleServer>,
    ) -> Result<AbortHandle, McpError> {
        let mut rx = self.player.state_rx().ok_or_else(|| {
            McpError::internal_error(
                "player state watch is not enabled; cannot subscribe to mtrack://status",
                None,
            )
        })?;
        let handle = tokio::spawn(async move {
            // Mark the initial value as seen so we only emit on actual changes
            // after subscribe returns.
            rx.mark_unchanged();
            while rx.changed().await.is_ok() {
                let params = ResourceUpdatedNotificationParam {
                    uri: RESOURCE_STATUS_URI.to_string(),
                };
                if peer.notify_resource_updated(params).await.is_err() {
                    break;
                }
            }
        });
        Ok(handle.abort_handle())
    }

    /// Watches the `ConfigStore` broadcast and pushes updates for the config
    /// resource. The receiver lags rather than blocks the producer; we treat
    /// lag the same as a fresh change since the next `read_resource` call
    /// will get the current state regardless.
    fn spawn_config_subscription(
        &self,
        peer: Peer<RoleServer>,
    ) -> Result<AbortHandle, McpError> {
        let store = self.config_store()?;
        let mut rx = store.subscribe();
        let handle = tokio::spawn(async move {
            // Treat `Lagged` the same as a fresh change: the client will fetch
            // the current state on the next `resources/read`, so a missed
            // intermediate broadcast doesn't matter. `Closed` ends the loop.
            #[allow(clippy::while_let_loop)]
            loop {
                match rx.recv().await {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let params = ResourceUpdatedNotificationParam {
                            uri: RESOURCE_CONFIG_URI.to_string(),
                        };
                        if peer.notify_resource_updated(params).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(handle.abort_handle())
    }

    /// Returns the player's [`ConfigStore`] or an MCP error when none is wired.
    pub(crate) fn config_store(
        &self,
    ) -> Result<Arc<crate::config::store::ConfigStore>, McpError> {
        self.player.config_store().ok_or_else(|| {
            McpError::internal_error(
                "config store is not enabled (mtrack was not started with a mutable config)",
                None,
            )
        })
    }

    /// Resolves the songs root path from the live config and returns it as a
    /// `VerifiedRoot` (canonicalized, used for traversal-safe path joins).
    pub(crate) async fn songs_root_verified(
        &self,
    ) -> Result<crate::webui::safe_path::VerifiedRoot, McpError> {
        let store = self.config_store()?;
        let path = store.path().to_path_buf();
        let cfg = store.read_config().await;
        let songs = cfg.songs(&path);
        if !songs.exists() {
            tokio::fs::create_dir_all(&songs).await.map_err(|e| {
                McpError::internal_error(
                    format!("failed to create songs dir {}: {e}", songs.display()),
                    None,
                )
            })?;
        }
        crate::webui::safe_path::VerifiedRoot::new(&songs).map_err(safepath_err)
    }

    /// Resolves the configured lighting subdirectory (venues or fixture types)
    /// to an absolute path. The directory is created if it doesn't yet exist,
    /// so write tools work against fresh configs.
    pub(crate) async fn resolve_lighting_dir(
        &self,
        kind: LightingDirKind,
    ) -> Result<std::path::PathBuf, McpError> {
        let store = self.config_store()?;
        let config_path = store.path().to_path_buf();
        let cfg = store.read_config().await;
        let lighting = cfg.lighting_from_profiles().ok_or_else(|| {
            McpError::invalid_params(
                "no lighting configuration in the active profile",
                None,
            )
        })?;
        let dirs = lighting.directories().ok_or_else(|| {
            McpError::invalid_params(
                "no lighting `directories:` configured (expected `fixture_types`/`venues`)",
                None,
            )
        })?;
        let rel = match kind {
            LightingDirKind::Venues => dirs.venues(),
            LightingDirKind::FixtureTypes => dirs.fixture_types(),
        }
        .ok_or_else(|| {
            McpError::invalid_params(
                format!("no `{}` directory configured under `lighting.directories`", kind.field_name()),
                None,
            )
        })?;
        let rel_path = std::path::PathBuf::from(rel);
        let dir = if rel_path.is_absolute() {
            rel_path
        } else {
            let parent = config_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            parent.join(rel_path)
        };
        if !dir.exists() {
            tokio::fs::create_dir_all(&dir).await.map_err(|e| {
                McpError::internal_error(
                    format!("failed to create lighting dir {}: {e}", dir.display()),
                    None,
                )
            })?;
        }
        Ok(dir)
    }

    /// Resolves a single `.light` filename inside a lighting subdirectory,
    /// after validating the basename for path-traversal safety and the
    /// `.light` extension.
    pub(crate) async fn resolve_lighting_file(
        &self,
        kind: LightingDirKind,
        name: &str,
    ) -> Result<std::path::PathBuf, McpError> {
        validate_lighting_filename(name)?;
        let dir = self.resolve_lighting_dir(kind).await?;
        Ok(dir.join(name))
    }

    /// Rescans songs from disk and rebuilds the player's playlists. Used after
    /// any tool that creates, edits, or removes a song file so subsequent
    /// `list_songs` / `read_song` / `write_song_lighting` calls see the new
    /// state without requiring an mtrack restart. Mirrors the resolution
    /// logic in `cli::local::start`.
    pub(crate) async fn reload_songs_from_config(&self) -> Result<(), McpError> {
        let store = self.config_store()?;
        let config_path = store.path().to_path_buf();
        let cfg = store.read_config().await;

        let songs_path = cfg.songs(&config_path);
        let playlists_dir = cfg
            .playlists_dir(&config_path)
            .or_else(|| config_path.parent().map(|p| p.join("playlists")));

        let legacy_playlist_path = cfg.playlist().map(|rel| {
            if rel.is_absolute() {
                rel
            } else {
                let parent = config_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                parent.join(rel)
            }
        });

        self.player.reload_songs(
            &songs_path,
            playlists_dir.as_deref(),
            legacy_playlist_path.as_deref(),
        );
        Ok(())
    }

    /// Resolves a playlist file path. With `None`, returns the top-level
    /// `playlist:` file referenced from the config. With `Some(name)`, returns
    /// `<playlists_dir>/<name>.yaml` (after validating `name`).
    pub(crate) async fn resolve_playlist_path(
        &self,
        name: Option<&str>,
    ) -> Result<std::path::PathBuf, McpError> {
        let store = self.config_store()?;
        let config_path = store.path().to_path_buf();
        let cfg = store.read_config().await;
        match name {
            Some(n) => {
                crate::webui::safe_path::SafePath::validate_name(n).map_err(safepath_err)?;
                let dir = cfg.playlists_dir(&config_path).ok_or_else(|| {
                    McpError::invalid_params(
                        "no playlists_dir configured; cannot resolve named playlist",
                        None,
                    )
                })?;
                Ok(dir.join(format!("{n}.yaml")))
            }
            None => {
                let rel = cfg.playlist().ok_or_else(|| {
                    McpError::invalid_params(
                        "no top-level playlist file configured",
                        None,
                    )
                })?;
                if rel.is_absolute() {
                    Ok(rel)
                } else {
                    let parent = config_path
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    Ok(parent.join(rel))
                }
            }
        }
    }
}

/// Parses an `Option<T>` from a JSON value, accepting `null` as `None`.
pub(crate) fn parse_optional<T: for<'de> serde::Deserialize<'de>>(
    value: Value,
) -> Result<Option<T>, McpError> {
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| McpError::invalid_params(format!("invalid payload: {e}"), None))
}

/// Returns a tool response containing the updated config YAML and new checksum.
pub(crate) fn snapshot_response(snapshot: &crate::config::store::ConfigSnapshot) -> CallToolResult {
    ok_json(json!({
        "yaml": snapshot.yaml,
        "checksum": snapshot.checksum,
    }))
}

/// Parses a YAML string into `T`, returning an MCP `invalid_params` error.
pub(crate) fn serde_yaml_from_str<T: for<'de> serde::Deserialize<'de>>(
    yaml: &str,
) -> Result<T, McpError> {
    let cfg = ::config::Config::builder()
        .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
        .build()
        .map_err(|e| McpError::invalid_params(format!("invalid YAML: {e}"), None))?;
    cfg.try_deserialize()
        .map_err(|e| McpError::invalid_params(format!("invalid payload: {e}"), None))
}

/// Wraps a [`SafePathError`] as an `invalid_params` MCP error.
pub(crate) fn safepath_err(err: crate::webui::safe_path::SafePathError) -> McpError {
    McpError::invalid_params(err.to_string(), None)
}

/// Validates that `name` is a single path segment ending in `.light`.
pub(crate) fn validate_lighting_filename(name: &str) -> Result<(), McpError> {
    crate::webui::safe_path::SafePath::validate_name(name).map_err(safepath_err)?;
    if !name.ends_with(".light") {
        return Err(McpError::invalid_params(
            "lighting filename must end with .light",
            None,
        ));
    }
    Ok(())
}

/// Snapshot taken under an immutable lock of the lighting system so the
/// `list_groups` tool can release the lock before re-acquiring it mutably
/// for logical-group resolution.
struct GroupSnapshot {
    venue: Option<String>,
    venue_groups: Vec<Value>,
    logical_defs: Vec<(String, Vec<String>)>,
}

/// Identifies one of the two lighting subdirectories configured under
/// `lighting.directories`. Used by the venue / fixture-type tools.
#[derive(Copy, Clone, Debug)]
pub(crate) enum LightingDirKind {
    Venues,
    FixtureTypes,
}

impl LightingDirKind {
    pub(crate) fn field_name(self) -> &'static str {
        match self {
            LightingDirKind::Venues => "venues",
            LightingDirKind::FixtureTypes => "fixture_types",
        }
    }
}

/// Applies a string-replacement patch with the same semantics as Claude Code's
/// `Edit` tool: `old_string` must be present; if it occurs more than once and
/// `replace_all` is false, the patch is rejected so the caller can disambiguate
/// by adding context. Returns the new file contents.
pub(crate) fn apply_patch(
    content: &str,
    patch: &PatchFields,
) -> Result<String, McpError> {
    if patch.old_string.is_empty() {
        return Err(McpError::invalid_params(
            "old_string must not be empty",
            None,
        ));
    }
    if patch.old_string == patch.new_string {
        return Err(McpError::invalid_params(
            "old_string and new_string are identical",
            None,
        ));
    }
    let count = content.matches(patch.old_string.as_str()).count();
    if count == 0 {
        return Err(McpError::invalid_params(
            "old_string was not found in the file",
            None,
        ));
    }
    if count > 1 && !patch.replace_all {
        return Err(McpError::invalid_params(
            format!(
                "old_string occurs {count} times; expand the surrounding \
                 context until it is unique, or pass replace_all: true"
            ),
            None,
        ));
    }
    Ok(if patch.replace_all {
        content.replace(&patch.old_string, &patch.new_string)
    } else {
        content.replacen(&patch.old_string, &patch.new_string, 1)
    })
}

/// Standard response shape for a successful patch tool call. Echoes path,
/// before/after byte counts, and the new contents so callers can verify the
/// edit without an extra round-trip.
pub(crate) fn patch_response(
    path: &std::path::Path,
    before: &str,
    after: &str,
) -> CallToolResult {
    ok_json(json!({
        "path": path.display().to_string(),
        "bytes_before": before.len(),
        "bytes_after": after.len(),
        "contents": after,
    }))
}

/// Reads a file as UTF-8, returning an MCP-friendly error on I/O failure.
pub(crate) async fn read_text(path: &std::path::Path) -> Result<String, McpError> {
    tokio::fs::read_to_string(path).await.map_err(|e| {
        McpError::internal_error(format!("failed to read {}: {e}", path.display()), None)
    })
}

/// Lists `.light` files directly under `dir` (no recursion). Subdirectories
/// and other extensions are skipped silently.
pub(crate) async fn list_light_files(dir: &std::path::Path) -> Result<Vec<String>, McpError> {
    let mut out = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| {
        McpError::internal_error(format!("failed to list {}: {e}", dir.display()), None)
    })?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| McpError::internal_error(format!("read_dir entry: {e}"), None))?
    {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("light") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Writes a string atomically to `path` via tempfile-then-rename. Reuses the
/// existing pattern used by the config store.
pub(crate) async fn atomic_write_string(
    path: &std::path::Path,
    content: &str,
) -> Result<(), McpError> {
    let path = path.to_path_buf();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("path has no parent"))?;
        std::fs::create_dir_all(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        tmp.write_all(content.as_bytes())?;
        tmp.flush()?;
        tmp.persist(&path).map_err(|e| e.error)?;
        Ok(())
    })
    .await
    .map_err(|e| McpError::internal_error(format!("join error: {e}"), None))?
    .map_err(|e| McpError::internal_error(format!("write failed: {e}"), None))
}

#[cfg(test)]
mod bpm_tests {
    use super::{compute_bpm_from_beats, compute_tempo_segments};

    #[test]
    fn empty_or_singleton_yields_none() {
        assert!(compute_bpm_from_beats(&[]).is_none());
        assert!(compute_bpm_from_beats(&[1.0]).is_none());
    }

    #[test]
    fn uniform_120bpm() {
        // 120 BPM = 0.5s per beat
        let beats: Vec<f64> = (0..16).map(|i| i as f64 * 0.5).collect();
        let bpm = compute_bpm_from_beats(&beats).unwrap();
        assert!((bpm - 120.0).abs() < 1e-9, "expected 120, got {bpm}");
    }

    #[test]
    fn median_rejects_outliers() {
        // One badly mistimed beat shouldn't move BPM much.
        let mut beats: Vec<f64> = (0..20).map(|i| i as f64 * 0.5).collect();
        beats.push(beats.last().unwrap() + 5.0); // huge gap at the end
        let bpm = compute_bpm_from_beats(&beats).unwrap();
        assert!((bpm - 120.0).abs() < 1.0, "median should resist outlier; got {bpm}");
    }

    #[test]
    fn duplicate_beats_dropped() {
        // Two beats at identical time would produce a 0-interval; filter
        // those out rather than dividing by zero.
        let beats = vec![0.0, 0.0, 0.5, 1.0, 1.5];
        let bpm = compute_bpm_from_beats(&beats).unwrap();
        assert!(bpm.is_finite() && bpm > 0.0);
    }

    /// Append `count` beats at `bpm` starting at `start` to `beats`. Returns
    /// the time at which the last appended beat lands (so callers can chain).
    fn extend_at(beats: &mut Vec<f64>, start: f64, bpm: f64, count: usize) -> f64 {
        let interval = 60.0 / bpm;
        let mut t = start;
        for _ in 0..count {
            beats.push(t);
            t += interval;
        }
        // Return where the next beat would land; chains naturally feed this
        // as `start` to keep the tempo continuous up to the boundary.
        t
    }

    fn bpms(segments: &[serde_json::Value]) -> Vec<f64> {
        segments
            .iter()
            .map(|s| s["bpm"].as_f64().unwrap())
            .collect()
    }

    #[test]
    fn segments_uniform_song_yields_one_segment() {
        let mut beats = Vec::new();
        extend_at(&mut beats, 0.0, 120.0, 64);
        let segments = compute_tempo_segments(&beats);
        assert_eq!(bpms(&segments), vec![120.0]);
    }

    #[test]
    fn segments_detect_half_time_bridge() {
        // A song with: 100 beats at 120, then 100 at 60 (half-time), then 100 at 120.
        // This is the canonical "Sigurd's Song" shape we want to surface
        // cleanly instead of hiding behind a single median BPM.
        let mut beats = Vec::new();
        let t1 = extend_at(&mut beats, 0.0, 120.0, 100);
        let t2 = extend_at(&mut beats, t1, 60.0, 100);
        let _ = extend_at(&mut beats, t2, 120.0, 100);

        let segments = compute_tempo_segments(&beats);
        let detected = bpms(&segments);
        assert_eq!(
            detected,
            vec![120.0, 60.0, 120.0],
            "expected three distinct segments, got {detected:?}",
        );
    }

    #[test]
    fn segments_smooth_over_isolated_outliers() {
        // 60 beats at 120 BPM with a single jittered beat in the middle
        // should still produce a single segment.
        let mut beats: Vec<f64> = (0..60).map(|i| i as f64 * 0.5).collect();
        beats[30] += 0.05; // a small smear, well within median tolerance
        beats[31] -= 0.05;
        let segments = compute_tempo_segments(&beats);
        assert_eq!(
            segments.len(),
            1,
            "single outlier should not split a segment; got {segments:?}",
        );
    }

    #[test]
    fn segments_drop_short_intermediate_runs() {
        // 40 beats at 120, 3 beats at random tempo, 40 beats at 120.
        // The short run is under TEMPO_MIN_SEGMENT_INTERVALS so it should
        // be folded back into the surrounding 120 BPM segment.
        let mut beats = Vec::new();
        let t1 = extend_at(&mut beats, 0.0, 120.0, 40);
        let t2 = extend_at(&mut beats, t1, 80.0, 3);
        let _ = extend_at(&mut beats, t2, 120.0, 40);
        let segments = compute_tempo_segments(&beats);
        assert_eq!(
            bpms(&segments),
            vec![120.0],
            "tiny intermediate run should have been merged; got {segments:?}",
        );
    }

    #[test]
    fn segments_handle_too_few_beats() {
        // Anything below the smoothing window simply returns no segments
        // rather than panicking or emitting noise.
        assert!(compute_tempo_segments(&[]).is_empty());
        assert!(compute_tempo_segments(&[0.0, 0.5]).is_empty());
    }
}
