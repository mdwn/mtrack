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

//! The web server's route for the lighting asset store: the rig models,
//! meshes, thumbnails and scenery the stage layout and the fixture pages
//! name, which an `<img>` and the 3D loader fetch by URL.
//!
//! `GET /lighting-assets/{*path}` is not behind the lock (a read, as every
//! `GET` always was), and is on the web server only; the gRPC port has
//! `GetLightingAsset` for a client that cannot fetch over HTTP.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde_json::json;

use super::api::lighting_api::project_root;
use super::server::WebUiState;
use crate::lighting;

/// The route.
pub fn router() -> Router<WebUiState> {
    Router::new().route("/lighting-assets/{*path}", get(get_lighting_asset))
}

/// `GET /lighting-assets/{*path}` — serves a file of the project's
/// asset store (`lighting/.cache/assets/`, design §16.2): rig models,
/// meshes and thumbnails for the 3D view. Paths are content-addressed, so
/// a hit is immutable and cached as such.
async fn get_lighting_asset(
    State(state): State<WebUiState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    use lighting::asset_store::{read_asset, AssetError};

    let project = project_root(&state.config_path)?;
    let gone =
        |error: &str| (StatusCode::NOT_FOUND, Json(json!({ "error": error }))).into_response();
    let asset = tokio::task::spawn_blocking(move || read_asset(&project, &path))
        .await
        .map_err(|_| gone("Asset not found"))?
        .map_err(|e| match e {
            AssetError::NotAnAsset => gone("Not an asset type"),
            // No store yet (no referential fixture type has expanded) is a
            // plain 404, not a server error.
            AssetError::NoStore => gone("No asset store"),
            AssetError::Path(e) => e.into_response(),
            AssetError::Unreadable => gone("Asset not found"),
        })?;
    let (bytes, content_type) = (asset.bytes, asset.content_type);
    let len = bytes.len();

    // The bytes are a stranger's (copied out of a GDTF archive), so the
    // type is not to be sniffed, and an SVG — which may script — is served
    // as a picture only: no scripts, no fetches, sandboxed if navigated to.
    let mut response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .header(axum::http::header::CONTENT_LENGTH, len)
        .header(axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            axum::http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable",
        );
    if content_type == "image/svg+xml" {
        response = response.header(
            axum::http::header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; sandbox",
        );
    }
    Ok::<_, axum::response::Response>(
        response
            .body(axum::body::Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webui::api::test_helpers::{response_body, test_state};
    use axum::body::Body;
    use tower::ServiceExt;

    #[tokio::test]
    async fn assets_are_served_from_the_store_with_containment() {
        let (state, dir) = test_state();
        let store = dir.path().join("lighting/.cache/assets/abc123");
        std::fs::create_dir_all(store.join("models")).unwrap();
        std::fs::write(store.join("rig-1-v1.json"), "{\"version\":1}").unwrap();
        std::fs::write(store.join("models/yoke.glb"), b"glTF").unwrap();
        std::fs::write(store.join("thumbnail.svg"), "<svg><script/></svg>").unwrap();
        std::fs::write(store.join("notes.txt"), "no").unwrap();
        // Something outside the store that a traversal would reach.
        std::fs::write(dir.path().join("lighting/secret.json"), "{}").unwrap();

        let get = |uri: String| {
            let app = router().with_state(state.clone());
            async move {
                app.oneshot(
                    http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };

        let response = get("/lighting-assets/abc123/rig-1-v1.json".to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "application/json",
            "json is served as json"
        );
        assert!(response.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable"));
        assert_eq!(response_body(response).await, "{\"version\":1}");

        let response = get("/lighting-assets/abc123/models/yoke.glb".to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "model/gltf-binary");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(response.headers().get("content-security-policy").is_none());

        let response = get("/lighting-assets/abc123/thumbnail.svg".to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/svg+xml");
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(
            csp.contains("default-src 'none'") && csp.contains("sandbox"),
            "{csp}"
        );

        // Not an asset type, missing, a directory, and a traversal.
        for (uri, why) in [
            (
                "/lighting-assets/abc123/notes.txt",
                "text is not an asset type",
            ),
            ("/lighting-assets/abc123/models/head.glb", "missing"),
            ("/lighting-assets/abc123/models", "a directory"),
            ("/lighting-assets/../secret.json", "traversal"),
            ("/lighting-assets/abc123/../../secret.json", "traversal"),
        ] {
            let response = get(uri.to_string()).await;
            assert!(
                response.status() == StatusCode::NOT_FOUND
                    || response.status() == StatusCode::BAD_REQUEST
                    || response.status() == StatusCode::FORBIDDEN,
                "{uri} ({why}): {}",
                response.status()
            );
        }
    }

    #[tokio::test]
    async fn a_project_without_a_store_answers_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting-assets/abc/rig.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
