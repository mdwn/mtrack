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

use crate::config;
use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use super::super::config_io;
use super::helpers::{validate_resource_name, validate_resource_name_to_write};

// ---------------------------------------------------------------------------
// Playlist file helpers, shared by the gRPC service and MCP
// ---------------------------------------------------------------------------

/// Validates a playlist name for use in file paths.
/// The player's own playlist is refused (400) saying so.
#[allow(clippy::result_large_err)]
pub(crate) fn validate_playlist_name(name: &str) -> Result<(), axum::response::Response> {
    if crate::playlist::is_reserved_name(name) {
        let reserved = crate::playlist::PlaylistFileError::Reserved(name.to_string());
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": reserved.to_string()})),
        )
            .into_response());
    }
    validate_resource_name(name, "playlist", false)
}

/// The file that holds the playlist `name` in `dir` (`playlist::playlist_file`:
/// its `.yaml`, else its `.yml`, as listed, else a new `.yaml`), checked to
/// be inside `dir`. The reserved name is refused (400), and a new file named
/// as one there but for case (409).
#[allow(clippy::result_large_err)]
fn playlist_path(
    dir: &std::path::Path,
    name: &str,
) -> Result<crate::util::NamedFile, axum::response::Response> {
    use crate::playlist::PlaylistFileError;
    let file = match crate::playlist::playlist_file(dir, name) {
        Ok(file) => Ok(file),
        Err(PlaylistFileError::CaseClash(clash)) => Err(clash),
        Err(e @ PlaylistFileError::Reserved(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": e.to_string()})),
            )
                .into_response())
        }
    };
    super::helpers::resolve_named_file(dir, file)
}

/// A gap as written in a playlist: whole seconds as `2s`, else `1500ms`.
/// The gRPC playlist read's and MCP's.
pub(crate) fn format_gap(gap: std::time::Duration) -> String {
    let ms = gap.as_millis();
    if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// Held by each playlist save and delete from reading the file to reloading
/// the player, here and over MCP, so two at once (two editor tabs, say) take
/// turns: a save reads the file to keep what the editor does not send, and
/// one reading it while another rewrote it found it short and refused (#578).
pub(crate) static PLAYLIST_WRITES: super::write_lock::WriteLock =
    super::write_lock::WriteLock::new();

/// The file a save of the playlist `name` writes, in the playlists
/// directory `dir`, by the one rule of the gRPC `SavePlaylist` and MCP's
/// `write_playlist` and `patch_playlist`: the name checked
/// (`validate_playlist_name`; a new one by the rules for new names); the
/// playlist's file as the directory lists it (`playlist_path`), or a new
/// `<name>.yaml` when the directory is not there yet. Nothing is made:
/// [`write_playlist_file`] makes the directory, once the save is checked,
/// so a refused save leaves none behind.
pub(crate) async fn playlist_file_to_write(
    dir: &std::path::Path,
    name: &str,
) -> Result<crate::util::NamedFile, axum::response::Response> {
    validate_playlist_name(name)?;
    let dir_exists = tokio::fs::try_exists(dir).await.unwrap_or(false);
    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in playlist_path, and
    // a missing directory's by `ensure_dir_within` when it is made.
    let file = if dir_exists {
        playlist_path(dir, name)?
    } else {
        // No directory, no files: a new `<name>.yaml`.
        crate::playlist::playlist_file(dir, name).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        })?
    };
    validate_resource_name_to_write(name, "playlist", file.exists())?;
    Ok(file)
}

/// A playlist as the text of its file, which ends with a newline: the web
/// UI's, MCP's and gRPC's saves.
#[allow(clippy::result_large_err)]
pub(crate) fn playlist_yaml(
    playlist: &config::Playlist,
) -> Result<String, axum::response::Response> {
    match crate::util::to_yaml_string(playlist) {
        Ok(y) if y.ends_with('\n') => Ok(y),
        Ok(y) => Ok(y + "\n"),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to serialize playlist: {}", e)})),
        )
            .into_response()),
    }
}

/// Refuses (422) a playlist the player would refuse to load, before it is
/// written: a song it does not have, a bad fade or ambient, or a fade
/// leaving out a track its song does not have. So is a fade the player
/// would cut short, longer than the song it plays under, and an ambient
/// show outside the project. The gRPC and MCP playlist saves.
#[allow(clippy::result_large_err)]
pub(crate) fn check_playlist(
    playlist: &config::Playlist,
    player: &crate::player::Player,
) -> Result<(), axum::response::Response> {
    crate::playlist::validate_for_write(playlist, &player.songs()).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": e.to_string()})),
        )
            .into_response()
    })
}

/// Writes a playlist's `yaml` to its `file`, atomically, off the async
/// runtime: the gRPC and MCP playlist saves, once checked. The
/// file's directory is made first if missing, only inside `project`: a
/// `playlists_dir:` (or `playlist:`) pointing elsewhere is the operator's
/// to set up, like `songs` and the rest. The file is written under the
/// directory as made, not as spelled -- those differ when the setting
/// holds a `..`. A new playlist is created as new (`config_io::write`): a
/// file made since the lookup, or one the filesystem takes for this name,
/// is refused (409), never written over. Answers the path written.
pub(crate) async fn write_playlist_file(
    file: &crate::util::NamedFile,
    yaml: String,
    project: &std::path::Path,
) -> Result<std::path::PathBuf, axum::response::Response> {
    let file_path = match (file.path().parent(), file.path().file_name()) {
        (Some(parent), Some(file_name)) if !parent.as_os_str().is_empty() => {
            super::helpers::ensure_dir_within(parent, project)
                .await?
                .join(file_name)
        }
        _ => file.path().to_path_buf(),
    };
    let fp = file_path.clone();
    let new = !file.exists();
    let result = super::helpers::spawn_blocking_io("write playlist", move || {
        Ok::<_, String>(config_io::write(&fp, &yaml, new))
    })
    .await?;
    super::helpers::written(result, &file_path, "playlist")?;
    Ok(file_path)
}

/// Deletes the file of the playlist `name` (already checked by
/// [`validate_playlist_name`]) in `dir`, off the async runtime; 404 when
/// there is none. The caller holds [`PLAYLIST_WRITES`] and reloads the
/// player: the gRPC delete.
#[allow(clippy::result_large_err)]
pub(crate) async fn remove_playlist_file(
    dir: &std::path::Path,
    name: &str,
) -> Result<(), axum::response::Response> {
    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in playlist_path.
    let target = playlist_path(dir, name)?;
    if !target.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Playlist '{}' not found", name)})),
        )
            .into_response());
    }
    // Remove it off the async runtime.
    super::helpers::spawn_blocking_io("delete playlist", move || {
        std::fs::remove_file(target.path())
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::format_gap;

    #[test]
    fn gaps_are_written_as_a_playlist_would() {
        use std::time::Duration;
        assert_eq!(format_gap(Duration::from_secs(2)), "2s");
        assert_eq!(format_gap(Duration::from_millis(1500)), "1500ms");
    }

    #[test]
    fn validate_playlist_name_rejects_empty() {
        assert!(super::validate_playlist_name("").is_err());
    }

    #[test]
    fn validate_playlist_name_rejects_dots() {
        assert!(super::validate_playlist_name("..").is_err());
        assert!(super::validate_playlist_name("foo/..").is_err());
        assert!(super::validate_playlist_name("a\\b").is_err());
    }
}
