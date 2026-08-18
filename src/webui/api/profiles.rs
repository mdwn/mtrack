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

use super::super::config_io;
use super::super::server::WebUiState;
use tracing::warn;

use super::config_api::{reject_if_playing, reload_hardware_after_mutation};
use super::helpers::{
    require_configured_dir, resolve_resource_path, spawn_blocking_io, validate_resource_name,
};
use crate::config::Profile;
use config::Config;

/// Validates a profile filename for use in file paths.
#[allow(clippy::result_large_err)]
fn validate_profile_filename(name: &str) -> Result<(), axum::response::Response> {
    validate_resource_name(name, "profile", None)
}

/// GET /api/profiles — list profile files from profiles_dir.
pub(super) async fn get_profiles(State(state): State<WebUiState>) -> impl IntoResponse {
    let profiles_dir = require_configured_dir(
        &state.profiles_dir,
        "profiles",
        StatusCode::SERVICE_UNAVAILABLE,
    )?;

    // codeql[rust/path-injection] profiles_dir comes from server config, not user input.
    let result = spawn_blocking_io("read profiles dir", move || {
        let entries = std::fs::read_dir(&profiles_dir)?;
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
                    "has_trigger": profile.trigger().is_some(),
                    "has_controllers": !profile.controllers().is_empty(),
                }),
            ));
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        Ok::<_, std::io::Error>(items.into_iter().map(|(_, v)| v).collect::<Vec<_>>())
    })
    .await?;
    Ok::<_, axum::response::Response>((StatusCode::OK, Json(json!(result))).into_response())
}

