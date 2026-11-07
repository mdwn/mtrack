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
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::safe_path::{SafePath, VerifiedRoot};
use super::super::server::WebUiState;
use crate::songs;
use crate::webui::wire::songs::{Browse, BrowseEntry, BulkImport, FileKind, ImportFailure};

/// Resolves the project root (parent of config_path) as a VerifiedRoot.
fn project_root(state: &WebUiState) -> Result<VerifiedRoot, Box<axum::response::Response>> {
    let config_canonical = state.config_path.canonicalize().map_err(|e| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve config path: {}", e)})),
            )
                .into_response(),
        )
    })?;
    let parent = config_canonical.parent().ok_or_else(|| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Unable to determine config root directory"})),
            )
                .into_response(),
        )
    })?;
    VerifiedRoot::new(parent).map_err(|e| Box::new(e.into_response()))
}

/// Resolves a project-relative path (e.g. "/songs") under the project root.
fn resolve_browse_path(
    root: &VerifiedRoot,
    path: &str,
) -> Result<SafePath, Box<axum::response::Response>> {
    if path.is_empty() || path == "/" {
        return Ok(root.as_safe_path());
    }
    let relative = path.strip_prefix('/').unwrap_or(path);
    SafePath::resolve(&root.as_path().join(relative), root).map_err(|_| {
        Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Not a directory: {}", path)})),
            )
                .into_response(),
        )
    })
}

