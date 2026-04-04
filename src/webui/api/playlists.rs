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
use super::helpers::{require_configured_dir, resolve_resource_path, validate_resource_name};
use crate::config;

// ---------------------------------------------------------------------------
// Playlist CRUD endpoints
// ---------------------------------------------------------------------------

/// Validates a playlist name for use in file paths.
#[allow(clippy::result_large_err)]
fn validate_playlist_name(name: &str) -> Result<(), axum::response::Response> {
    validate_resource_name(name, "playlist", Some("all_songs"))
}

/// GET /api/playlists — list all playlists with name, song count, and active status.
pub(super) async fn get_playlists(State(state): State<WebUiState>) -> impl IntoResponse {
    let playlists = state.player.list_playlists();
    let active = {
        let active_playlist = state.player.get_playlist();
        active_playlist.name().to_string()
    };
    let items: Vec<serde_json::Value> = playlists
        .iter()
        .map(|name| {
            let playlist_map = state.player.playlists_snapshot();
            let song_count = playlist_map.get(name).map(|p| p.songs().len()).unwrap_or(0);
            json!({
                "name": name,
                "song_count": song_count,
                "is_active": name == &active,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!(items))).into_response()
}

/// GET /api/playlists/:name — get a playlist's songs and available songs.
pub(super) async fn get_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let playlists = state.player.playlists_snapshot();
    let Some(pl) = playlists.get(&name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Playlist '{}' not found", name)})),
        )
            .into_response();
    };
    let all_songs: Vec<String> = state
        .player
        .songs()
        .sorted_list()
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "name": name,
            "songs": pl.songs(),
            "available_songs": all_songs,
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct PlaylistBody {
    songs: Vec<String>,
}

/// PUT /api/playlists/:name — create or update a playlist.
pub(super) async fn put_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Json(body): Json<PlaylistBody>,
) -> impl IntoResponse {
    validate_playlist_name(&name)?;
    let playlists_dir =
        require_configured_dir(&state.playlists_dir, "playlists", StatusCode::NOT_FOUND)?;

    // Write the playlist YAML file.
    let playlist_config = config::Playlist::new(&body.songs);
    let yaml = match crate::util::to_yaml_string(&playlist_config) {
        Ok(y) => y,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to serialize playlist: {}", e)})),
            )
                .into_response());
        }
    };

    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in resolve_resource_path.
    let file_path = resolve_resource_path(&playlists_dir, &name, "yaml")?;

    // Ensure directory exists and write atomically, off the async runtime.
    let dir = playlists_dir.clone();
    let fp = file_path.clone();
    let yaml_owned = yaml;
    super::helpers::spawn_blocking_io("write playlist", move || {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        config_io::atomic_write(&fp, &yaml_owned)
    })
    .await?;

    // Reload songs to pick up the new playlist.
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "name": name})),
        )
            .into_response(),
    )
}

/// DELETE /api/playlists/:name — delete a playlist.
pub(super) async fn delete_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    validate_playlist_name(&name)?;
    let playlists_dir =
        require_configured_dir(&state.playlists_dir, "playlists", StatusCode::NOT_FOUND)?;

    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in resolve_resource_path.
    let file_path = resolve_resource_path(&playlists_dir, &name, "yaml")?;
    let yml_path = resolve_resource_path(&playlists_dir, &name, "yml")?;

    // Determine which file to delete, then remove it off the async runtime.
    let target = if file_path.is_file() {
        file_path
    } else if yml_path.is_file() {
        yml_path
    } else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Playlist '{}' not found", name)})),
        )
            .into_response());
    };
    super::helpers::spawn_blocking_io("delete playlist", move || std::fs::remove_file(&target))
        .await?;

    // Reload songs to remove the playlist.
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "deleted", "name": name})),
        )
            .into_response(),
    )
}

/// POST /api/playlists/:name/activate — switch the active playlist.
pub(super) async fn activate_playlist(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.player.switch_to_playlist(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"status": "activated", "name": name})),
        )
            .into_response(),
        Err(e) => {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            (status, Json(json!({"error": e}))).into_response()
        }
    }
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_playlists_returns_list() {
        let (state, _dir) = test_state();
        // Create a playlist file with an empty songs list so it loads without
        // requiring actual song directories on disk.
        let playlists_dir = state.playlists_dir.clone().unwrap();
        std::fs::write(playlists_dir.join("mylist.yaml"), "songs: []\n").unwrap();
        // Reload songs so the player picks up the new playlist.
        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlists")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        // Should contain at least the playlist we created.
        assert!(arr.iter().any(|v| v["name"] == "mylist"));
    }

    #[tokio::test]
    async fn get_playlist_by_name_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlists/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_playlist_by_name_creates_file() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir.clone().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlists/mylist")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"songs": ["Song A"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(playlists_dir.join("mylist.yaml").exists());
    }

    #[tokio::test]
    async fn put_playlist_by_name_invalid_name_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlists/all_songs")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"songs": ["Song A"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_playlist_by_name_path_traversal_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlists/..%2Fevil")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"songs": ["Song A"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_playlist_by_name_success() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir.clone().unwrap();
        // Create a playlist file first.
        std::fs::write(
            playlists_dir.join("mylist.yaml"),
            "songs:\n  - \"Song A\"\n",
        )
        .unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/playlists/mylist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!playlists_dir.join("mylist.yaml").exists());
    }

    #[tokio::test]
    async fn delete_playlist_by_name_all_songs_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/playlists/all_songs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_playlist_by_name_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/playlists/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn activate_playlist_success() {
        let (state, _dir) = test_state();
        // "all_songs" is always present in the player's playlists map.
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/playlists/all_songs/activate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn activate_playlist_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/playlists/nonexistent/activate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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

    #[tokio::test]
    async fn get_playlist_by_name_success() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir.clone().unwrap();
        // Use an empty song list so the playlist loads without requiring actual songs on disk.
        std::fs::write(playlists_dir.join("testlist.yaml"), "songs: []\n").unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlists/testlist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["name"], "testlist");
        assert!(parsed["songs"].is_array());
        assert!(parsed["available_songs"].is_array());
    }

    #[tokio::test]
    async fn get_playlist_by_name_yml_extension() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir.clone().unwrap();
        // Write a .yml file (not .yaml) with an empty song list.
        std::fs::write(playlists_dir.join("ymllist.yml"), "songs: []\n").unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlists/ymllist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["name"], "ymllist");
    }

    #[tokio::test]
    async fn get_playlists_with_active() {
        let (state, _dir) = test_state();
        // all_songs should be the active playlist by default.
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlists")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let arr = parsed.as_array().unwrap();
        // Find the all_songs entry and verify it's marked active.
        let all_songs = arr
            .iter()
            .find(|v| v["name"] == "all_songs")
            .expect("all_songs should be present");
        assert_eq!(all_songs["is_active"], true);
    }
}