/// GET /api/profiles/:filename — read a single profile file.
pub(super) async fn get_profile(
    State(state): State<WebUiState>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    validate_profile_filename(&filename)?;
    let profiles_dir = require_configured_dir(
        &state.profiles_dir,
        "profiles",
        StatusCode::SERVICE_UNAVAILABLE,
    )?;

    // Try .yaml then .yml.
    let file_path = {
        let yaml_path = resolve_resource_path(&profiles_dir, &filename, "yaml")?;
        if yaml_path.is_file() {
            yaml_path
        } else {
            let yml_path = resolve_resource_path(&profiles_dir, &filename, "yml")?;
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

    // codeql[rust/path-injection] file_path is validated via resolve_resource_path.
    let fp = file_path.clone();
    let (raw, profile) = spawn_blocking_io("read profile", move || {
        let raw =
            std::fs::read_to_string(&fp).map_err(|e| format!("Failed to read profile: {}", e))?;
        let profile: Profile = Config::builder()
            .add_source(config::File::from(fp.as_path()))
            .build()
            .and_then(|c| c.try_deserialize())
            .map_err(|e| format!("Failed to parse profile: {}", e))?;
        Ok::<_, String>((raw, profile))
    })
    .await?;

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
    let profiles_dir = require_configured_dir(
        &state.profiles_dir,
        "profiles",
        StatusCode::SERVICE_UNAVAILABLE,
    )?;

    // Validate that the body deserializes as a Profile.
    let profile: Profile = serde_json::from_value(body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid profile: {}", e)})),
        )
            .into_response()
    })?;

    if let Err(errors) = profile.validate() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response());
    }

    let yaml = crate::util::to_yaml_string(&profile).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to serialize profile: {}", e)})),
        )
            .into_response()
    })?;

    // Before `resolve_resource_path`, which canonicalizes the directory and
    // fails if it is missing -- a creation after that call can never run.
    // Created only when it is inside the project: a `profiles_dir:` pointing
    // elsewhere is the operator's to set up.
    super::helpers::ensure_configured_dir(&profiles_dir, &state).await?;

    // codeql[rust/path-injection] filename is validated; path is verified via resolve_resource_path.
    let file_path = resolve_resource_path(&profiles_dir, &filename, "yaml")?;

    // Write the file off the async runtime.
    let fp = file_path;
    let yaml_owned = yaml;
    spawn_blocking_io("write profile", move || {
        config_io::staged_write(&fp, &yaml_owned)
    })
    .await?;

    // The store's copy is what the reload re-initialises from, and writing the
    // file did not touch it. Without this the save is acknowledged and then
    // ignored until restart.
    // A failure here is not cosmetic: the reload below would re-initialise from
    // the boot-time copy and the save would be silently ignored, which is the
    // whole defect this call exists to close. `Player::deserialize` validates
    // the entire config, so one unrelated bad file in the directory is enough
    // to cause it — the caller has to be told rather than left with a 200.
    if let Some(store) = state.player.config_store() {
        if let Err(e) = store.reload_from_disk().await {
            warn!("Config reload after profile write failed: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!(
                        "the profile was written but the running config could not be \
                         reloaded, so it will not take effect until restart: {e}"
                    )
                })),
            )
                .into_response());
        }
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
    let profiles_dir = require_configured_dir(
        &state.profiles_dir,
        "profiles",
        StatusCode::SERVICE_UNAVAILABLE,
    )?;

    // codeql[rust/path-injection] filename is validated; path is verified via resolve_resource_path.
    let file_path = resolve_resource_path(&profiles_dir, &filename, "yaml")?;
    let yml_path = resolve_resource_path(&profiles_dir, &filename, "yml")?;

    let target = if file_path.is_file() {
        file_path
    } else if yml_path.is_file() {
        yml_path
    } else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Profile '{}' not found", filename)})),
        )
            .into_response());
    };
    spawn_blocking_io("delete profile", move || std::fs::remove_file(&target)).await?;

    // The store's copy is what the reload re-initialises from, and writing the
    // file did not touch it. Without this the save is acknowledged and then
    // ignored until restart.
    // A failure here is not cosmetic: the reload below would re-initialise from
    // the boot-time copy and the save would be silently ignored, which is the
    // whole defect this call exists to close. `Player::deserialize` validates
    // the entire config, so one unrelated bad file in the directory is enough
    // to cause it — the caller has to be told rather than left with a 200.
    if let Some(store) = state.player.config_store() {
        if let Err(e) = store.reload_from_disk().await {
            warn!("Config reload after profile write failed: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!(
                        "the profile was deleted but the running config could not be \
                         reloaded, so it will not take effect until restart: {e}"
                    )
                })),
            )
                .into_response());
        }
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

    /// Saving a profile file must reach the config the player reloads from.
    ///
    /// `Player::deserialize` copies `profiles_dir` files into the in-memory
    /// config at startup, and `reload_hardware` re-initialises from that copy.
    /// Writing the file alone left the save acknowledged with a 200, followed
    /// by a hardware reload that rebuilt everything from the profile as it was
    /// at boot — so a trigger disabled through the editor stayed live until the
    /// process restarted, which is what #380 was reported as.
    #[tokio::test]
    async fn put_profile_updates_the_config_the_reload_reads() {
        let (mut state, dir) = test_state_with_store();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        std::fs::write(
            profiles_dir.join("rig.yaml"),
            "hostname: rig\naudio:\n  device: dev-x\n  track_mappings:\n    drums: [1]\n",
        )
        .unwrap();
        std::fs::write(&state.config_path, "songs: songs\nprofiles_dir: profiles\n").unwrap();
        state.profiles_dir = Some(profiles_dir.clone());

        // Reload so the store starts from what is on disk, the way startup does.
        let store = state.player.config_store().expect("store");
        store.reload_from_disk().await.expect("initial load");
        let loaded = store.read_config().await;
        assert_eq!(
            loaded.profile_list().unwrap_or_default().len(),
            1,
            "the profile file should be in the store to begin with"
        );
        drop(loaded);

        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/profiles/rig")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"hostname": "rig", "audio": {"device": "dev-changed", "track_mappings": {"drums": [1]}}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // The store — not just the file — must reflect the edit, because that
        // is what the hardware reload reads.
        let after = store.read_config().await;
        let profiles = after.profile_list().unwrap_or_default();
        let device = profiles
            .first()
            .and_then(|p| p.audio_config())
            .map(|ac| ac.audio().device().to_string());
        assert_eq!(
            device.as_deref(),
            Some("dev-changed"),
            "the reload would re-initialise from the pre-save profile"
        );
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