/// GET /api/browse?path=... — lists files and directories at a filesystem path.
///
/// Restricted to the directory containing mtrack.yaml (the config root).
/// If no `path` query parameter is provided, defaults to the config root.
/// Returns entries sorted: directories first, then files alphabetically.
pub(super) async fn browse_directory(
    State(state): State<WebUiState>,
    Query(params): Query<BrowseParams>,
) -> impl IntoResponse {
    let root = match project_root(&state) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    let dir_safe = match resolve_browse_path(&root, &params.path) {
        Ok(p) => p,
        Err(e) => return *e,
    };
    if !dir_safe.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Not a directory: {}", params.path)})),
        )
            .into_response();
    }

    let root_canonical = root.as_path().to_path_buf();
    let dir_canonical = dir_safe.as_path().to_path_buf();

    // Convert an absolute path to a project-relative path (e.g., "/songs/foo").
    let to_relative = |abs: &std::path::Path| -> String {
        let suffix = abs
            .strip_prefix(&root_canonical)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if suffix.is_empty() {
            "/".to_string()
        } else {
            format!("/{suffix}")
        }
    };

    let mut entries: Vec<BrowseEntry> = Vec::new();
    match std::fs::read_dir(&dir_canonical) {
        Ok(iter) => {
            for entry in iter.flatten() {
                let path = entry.path();
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Skip hidden files
                if songs::is_hidden(&path) {
                    continue;
                }
                let is_dir = path.is_dir();
                entries.push(BrowseEntry {
                    kind: if is_dir {
                        FileKind::Directory
                    } else {
                        FileKind::of(&path)
                    },
                    path: to_relative(&path),
                    name,
                    is_dir,
                });
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to read directory: {}", e)})),
            )
                .into_response();
        }
    }

    // Sort: directories first, then alphabetically by name
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    (
        StatusCode::OK,
        Json(Browse {
            path: to_relative(&dir_canonical),
            root: root_canonical.to_string_lossy().into_owned(),
            entries,
        }),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct BrowseParams {
    #[serde(default)]
    path: String,
}

/// POST /api/browse/create-song — auto-generates a song.yaml in a project-relative directory.
///
/// Expects a JSON body with `path` (project-relative directory, e.g. "/songs/Afar")
/// and an optional `name` override. The backend scans the directory for audio/MIDI/lighting
/// files and generates the song config automatically, including per-channel track splitting
/// for stereo and multichannel audio files.
pub(super) async fn create_song_in_directory(
    State(state): State<WebUiState>,
    Json(body): Json<CreateSongInDirRequest>,
) -> impl IntoResponse {
    let root = match project_root(&state) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    let dir_safe = match resolve_browse_path(&root, &body.path) {
        Ok(p) => p,
        Err(e) => return *e,
    };
    if !dir_safe.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Path is not a directory"})),
        )
            .into_response();
    }
    let dir_canonical = dir_safe.as_path().to_path_buf();

    let Some(config_path) = songs::song_config_to_write(&dir_canonical) else {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "a song config already exists in this directory"})),
        )
            .into_response();
    };

    // The song's name, by the one rule for a new song's (#674): one given
    // must not be taken by a song loaded, failed or configured under the
    // songs root (a 409 naming it); with none, the folder's name, else its
    // path below the songs root, else the first free suffix. Off the async
    // worker: the configured names are a walk of the songs tree.
    let taken = super::helpers::player_song_names(&state.player);
    let songs_root = state.songs_path();
    let asked = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    let naming = super::helpers::spawn_blocking_io("read the songs folder", {
        let dir = dir_canonical.clone();
        let asked = asked.clone();
        move || -> std::io::Result<Result<String, String>> {
            let root = songs_root.canonicalize().unwrap_or(songs_root);
            match asked {
                Some(name) => {
                    let mut taken = taken;
                    taken.extend(songs::configured_song_names(&root)?);
                    Ok(if taken.contains(&name) {
                        Err(name)
                    } else {
                        Ok(name)
                    })
                }
                None => {
                    // Below the songs root, the path a contested name is
                    // given is from there; elsewhere, from the folder's own.
                    let base = if dir.starts_with(&root) {
                        root
                    } else {
                        dir.parent().unwrap_or(&dir).to_path_buf()
                    };
                    let song = songs::new_song(&base, &dir, taken)?;
                    Ok(Ok(song.map_or_else(
                        || crate::util::filename_display(&dir).to_string(),
                        |song| song.name,
                    )))
                }
            }
        }
    })
    .await;
    let name = match naming {
        Ok(Ok(name)) => name,
        Ok(Err(taken)) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": format!("a song named \"{taken}\" already exists; choose another name"),
                    "taken": taken,
                })),
            )
                .into_response();
        }
        Err(e) => return e,
    };

    // Use Song::initialize to scan the directory and build the config with proper
    // channel-aware track splitting (stereo -> L/R, multichannel -> per-channel).
    // Same blocking decode/analysis as the bulk path, for one song.
    let scan = tokio::task::spawn_blocking({
        let dir = dir_canonical.clone();
        move || {
            songs::Song::initialize(&dir)
                .map(|song| song.get_config())
                .map_err(|e| e.to_string())
        }
    })
    .await;
    let mut song_config = match scan {
        Ok(Ok(config)) => config,
        Ok(Err(e)) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Failed to scan directory: {}", e)})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Directory scan failed: {e}")})),
            )
                .into_response();
        }
    };

    song_config.set_name(&name);

    // Flattened before the reload's await: the save's error is not Send.
    if let Err(e) = song_config.save(&config_path).map_err(|e| e.to_string()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save song config: {}", e)})),
        )
            .into_response();
    }
    // Refresh the player's all-songs playlist so newly imported songs appear.
    if let Err(resp) = super::helpers::reload_songs(&state).await {
        return resp;
    }
    (
        StatusCode::CREATED,
        Json(json!({"status": "created", "path": config_path.to_string_lossy(), "song": name})),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct CreateSongInDirRequest {
    path: String,
    name: Option<String>,
}

/// POST /api/browse/bulk-import — creates a song.yaml in each song folder under a parent
/// directory ([`import_song_dirs`]). Returns a summary of created, skipped, and failed
/// directories.
pub(super) async fn bulk_import(
    State(state): State<WebUiState>,
    Json(body): Json<BulkImportRequest>,
) -> impl IntoResponse {
    let root = match project_root(&state) {
        Ok(r) => r,
        Err(e) => return *e,
    };
    let dir_safe = match resolve_browse_path(&root, &body.path) {
        Ok(p) => p,
        Err(e) => return *e,
    };
    if !dir_safe.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Path is not a directory"})),
        )
            .into_response();
    }
    let dir_canonical = dir_safe.as_path().to_path_buf();

    // `Song::initialize` decodes and analyses each candidate's click track, so
    // a first import of a large library is minutes of blocking CPU and file
    // I/O. Off the async worker, or it parks a runtime thread for the duration.
    // codeql[rust/path-injection] dir_canonical is canonicalized and
    // verified via starts_with(root_canonical) above, preventing traversal.
    // The songs the player has loaded, or failed to, under the songs root:
    // no new song is named as one of them.
    let taken = super::helpers::player_song_names(&state.player);
    let songs_root = state.songs_path();
    let scan =
        tokio::task::spawn_blocking(move || import_song_dirs(&dir_canonical, &songs_root, taken))
            .await;

    let (created, skipped, failed) = match scan {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to read directory: {e}")})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Import scan failed: {e}")})),
            )
                .into_response();
        }
    };

    // Refresh the player's song state once after all imports
    if !created.is_empty() {
        if let Err(resp) = super::helpers::reload_songs(&state).await {
            return resp;
        }
    }

    (
        StatusCode::OK,
        Json(BulkImport {
            created,
            skipped,
            failed,
        }),
    )
        .into_response()
}

