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
use super::lighting_api::{content_version, if_match_version};
use crate::config::Profile;
use config::Config;

/// Validates a profile filename for use in file paths.
/// Serialises every read-check-write of a profile file.
static PROFILE_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A YAML document as JSON, for merging. `None` when it does not parse.
fn yaml_to_json(text: &str) -> Option<serde_json::Value> {
    use yaml_rust2::{Yaml, YamlLoader};
    fn convert(y: &Yaml) -> serde_json::Value {
        match y {
            Yaml::Null | Yaml::BadValue | Yaml::Alias(_) => serde_json::Value::Null,
            Yaml::Boolean(b) => (*b).into(),
            Yaml::Integer(i) => (*i).into(),
            Yaml::Real(r) => r
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map_or_else(|| r.clone().into(), serde_json::Value::Number),
            Yaml::String(s) => s.clone().into(),
            Yaml::Array(a) => a.iter().map(convert).collect(),
            Yaml::Hash(h) => h
                .iter()
                .filter_map(|(k, v)| {
                    let key = match k {
                        Yaml::String(s) => s.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        Yaml::Real(r) => r.clone(),
                        Yaml::Boolean(b) => b.to_string(),
                        _ => return None,
                    };
                    Some((key, convert(v)))
                })
                .collect(),
        }
    }
    YamlLoader::load_from_str(text).ok()?.first().map(convert)
}

/// Copies into `new` the keys of the existing profile file that the typed
/// profile does not know (a key this version does not model, or one added by
/// hand), so a save from the UI does not silently drop them. Keys are
/// compared case-insensitively, as the profile loader reads them. Only
/// mappings are merged; list items are rewritten as the UI sent them.
/// Comments are not carried: the YAML crate does not keep them.
fn carry_unknown_keys(path: &std::path::Path, old_bytes: &[u8], new: &mut serde_json::Value) {
    let Some(raw) = std::str::from_utf8(old_bytes).ok().and_then(yaml_to_json) else {
        return;
    };
    let typed = Config::builder()
        .add_source(config::File::from(path))
        .build()
        .and_then(|c| c.try_deserialize::<Profile>())
        .ok()
        .and_then(|p| serde_json::to_value(&p).ok());
    let Some(typed) = typed else { return };
    carry(&raw, &typed, new);
}

fn carry(raw: &serde_json::Value, typed: &serde_json::Value, new: &mut serde_json::Value) {
    let (Some(raw), Some(typed), Some(new)) =
        (raw.as_object(), typed.as_object(), new.as_object_mut())
    else {
        return;
    };
    for (key, value) in raw {
        let lower = key.to_lowercase();
        match typed.iter().find(|(k, _)| k.to_lowercase() == lower) {
            None => {
                if !new.keys().any(|k| k.to_lowercase() == lower) {
                    new.insert(key.clone(), value.clone());
                }
            }
            Some((typed_key, typed_value)) => {
                if let Some(slot) = new.get_mut(typed_key) {
                    carry(value, typed_value, slot);
                }
            }
        }
    }
}

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
            Json(json!({
                "profile": profile_json,
                "yaml": raw,
                "version": content_version(raw.as_bytes()),
            })),
        )
            .into_response(),
    )
}

