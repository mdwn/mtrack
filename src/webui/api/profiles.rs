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

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use std::path::PathBuf;

use super::super::config_io;
use super::super::server::WebUiState;
use super::config_api::{reject_if_playing, reload_hardware_after_mutation};
use crate::config::Profile;
use config::Config;

/// Validates a profile filename for use in file paths.
#[allow(clippy::result_large_err)]
fn validate_profile_filename(name: &str) -> Result<(), axum::response::Response> {
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid profile filename"})),
        )
            .into_response());
    }
    Ok(())
}

/// Returns the profiles directory, or an error response if not configured.
#[allow(clippy::result_large_err)]
fn require_profiles_dir(state: &WebUiState) -> Result<PathBuf, axum::response::Response> {
    state.profiles_dir.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "No profiles directory configured"})),
        )
            .into_response()
    })
}

/// Resolves a profile file path within the profiles directory, verifying
/// that the result does not escape the directory via symlinks or other tricks.
#[allow(clippy::result_large_err)]
fn resolve_profile_path(
    profiles_dir: &std::path::Path,
    name: &str,
    ext: &str,
) -> Result<PathBuf, axum::response::Response> {
    let dir_canonical = profiles_dir.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to resolve profiles dir: {}", e)})),
        )
            .into_response()
    })?;
    let file_path = dir_canonical.join(format!("{}.{}", name, ext));
    if file_path.exists() {
        let canonical = file_path.canonicalize().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve path: {}", e)})),
            )
                .into_response()
        })?;
        if !canonical.starts_with(&dir_canonical) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid profile path"})),
            )
                .into_response());
        }
        Ok(canonical)
    } else {
        let parent = file_path.parent().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid profile path"})),
            )
                .into_response()
        })?;
        if parent != dir_canonical {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid profile path"})),
            )
                .into_response());
        }
        Ok(file_path)
    }
}

/// GET /api/profiles — list profile files from profiles_dir.
pub(super) async fn get_profiles(State(state): State<WebUiState>) -> impl IntoResponse {
    let profiles_dir = require_profiles_dir(&state)?;

    // codeql[rust/path-injection] profiles_dir comes from server config, not user input.
    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(e) => e,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to read profiles dir: {}", e)})),
            )
                .into_response());
        }
    };

    let mut items: Vec<(String, serde_json::Value)> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" {
            continue;
        }
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Parse the profile; skip unparseable files.
        let profile = match Config::builder()
            .add_source(config::File::from(path.as_path()))
            .build()
            .and_then(|c| c.try_deserialize::<Profile>())
        {
            Ok(p) => p,
            Err(_) => continue,
        };

        items.push((
            filename.clone(),
            json!({
                "filename": filename,
                "hostname": profile.hostname(),
                "has_audio": profile.audio_config().is_some(),
                "has_midi": profile.midi().is_some(),
                "has_dmx": profile.dmx().is_some(),
            }),
        ));
    }

    items.sort_by(|a, b| a.0.cmp(&b.0));
    let result: Vec<serde_json::Value> = items.into_iter().map(|(_, v)| v).collect();
    Ok::<_, axum::response::Response>((StatusCode::OK, Json(json!(result))).into_response())
}

/// GET /api/profiles/:filename — read a single profile file.
pub(super) async fn get_profile(
    State(state): State<WebUiState>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    validate_profile_filename(&filename)?;
    let profiles_dir = require_profiles_dir(&state)?;

    // Try .yaml then .yml.
    let file_path = {
        let yaml_path = resolve_profile_path(&profiles_dir, &filename, "yaml")?;
        if yaml_path.is_file() {
            yaml_path
        } else {
            let yml_path = resolve_profile_path(&profiles_dir, &filename, "yml")?;
            if yml_path.is_file() {
                yml_path
            } else {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("Profile '{}' not found", filename)})),
                )
                    .into_response());
            }
        }
    };

    // codeql[rust/path-injection] file_path is validated via resolve_profile_path.
    let raw = std::fs::read_to_string(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to read profile: {}", e)})),
        )
            .into_response()
    })?;

    let profile: Profile = Config::builder()
        .add_source(config::File::from(file_path.as_path()))
        .build()
        .and_then(|c| c.try_deserialize())
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to parse profile: {}", e)})),
            )
                .into_response()
        })?;

    let profile_json = serde_json::to_value(&profile).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to serialize profile: {}", e)})),
        )
            .into_response()
    })?;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"profile": profile_json, "yaml": raw})),
        )
            .into_response(),
    )
}