/// Imports each song folder under `root` that has no config to keep
/// ([`songs::new_songs`]), named past the songs in `taken` and those
/// configured under `songs_root` too; one with a config is skipped. Each is
/// reported by its path below `root`. An unreadable `root` is an error.
fn import_song_dirs(
    root: &std::path::Path,
    songs_root: &std::path::Path,
    mut taken: std::collections::HashSet<String>,
) -> std::io::Result<(Vec<String>, Vec<String>, Vec<ImportFailure>)> {
    match songs::configured_song_names(songs_root) {
        Ok(names) => taken.extend(names),
        Err(e) => tracing::warn!(
            dir = %songs_root.display(),
            error = %e,
            "Cannot read the songs folder for its song names"
        ),
    }
    let found = songs::new_songs(root, taken)?;
    let display_name = |dir: &std::path::Path| {
        dir.strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .to_string()
    };
    let skipped = found.kept.iter().map(|dir| display_name(dir)).collect();
    let mut created: Vec<String> = Vec::new();
    let mut failed: Vec<ImportFailure> = Vec::new();
    for song in &found.new {
        match song.write() {
            Ok(()) => created.push(display_name(&song.dir)),
            Err(e) => failed.push(ImportFailure {
                name: display_name(&song.dir),
                error: format!("{}", e),
            }),
        }
    }
    Ok((created, skipped, failed))
}