/// PUT /api/profiles/:filename — create or update a profile file.
pub(super) async fn put_profile(
    State(state): State<WebUiState>,
    Path(filename): Path<String>,
    headers: axum::http::HeaderMap,
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

    let new_json = serde_json::to_value(&profile).map_err(|e| {
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
    let profiles_dir = super::helpers::ensure_configured_dir(&profiles_dir, &state).await?;

    // The file the profile already lives in (`.yaml`, else `.yml`, as the
    // read does), so a save never leaves two files for one profile.
    // codeql[rust/path-injection] filename is validated; path is verified via resolve_resource_path.
    let yaml_path = resolve_resource_path(&profiles_dir, &filename, "yaml")?;
    let file_path = if yaml_path.is_file() {
        yaml_path
    } else {
        let yml_path = resolve_resource_path(&profiles_dir, &filename, "yml")?;
        if yml_path.is_file() {
            yml_path
        } else {
            yaml_path
        }
    };

    // Write the file off the async runtime. The version check and the write
    // happen under one lock so two saves cannot both pass the check.
    let expected = if_match_version(&headers);
    let fp = file_path;
    let outcome = spawn_blocking_io("write profile", move || {
        let _guard = PROFILE_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::fs::read(&fp).ok();
        if let Some(expected) = &expected {
            let current = old.as_deref().map(content_version);
            if current.as_deref() != Some(expected.as_str()) {
                return Ok::<_, String>(Err(current));
            }
        }
        let mut merged = new_json;
        if let Some(old) = &old {
            carry_unknown_keys(&fp, old, &mut merged);
        }
        let yaml = crate::util::to_yaml_string(&merged).map_err(|e| e.to_string())?;
        config_io::staged_write(&fp, &yaml)?;
        Ok(Ok(content_version(yaml.as_bytes())))
    })
    .await?;
    let version = match outcome {
        Ok(version) => version,
        Err(current) => {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!(
                        "profile \"{filename}\" changed since you loaded it (another tab or \
                         page); it has been reloaded, so reapply your change"
                    ),
                    "conflict": true,
                    "version": current,
                })),
            )
                .into_response());
        }
    };

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
            Json(json!({"status": "saved", "filename": filename, "version": version})),
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

    const HOST_A: &str =
        "hostname: host-a\naudio:\n  device: dev-a\n  track_mappings:\n    drums: [1]\n";

    async fn put_host_a(
        app: &axum::Router,
        version: Option<&str>,
        device: &str,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = http::Request::builder()
            .method("PUT")
            .uri("/profiles/host-a")
            .header("content-type", "application/json");
        if let Some(v) = version {
            req = req.header("if-match", v);
        }
        let body = serde_json::json!({
            "hostname": "host-a",
            "audio": {"device": device, "track_mappings": {"drums": [1]}}
        });
        let response = app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    #[tokio::test]
    async fn a_stale_profile_version_is_refused_and_the_current_one_saves() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        write_profile_file(&profiles_dir, "host-a.yaml", HOST_A);
        state.profiles_dir = Some(profiles_dir.clone());
        let app = router().with_state(state);

        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri("/profiles/host-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let got: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        let v1 = got["version"].as_str().unwrap().to_string();

        let (status, saved) = put_host_a(&app, Some(&v1), "dev-b").await;
        assert_eq!(status, StatusCode::OK);
        let v2 = saved["version"].as_str().unwrap().to_string();
        let after_first = std::fs::read_to_string(profiles_dir.join("host-a.yaml")).unwrap();
        assert!(after_first.contains("dev-b"));

        let (status, body) = put_host_a(&app, Some(&v1), "dev-c").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["version"].as_str().unwrap(), v2);
        assert_eq!(
            std::fs::read_to_string(profiles_dir.join("host-a.yaml")).unwrap(),
            after_first
        );

        let (status, _) = put_host_a(&app, Some(&v2), "dev-c").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn saving_a_profile_keeps_keys_the_editor_does_not_know() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        write_profile_file(
            &profiles_dir,
            "host-a.yaml",
            "hostname: host-a\nvenue_notes: front of house\naudio:\n  device: dev-a\n  house_tweak: 3\n  track_mappings:\n    drums: [1]\n",
        );
        state.profiles_dir = Some(profiles_dir.clone());
        let app = router().with_state(state);

        let (status, _) = put_host_a(&app, None, "dev-b").await;
        assert_eq!(status, StatusCode::OK);
        let text = std::fs::read_to_string(profiles_dir.join("host-a.yaml")).unwrap();
        assert!(text.contains("dev-b"), "{text}");
        assert!(text.contains("venue_notes"), "{text}");
        assert!(text.contains("house_tweak"), "{text}");
    }

    #[tokio::test]
    async fn a_yml_profile_is_saved_in_place() {
        let (mut state, dir) = test_state();
        let profiles_dir = dir.path().join("profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        write_profile_file(&profiles_dir, "host-a.yml", HOST_A);
        state.profiles_dir = Some(profiles_dir.clone());
        let app = router().with_state(state);

        let (status, _) = put_host_a(&app, None, "dev-b").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!profiles_dir.join("host-a.yaml").exists());
        assert!(std::fs::read_to_string(profiles_dir.join("host-a.yml"))
            .unwrap()
            .contains("dev-b"));
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