/// PUT /api/profiles/:filename — create or update a profile file.
pub(super) async fn put_profile(
    State(state): State<WebUiState>,
    Path(filename): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    validate_profile_filename(&filename)?;
    if let Some(resp) = reject_if_playing(&state).await {
        return Err(resp);
    }
    let profiles_dir = require_profiles_dir(&state)?;

    // Validate that the body deserializes as a Profile.
    let profile: Profile = serde_json::from_value(body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid profile: {}", e)})),
        )
            .into_response()
    })?;

    let yaml = crate::util::to_yaml_string(&profile).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to serialize profile: {}", e)})),
        )
            .into_response()
    })?;

    // codeql[rust/path-injection] profiles_dir comes from server config, not user input.
    if let Err(e) = std::fs::create_dir_all(&profiles_dir) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create profiles directory: {}", e)})),
        )
            .into_response());
    }

    // codeql[rust/path-injection] filename is validated; path is verified via resolve_profile_path.
    let file_path = resolve_profile_path(&profiles_dir, &filename, "yaml")?;
    if let Err(e) = config_io::atomic_write(&file_path, &yaml) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response());
    }

    reload_hardware_after_mutation(&state).await;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "filename": filename})),
        )
            .into_response(),
    )
}

/// DELETE /api/profiles/:filename — delete a profile file.
pub(super) async fn delete_profile_file(
    State(state): State<WebUiState>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    validate_profile_filename(&filename)?;
    if let Some(resp) = reject_if_playing(&state).await {
        return Err(resp);
    }
    let profiles_dir = require_profiles_dir(&state)?;

    // codeql[rust/path-injection] filename is validated; path is verified via resolve_profile_path.
    let file_path = resolve_profile_path(&profiles_dir, &filename, "yaml")?;
    if !file_path.is_file() {
        // Also check .yml extension.
        let yml_path = resolve_profile_path(&profiles_dir, &filename, "yml")?;
        if yml_path.is_file() {
            std::fs::remove_file(&yml_path).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to delete file: {}", e)})),
                )
                    .into_response()
            })?;
        } else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Profile '{}' not found", filename)})),
            )
                .into_response());
        }
    } else {
        std::fs::remove_file(&file_path).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to delete file: {}", e)})),
            )
                .into_response()
        })?;
    }

    reload_hardware_after_mutation(&state).await;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "deleted", "filename": filename})),
        )
            .into_response(),
    )
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    fn write_profile_file(dir: &std::path::Path, filename: &str, content: &str) {
        std::fs::write(dir.join(filename), content).unwrap();
    }

    #[tokio::test]
    async fn get_profiles_lists_files() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        write_profile_file(
            &profiles_dir,
            "01-host-a.yaml",
            "hostname: host-a\naudio:\n  device: dev-a\n  track_mappings:\n    drums: [1]\n",
        );
        write_profile_file(
            &profiles_dir,
            "02-host-b.yml",
            "hostname: host-b\nmidi:\n  device: midi-b\n",
        );
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/profiles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["filename"], "01-host-a");
        assert_eq!(arr[0]["hostname"], "host-a");
        assert_eq!(arr[0]["has_audio"], true);
        assert_eq!(arr[1]["filename"], "02-host-b");
        assert_eq!(arr[1]["hostname"], "host-b");
        assert_eq!(arr[1]["has_midi"], true);
    }

    #[tokio::test]
    async fn get_profiles_empty_dir() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/profiles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_profiles_no_dir_configured() {
        let (state, _dir) = test_state();
        // profiles_dir is already None
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/profiles")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn get_profile_by_filename() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        write_profile_file(
            &profiles_dir,
            "host-a.yaml",
            "hostname: host-a\naudio:\n  device: dev-a\n  track_mappings:\n    drums: [1]\n",
        );
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/profiles/host-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["profile"]["hostname"].as_str().unwrap() == "host-a");
        assert!(parsed["yaml"].as_str().unwrap().contains("host-a"));
    }

    #[tokio::test]
    async fn get_profile_not_found() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/profiles/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_profile_creates_file() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir.clone());
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/profiles/new-host")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"hostname": "new-host", "audio": {"device": "dev-x", "track_mappings": {"drums": [1]}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(profiles_dir.join("new-host.yaml").exists());
    }

    #[tokio::test]
    async fn put_profile_validates() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        // Invalid JSON body — controllers should be an array, not a string.
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/profiles/bad")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"controllers": "not-an-array"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_profile_removes_file() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        write_profile_file(
            &profiles_dir,
            "host-a.yaml",
            "hostname: host-a\naudio:\n  device: dev-a\n  track_mappings:\n    drums: [1]\n",
        );
        state.profiles_dir = Some(profiles_dir.clone());
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/profiles/host-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!profiles_dir.join("host-a.yaml").exists());
    }

    #[tokio::test]
    async fn delete_profile_not_found() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/profiles/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_profile_path_traversal_rejected() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/profiles/..%2Fevil")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"hostname": "evil"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_profile_path_traversal_rejected() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        state.profiles_dir = Some(profiles_dir);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/profiles/..%2Fevil")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