#[derive(serde::Deserialize)]
pub(super) struct BulkImportRequest {
    path: String,
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn browse_directory_default_lists_project_root() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.get("path").is_some());
        assert!(parsed.get("root").is_some());
        assert!(parsed.get("entries").is_some());
        assert!(parsed["entries"].is_array());
    }

    #[tokio::test]
    async fn browse_directory_with_path() {
        let (state, _dir) = test_state();
        // Create a subdirectory inside the project root.
        let subdir = _dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("file.txt"), "hello").unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/subdir")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let entries = parsed["entries"].as_array().unwrap();
        assert!(!entries.is_empty());
    }

    #[tokio::test]
    async fn browse_directory_path_traversal_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/../../../etc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The server should reject this with FORBIDDEN or NOT_FOUND.
        let status = response.status();
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "expected 403 or 404, got {status}"
        );
    }

    #[tokio::test]
    async fn create_song_in_directory_success() {
        let (state, _dir) = test_state();
        // Create a directory with a WAV file inside the project root.
        let song_dir = _dir.path().join("songs").join("mysong");
        std::fs::create_dir_all(&song_dir).unwrap();
        let wav_bytes = create_test_wav();
        std::fs::write(song_dir.join("track.wav"), &wav_bytes).unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs/mysong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "created");
    }

    #[tokio::test]
    async fn create_song_in_directory_conflict() {
        let (state, _dir) = test_state();
        // Create a directory that already has song.yaml.
        let song_dir = _dir.path().join("songs").join("existing");
        std::fs::create_dir_all(&song_dir).unwrap();
        std::fs::write(song_dir.join("song.yaml"), "name: existing\ntracks: []\n").unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs/existing"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    /// A directory whose config is `song.yml` already has one: create-song
    /// is refused, and no `song.yaml` is written beside it.
    #[tokio::test]
    async fn create_song_in_directory_conflict_with_song_yml() {
        let (state, dir) = test_state();
        let song_dir = dir.path().join("songs").join("existing");
        std::fs::create_dir_all(&song_dir).unwrap();
        std::fs::write(song_dir.join("song.yml"), "name: existing\ntracks: []\n").unwrap();

        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs/existing"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!song_dir.join("song.yaml").exists());
    }

    /// Create-song at a songs root holding a flat-layout song
    /// (`<root>/flat.yaml`) is refused: the root has a song config.
    #[tokio::test]
    async fn create_song_in_directory_conflict_with_a_flat_song() {
        let (state, dir) = test_state();
        let songs = dir.path().join("songs");
        std::fs::create_dir_all(&songs).unwrap();
        std::fs::write(songs.join("flat.yaml"), "name: Flat\ntracks: []\n").unwrap();

        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!songs.join("song.yaml").exists());
    }

    /// The browser leaves hidden entries out: dot-files, AppleDouble files
    /// and hidden directories.
    #[tokio::test]
    async fn browse_directory_hides_hidden_entries() {
        let (state, dir) = test_state();
        let subdir = dir.path().join("hiddendir");
        std::fs::create_dir_all(subdir.join(".git")).unwrap();
        std::fs::write(subdir.join("track.wav"), create_test_wav()).unwrap();
        std::fs::write(subdir.join("._track.wav"), "fork").unwrap();
        std::fs::write(subdir.join(".draft.light"), "draft").unwrap();

        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/hiddendir")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        let names: Vec<_> = parsed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["track.wav"]);
    }

    /// The bulk import passes over a hidden directory.
    #[tokio::test]
    async fn bulk_import_passes_over_hidden_dirs() {
        let (state, dir) = test_state();
        let parent = dir.path().join("songs");
        for song in ["Alpha", ".trash"] {
            std::fs::create_dir_all(parent.join(song)).unwrap();
            std::fs::write(parent.join(song).join("track.wav"), create_test_wav()).unwrap();
        }

        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(parent.join("Alpha/song.yaml").exists());
        assert!(!parent.join(".trash/song.yaml").exists());
    }

    #[tokio::test]
    async fn browse_directory_classifies_file_types() {
        let (state, _dir) = test_state();
        let subdir = _dir.path().join("typedir");
        std::fs::create_dir(&subdir).unwrap();
        let wav_bytes = create_test_wav();
        std::fs::write(subdir.join("track.wav"), &wav_bytes).unwrap();
        std::fs::write(subdir.join("notes.mid"), "midi data").unwrap();
        std::fs::write(subdir.join("show.light"), "light data").unwrap();
        std::fs::write(subdir.join("readme.txt"), "text data").unwrap();
        std::fs::write(subdir.join("Bass.WAV"), &wav_bytes).unwrap();
        std::fs::write(subdir.join("Notes.MID"), "midi data").unwrap();
        std::fs::write(subdir.join("Show.LIGHT"), "light data").unwrap();
        std::fs::write(subdir.join("notes.midi"), "midi data").unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/typedir")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let entries = parsed["entries"].as_array().unwrap();

        let find_entry = |name: &str| entries.iter().find(|e| e["name"] == name).unwrap();
        assert_eq!(find_entry("track.wav")["type"], "audio");
        assert_eq!(find_entry("notes.mid")["type"], "midi");
        assert_eq!(find_entry("show.light")["type"], "lighting");
        assert_eq!(find_entry("readme.txt")["type"], "other");
        assert_eq!(find_entry("Bass.WAV")["type"], "audio");
        assert_eq!(find_entry("Notes.MID")["type"], "midi");
        assert_eq!(find_entry("Show.LIGHT")["type"], "lighting");
        assert_eq!(find_entry("notes.midi")["type"], "other");
    }

    /// Posts a create-song for `songs/<folder>` (holding a WAV), with
    /// `name` if given: the status and the answer.
    async fn create_song_named(
        state: &crate::webui::server::WebUiState,
        project: &std::path::Path,
        folder: &str,
        name: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let song_dir = project.join("songs").join(folder);
        std::fs::create_dir_all(&song_dir).unwrap();
        std::fs::write(song_dir.join("track.wav"), create_test_wav()).unwrap();
        let body = serde_json::json!({"path": format!("/songs/{folder}"), "name": name});
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        (
            status,
            serde_json::from_str(&response_body(response).await).unwrap(),
        )
    }

    /// #674: a name given that a song has already, loaded or configured
    /// under the songs root, is a 409 naming it, and nothing is written.
    #[tokio::test]
    async fn create_song_refuses_a_name_taken() {
        let (state, dir) = test_state();
        let other = state.songs_path().join("Other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("song.yaml"), "name: Fresh\ntracks: []\n").unwrap();
        for taken in ["Song A", "Fresh"] {
            let (status, parsed) = create_song_named(&state, dir.path(), "new", Some(taken)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{taken}: {parsed}");
            assert_eq!(parsed["taken"], taken);
            assert_eq!(
                parsed["error"],
                format!("a song named \"{taken}\" already exists; choose another name")
            );
            assert!(!dir.path().join("songs/new/song.yaml").exists());
        }
        let (status, parsed) = create_song_named(&state, dir.path(), "new", Some("Free")).await;
        assert_eq!(status, StatusCode::CREATED, "{parsed}");
        assert_eq!(parsed["song"], "Free");
    }

    /// #674: with no name given, the folder's, by the bulk import's rule:
    /// past a name taken, the first free suffix.
    #[tokio::test]
    async fn create_song_with_no_name_is_named_past_the_names_taken() {
        let (state, dir) = test_state();
        let (status, parsed) = create_song_named(&state, dir.path(), "Song A", None).await;
        assert_eq!(status, StatusCode::CREATED, "{parsed}");
        assert_eq!(parsed["song"], "Song A (2)");
        let config =
            crate::config::Song::deserialize(&dir.path().join("songs/Song A/song.yaml")).unwrap();
        assert_eq!(config.name(), "Song A (2)");
    }

    #[tokio::test]
    async fn create_song_in_directory_with_name_override() {
        let (state, _dir) = test_state();
        let song_dir = _dir.path().join("songs").join("override_test");
        std::fs::create_dir_all(&song_dir).unwrap();
        let wav_bytes = create_test_wav();
        std::fs::write(song_dir.join("track.wav"), &wav_bytes).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path": "/songs/override_test", "name": "Custom Name"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let config_content = std::fs::read_to_string(song_dir.join("song.yaml")).unwrap();
        assert!(
            config_content.contains("Custom Name"),
            "song.yaml should contain the custom name, got: {}",
            config_content
        );
    }

    #[tokio::test]
    async fn create_song_in_directory_nonexistent_path() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/nonexistent/dir"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn browse_directory_sorts_dirs_first() {
        let (state, _dir) = test_state();
        let subdir = _dir.path().join("sorttest");
        std::fs::create_dir(&subdir).unwrap();
        // Create files and directories
        std::fs::write(subdir.join("aaa_file.txt"), "data").unwrap();
        std::fs::create_dir(subdir.join("zzz_dir")).unwrap();
        std::fs::write(subdir.join("bbb_file.wav"), create_test_wav()).unwrap();
        std::fs::create_dir(subdir.join("aaa_dir")).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/sorttest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let entries = parsed["entries"].as_array().unwrap();

        // Directories should come first
        assert_eq!(entries.len(), 4);
        assert!(entries[0]["is_dir"].as_bool().unwrap());
        assert!(entries[1]["is_dir"].as_bool().unwrap());
        assert!(!entries[2]["is_dir"].as_bool().unwrap());
        assert!(!entries[3]["is_dir"].as_bool().unwrap());

        // Within groups, sorted alphabetically
        assert_eq!(entries[0]["name"], "aaa_dir");
        assert_eq!(entries[1]["name"], "zzz_dir");
    }

    #[tokio::test]
    async fn bulk_import_creates_songs() {
        let (state, _dir) = test_state();
        let parent = _dir.path().join("songs");
        std::fs::create_dir_all(&parent).unwrap();

        // Two subdirectories with audio, one without, one already has song.yaml
        let song_a = parent.join("Alpha");
        std::fs::create_dir_all(&song_a).unwrap();
        std::fs::write(song_a.join("track.wav"), create_test_wav()).unwrap();

        let song_b = parent.join("Beta");
        std::fs::create_dir_all(&song_b).unwrap();
        std::fs::write(song_b.join("track.wav"), create_test_wav()).unwrap();

        let song_c = parent.join("Gamma");
        std::fs::create_dir_all(&song_c).unwrap();
        // No audio files, no subdirs — skipped (not a song candidate)

        let song_d = parent.join("Delta");
        std::fs::create_dir_all(&song_d).unwrap();
        std::fs::write(song_d.join("track.wav"), create_test_wav()).unwrap();
        std::fs::write(song_d.join("song.yaml"), "name: Delta\ntracks: []\n").unwrap();
        // Already has song.yaml — should be skipped

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();

        let created: Vec<&str> = parsed["created"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let skipped: Vec<&str> = parsed["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        assert!(
            created.contains(&"Alpha"),
            "Alpha should be created: {:?}",
            created
        );
        assert!(
            created.contains(&"Beta"),
            "Beta should be created: {:?}",
            created
        );
        assert!(
            skipped.contains(&"Delta"),
            "Delta should be skipped (existing): {:?}",
            skipped
        );

        // Gamma has no audio and no subdirs — not imported, not in any list
        assert!(
            !created.iter().any(|s| s.contains("Gamma")),
            "Gamma should not be created (no audio): {:?}",
            created
        );

        // Verify song.yaml was actually created
        assert!(song_a.join("song.yaml").exists());
        assert!(song_b.join("song.yaml").exists());
        assert!(!song_c.join("song.yaml").exists());
    }

    /// A directory whose only audio is `Bass.WAV` is imported with that
    /// track: the import and `Song::initialize` read the extension by the
    /// same rule.
    #[tokio::test]
    async fn bulk_import_reads_extensions_in_any_case() {
        let (state, _dir) = test_state();
        let song = _dir.path().join("loud").join("Shout");
        std::fs::create_dir_all(&song).unwrap();
        std::fs::write(song.join("Bass.WAV"), create_test_wav()).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/loud"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["created"], serde_json::json!(["Shout"]), "{body}");
        let config = std::fs::read_to_string(song.join("song.yaml")).unwrap();
        assert!(config.contains("Bass.WAV"), "{config}");
    }

    /// The bulk import takes the walk's song folders (`songs::song_dirs`):
    /// a MIDI-only folder is imported; a folder holding only hidden media is
    /// not; one with a `song.yml` is skipped and left as it is.
    #[tokio::test]
    async fn bulk_import_takes_the_song_folders_walk() {
        let (state, _dir) = test_state();
        let parent = _dir.path().join("lib");
        for dir in ["Midi", "Hid", "Yml"] {
            std::fs::create_dir_all(parent.join(dir)).unwrap();
        }
        std::fs::write(parent.join("Midi/notes.mid"), "midi data").unwrap();
        std::fs::write(parent.join("Hid/._track.wav"), [0_u8, 5, 22, 7]).unwrap();
        std::fs::write(parent.join("Yml/track.wav"), create_test_wav()).unwrap();
        let yml = "name: Yml Live\ntracks: []\n";
        std::fs::write(parent.join("Yml/song.yml"), yml).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/lib"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["created"], serde_json::json!(["Midi"]), "{body}");
        assert_eq!(parsed["skipped"], serde_json::json!(["Yml"]), "{body}");
        assert!(parent.join("Midi/song.yaml").exists());
        assert!(!parent.join("Hid/song.yaml").exists());
        assert!(!parent.join("Yml/song.yaml").exists());
        assert_eq!(
            std::fs::read_to_string(parent.join("Yml/song.yml")).unwrap(),
            yml
        );
    }

    /// An unreadable subdirectory is passed over; an unreadable directory
    /// to import from is an error.
    #[cfg(unix)]
    #[tokio::test]
    async fn bulk_import_passes_over_an_unreadable_subdirectory() {
        use std::os::unix::fs::PermissionsExt;
        let (state, _dir) = test_state();
        let parent = _dir.path().join("lib");
        for dir in ["Foo", "Locked/Bar"] {
            std::fs::create_dir_all(parent.join(dir)).unwrap();
            std::fs::write(parent.join(dir).join("track.wav"), create_test_wav()).unwrap();
        }
        let lock = |path: &std::path::Path, mode| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        lock(&parent.join("Locked"), 0o000);
        if std::fs::read_dir(parent.join("Locked")).is_ok() {
            // Running as root: the folder reads all the same.
            lock(&parent.join("Locked"), 0o755);
            return;
        }
        let import = |path: &'static str| {
            let app = router().with_state(state.clone());
            async move {
                app.oneshot(
                    http::Request::builder()
                        .method("POST")
                        .uri("/browse/bulk-import")
                        .header("content-type", "application/json")
                        .body(Body::from(format!(r#"{{"path": "{path}"}}"#)))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        // On main, the import passed over `Locked` silently and answered an
        // import of `/lib/Locked` with 200 and nothing imported.
        let response = import("/lib").await;
        let status = response.status();
        let body = response_body(response).await;
        let locked = import("/lib/Locked").await.status();
        lock(&parent.join("Locked"), 0o755);

        assert_eq!(status, StatusCode::OK);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["created"], serde_json::json!(["Foo"]), "{body}");
        assert_eq!(locked, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!parent.join("Locked/Bar/song.yaml").exists());
    }

    /// POST /browse/bulk-import with `{"path": path}`, answered.
    async fn bulk_import(
        state: &crate::webui::server::WebUiState,
        path: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"path": "{path}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, serde_json::from_str(&body).unwrap())
    }

    /// The bulk import no longer follows a symlinked folder (it did on
    /// main), so it writes nothing outside the tree; it looks into a folder
    /// holding only a `song.yaml` of another kind and leaves that file; and
    /// a folder named as a song the player has ("Song A") is named past it.
    #[cfg(unix)]
    #[tokio::test]
    async fn bulk_import_symlinks_other_kinds_and_taken_names() {
        let (state, _dir) = test_state();
        let parent = _dir.path().join("lib");
        let outside = _dir.path().join("outside/Song");
        for dir in [&parent.join("Pl/Inner"), &parent.join("Song A"), &outside] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for dir in [&parent.join("Pl/Inner"), &parent.join("Song A"), &outside] {
            std::fs::write(dir.join("track.wav"), create_test_wav()).unwrap();
        }
        let playlist = "kind: playlist\nsongs: []\n";
        std::fs::write(parent.join("Pl/song.yaml"), playlist).unwrap();
        std::os::unix::fs::symlink(&outside, parent.join("Linked")).unwrap();

        let (status, parsed) = bulk_import(&state, "/lib").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["created"], serde_json::json!(["Pl/Inner", "Song A"]));
        assert_eq!(parsed["skipped"], serde_json::json!([]));
        assert!(!outside.join("song.yaml").exists());
        assert_eq!(
            std::fs::read_to_string(parent.join("Pl/song.yaml")).unwrap(),
            playlist
        );
        let named = crate::config::Song::deserialize(&parent.join("Song A/song.yaml")).unwrap();
        assert_eq!(named.name(), "Song A (2)");
    }

    /// A bulk import names a folder past a song configured under the songs
    /// folder that the player has not loaded (`songs/Later/song.yaml`
    /// naming "Encore", written since the last reload).
    #[tokio::test]
    async fn bulk_import_names_past_songs_configured_under_the_songs_folder() {
        let (state, _dir) = test_state();
        let later = state.songs_path().join("Later");
        std::fs::create_dir_all(&later).unwrap();
        std::fs::write(later.join("song.yaml"), "name: Encore\ntracks: []\n").unwrap();
        assert!(state.player.songs().get("Encore").is_err());
        let encore = _dir.path().join("inbox/Encore");
        std::fs::create_dir_all(&encore).unwrap();
        std::fs::write(encore.join("track.wav"), create_test_wav()).unwrap();

        let (status, parsed) = bulk_import(&state, "/inbox").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["created"], serde_json::json!(["Encore"]));
        let named = crate::config::Song::deserialize(&encore.join("song.yaml")).unwrap();
        assert_eq!(named.name(), "Encore (2)");
    }

    /// Create-song from a folder answers 409 beside a `song.yaml` of
    /// another kind, and leaves it (`songs::song_config_to_write`); it wrote
    /// over it on main.
    #[tokio::test]
    async fn create_song_in_directory_keeps_a_song_yaml_of_another_kind() {
        let (state, _dir) = test_state();
        let dir = _dir.path().join("Pl");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("track.wav"), create_test_wav()).unwrap();
        let playlist = "kind: playlist\nsongs: []\n";
        std::fs::write(dir.join("song.yaml"), playlist).unwrap();

        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/Pl"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            std::fs::read_to_string(dir.join("song.yaml")).unwrap(),
            playlist
        );
    }

    #[tokio::test]
    async fn bulk_import_nonexistent_path() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/nonexistent"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn bulk_import_empty_directory() {
        let (state, _dir) = test_state();
        let parent = _dir.path().join("empty_parent");
        std::fs::create_dir_all(&parent).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/empty_parent"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["created"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bulk_import_recursive_nested() {
        let (state, _dir) = test_state();
        let parent = _dir.path().join("music");
        std::fs::create_dir_all(&parent).unwrap();

        // Nested: music/artist/album/song
        let song_deep = parent.join("Artist").join("Album").join("Track1");
        std::fs::create_dir_all(&song_deep).unwrap();
        std::fs::write(song_deep.join("track.wav"), create_test_wav()).unwrap();

        // Also a direct child with audio
        let song_shallow = parent.join("MySong");
        std::fs::create_dir_all(&song_shallow).unwrap();
        std::fs::write(song_shallow.join("audio.wav"), create_test_wav()).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/bulk-import")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/music"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();

        let created: Vec<&str> = parsed["created"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        // Both the deeply nested and the shallow song should be found
        assert!(
            created.iter().any(|s| s.contains("Track1")),
            "Nested Track1 should be created: {:?}",
            created
        );
        assert!(
            created.iter().any(|s| s.contains("MySong")),
            "MySong should be created: {:?}",
            created
        );

        assert!(song_deep.join("song.yaml").exists());
        assert!(song_shallow.join("song.yaml").exists());
        // Intermediate dirs without audio should NOT have song.yaml
        assert!(!parent.join("Artist").join("song.yaml").exists());
        assert!(!parent
            .join("Artist")
            .join("Album")
            .join("song.yaml")
            .exists());
    }
}
