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
/// and optionally rejects a reserved name.
#[allow(clippy::result_large_err)]
pub(crate) fn validate_resource_name(
    name: &str,
    label: &str,
    reserved: Option<&str>,
) -> Result<(), axum::response::Response> {
    use super::super::safe_path::SafePath;
    let is_reserved = reserved.is_some_and(|r| name == r);
    if is_reserved || SafePath::validate_name(name).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid {} name", label)})),
        )
            .into_response());
    }
    Ok(())
}

/// Returns a configured directory path, or an error response if it is `None`.
#[allow(clippy::result_large_err)]
pub(crate) fn require_configured_dir(
    dir: &Option<PathBuf>,
    label: &str,
    status: StatusCode,
) -> Result<PathBuf, axum::response::Response> {
    dir.clone().ok_or_else(|| {
        (
            status,
            Json(json!({"error": format!("No {} directory configured", label)})),
        )
            .into_response()
    })
}

/// Resolves a resource file path within a directory, verifying that the result
/// does not escape via symlinks or other tricks.
///
/// For existing files, canonicalizes and verifies containment. For new files,
/// returns the joined path directly (the name has already been validated).
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_resource_path(
    dir: &std::path::Path,
    name: &str,
    ext: &str,
) -> Result<PathBuf, axum::response::Response> {
    let root = VerifiedRoot::new(dir).map_err(|e| e.into_response())?;
    let file_path = root.as_path().join(format!("{}.{}", name, ext));
    if file_path.exists() {
        let safe = super::super::safe_path::SafePath::resolve(&file_path, &root)
            .map_err(|e| e.into_response())?;
        Ok(safe.as_path().to_path_buf())
    } else {
        Ok(file_path)
    }
}

/// Creates a configured directory, before anything canonicalizes it.
///
/// Ordering matters: [`resolve_resource_path`] verifies the root by
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
) -> Result<(), axum::response::Response> {
    let owned = dir.to_path_buf();
    let project = project_dir(state);
    let outcome =
        tokio::task::spawn_blocking(move || crate::util::create_dir_within(&owned, &project)).await;

    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let status = match e {
                crate::util::CreateWithinError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::BAD_REQUEST,
            };
            Err((status, Json(json!({"error": e.to_string()}))).into_response())
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create directory: {e}")})),
        )
            .into_response()),
    }
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
    let label = label.to_string();
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to {}: {}", label, e)})),
        )
            .into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("{} task failed: {}", label, e)})),
        )
            .into_response()),
    }
}
