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

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use std::path::PathBuf;

use super::super::safe_path::VerifiedRoot;
use super::lighting_writes::Answer;

/// The project directory: where mtrack.yaml lives.
///
/// The one place the server will create a directory on a config's behalf. A
/// configured path outside it belongs to the operator — creating it turns a
/// typo into an empty directory, and writes outside the paths the generated
/// systemd unit fences the service into.
pub fn project_dir(state: &crate::webui::server::WebUiState) -> PathBuf {
    crate::util::project_dir_of(&state.config_path)
}

/// Validates a resource name for use in file paths.
///
/// Rejects names that would be unsafe as filenames (empty, path traversal, etc.)
/// and a name the caller reserves (`reserved`).
#[allow(clippy::result_large_err)]
pub(crate) fn validate_resource_name(
    name: &str,
    label: &str,
    reserved: bool,
) -> Result<(), axum::response::Response> {
    use super::super::safe_path::SafePath;
    if reserved || SafePath::validate_name(name).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid {} name", label)})),
        )
            .into_response());
    }
    Ok(())
}

/// Validates the name of a resource being written: one not already on disk
/// (`exists` false) is being created, and must also pass the rules for new
/// names ([`SafePath::validate_new_name`]). One already there is saved under
/// its own name, which only needs to be safe.
///
/// [`SafePath::validate_new_name`]: super::super::safe_path::SafePath::validate_new_name
#[allow(clippy::result_large_err)]
pub(crate) fn validate_resource_name_to_write(
    name: &str,
    label: &str,
    exists: bool,
) -> Result<(), axum::response::Response> {
    use super::super::safe_path::SafePath;
    if SafePath::validate_name_to_write(name, exists).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid {} name", label)})),
        )
            .into_response());
    }
    Ok(())
}

/// The file `file` holds a resource by in `dir` (a playlist's or a
/// profile's, as `util::yaml_file_at` found it in the listing, or the one
/// to create), held inside `dir`: one the listing has is canonicalized and
/// checked to stay inside, as symlinks could take it out. Whether
/// it exists is the listing's answer, not asked of the filesystem again.
/// A new file named as one there but for case is refused (409).
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_named_file(
    dir: &std::path::Path,
    file: Result<crate::util::NamedFile, crate::util::CaseClash>,
) -> Result<crate::util::NamedFile, axum::response::Response> {
    let file = file.map_err(|e| {
        (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response()
    })?;
    let root = VerifiedRoot::new(dir).map_err(|e| e.into_response())?;
    let joined = match file.path().file_name() {
        Some(file_name) => root.as_path().join(file_name),
        None => file.path().to_path_buf(),
    };
    if file.exists() {
        let safe = super::super::safe_path::SafePath::resolve(&joined, &root)
            .map_err(|e| e.into_response())?;
        Ok(file.with_path(safe.as_path().to_path_buf()))
    } else {
        Ok(file.with_path(joined))
    }
}

/// The answer to a write of `path` (`config_io::write`): a create beaten to
/// the file by one already there (`AlreadyExists`) is `exists`, a 409;
/// another failure a 500.
#[allow(clippy::result_large_err)]
pub(crate) fn written(
    result: std::io::Result<()>,
    path: &std::path::Path,
    what: &str,
) -> Result<(), axum::response::Response> {
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err((
            StatusCode::CONFLICT,
            Json(json!({"error": format!(
                "a {what} file `{}` already exists",
                crate::util::filename_display(path)
            )})),
        )
            .into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": crate::util::write_failure_message(crate::util::WriteTarget::File(path), &e)
            })),
        )
            .into_response()),
    }
}

/// Creates a configured directory, before anything canonicalizes it.
///
/// Ordering matters: [`resolve_named_file`] verifies the root by
/// canonicalizing it, which fails outright when the directory is not there. A
/// creation placed after that call can never run — the request has already
/// failed with "Root directory not found", which is also what a fresh install
/// used to meet instead of having the directory made for it.
///
/// A path outside the project or blocked by a file is the operator's to fix, so
/// it answers 400 rather than 500.
pub(crate) async fn ensure_configured_dir(
    dir: &std::path::Path,
    state: &crate::webui::server::WebUiState,
) -> Result<PathBuf, axum::response::Response> {
    ensure_dir_within(dir, &project_dir(state)).await
}

/// [`ensure_configured_dir`] inside the project directory `project`.
pub(crate) async fn ensure_dir_within(
    dir: &std::path::Path,
    project: &std::path::Path,
) -> Result<PathBuf, axum::response::Response> {
    let owned = dir.to_path_buf();
    let project = project.to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || make_configured_dir(&owned, &project)).await;

    match outcome {
        Ok(Ok(created)) => Ok(created),
        Ok(Err((status, error))) => Err((status, Json(json!({"error": error}))).into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create directory: {e}")})),
        )
            .into_response()),
    }
}

/// [`ensure_configured_dir`] on a blocking thread, with `project` the
/// directory it must be inside: the directory made, or the status and why.
pub(crate) fn make_configured_dir(
    dir: &std::path::Path,
    project: &std::path::Path,
) -> Result<PathBuf, (StatusCode, String)> {
    crate::util::create_dir_within(dir, project)
        .map(|created| created.into_path_buf())
        .map_err(|e| {
            let status = match e {
                crate::util::CreateWithinError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_REQUEST,
            };
            (status, e.to_string())
        })
}

/// Wraps a blocking filesystem operation in `tokio::task::spawn_blocking`,
/// mapping errors to HTTP responses.
///
/// Both the join error (task panic) and the inner result error are handled.
pub(crate) async fn spawn_blocking_io<F, T, E>(
    label: &str,
    f: F,
) -> Result<T, axum::response::Response>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    blocking_answer(label, f)
        .await
        .map_err(IntoResponse::into_response)
}

/// [`spawn_blocking_io`], with the failure as the plain [`Answer`] a
/// surface that is not HTTP (gRPC, MCP) can give as it stands.
pub(crate) async fn blocking_answer<F, T, E>(label: &str, f: F) -> Result<T, Answer>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    let label = label.to_string();
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err(Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to {}: {}", label, e),
        )),
        Err(e) => Err(Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{} task failed: {}", label, e),
        )),
    }
}

/// The names of the songs the player has loaded, or failed to: no new song
/// is given one of them (the bulk import, a track upload's new song).
pub(crate) fn player_song_names(
    player: &crate::player::Player,
) -> std::collections::HashSet<String> {
    let registry = player.songs();
    registry
        .list()
        .iter()
        .map(|song| song.name().to_string())
        .chain(registry.failures().iter().map(|f| f.name().to_string()))
        .collect()
}
