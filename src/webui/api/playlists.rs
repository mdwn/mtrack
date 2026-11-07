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

use crate::webui::wire::playlists::{PlaylistBody, PlaylistDetail, PlaylistInfo};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::server::WebUiState;
use super::helpers::{
    require_configured_dir, validate_resource_name, validate_resource_name_to_write,
};
use crate::config;

// ---------------------------------------------------------------------------
// Playlist CRUD endpoints
// ---------------------------------------------------------------------------

/// Validates a playlist name for use in file paths.
/// The player's own playlist is refused (400) saying so.
#[allow(clippy::result_large_err)]
fn validate_playlist_name(name: &str) -> Result<(), axum::response::Response> {
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

/// GET /api/playlists — list all playlists with name, song count, and active status.
pub(super) async fn get_playlists(State(state): State<WebUiState>) -> impl IntoResponse {
    let playlists = state.player.list_playlists();
    let active = {
        let active_playlist = state.player.get_playlist();
        active_playlist.name().to_string()
    };
    let items: Vec<PlaylistInfo> = playlists
        .iter()
        .map(|name| {
            let playlist_map = state.player.playlists_snapshot();
            let song_count = playlist_map.get(name).map(|p| p.songs().len()).unwrap_or(0);
            PlaylistInfo {
                name: name.clone(),
                song_count,
                is_active: name == &active,
                reserved: crate::playlist::is_reserved_name(name),
            }
        })
        .collect();
    (StatusCode::OK, Json(items)).into_response()
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
    let registry = state.player.songs();
    let all_songs: Vec<String> = registry
        .sorted_list()
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    // Each song's tracks, for picking the ones a fade leaves out.
    let song_tracks = registry
        .sorted_list()
        .iter()
        .map(|s| (s.name().to_string(), s.output_track_names()))
        .collect();
    // Each entry's auto play as the editor writes it: `true` for at once,
    // a gap, or null for none.
    let auto_play = pl
        .entry_auto_play()
        .into_iter()
        .map(|gap| match gap {
            None => None,
            Some(gap) if gap.is_zero() => Some(config::AutoPlayConfig::On(true)),
            Some(gap) => Some(config::AutoPlayConfig::Gap(format_gap(gap))),
        })
        .collect();
    let detail = PlaylistDetail {
        songs: pl.songs().clone(),
        auto_play,
        fades: pl.entry_fades().to_vec(),
        ambient: pl.ambient().cloned(),
        entry_ambient: pl.entry_ambient().to_vec(),
        available_songs: all_songs,
        song_tracks,
        name,
    };
    (StatusCode::OK, Json(detail)).into_response()
}

/// A gap as written in a playlist: whole seconds as `2s`, else `1500ms`.
fn format_gap(gap: std::time::Duration) -> String {
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
pub(crate) static PLAYLIST_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// PUT /api/playlists/:name — create or update a playlist.
pub(super) async fn put_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Json(body): Json<PlaylistBody>,
) -> impl IntoResponse {
    let _writing = PLAYLIST_WRITES.lock().await;
    let playlists_dir =
        require_configured_dir(&state.playlists_dir(), "playlists", StatusCode::NOT_FOUND)?;
    let file = playlist_file_to_write(&playlists_dir, &name).await?;
    let file_path = file.path().to_path_buf();

    // The editor knows only the songs: an existing playlist keeps its
    // transitions, and each entry its own with the song. One that does not
    // read is not overwritten, which would lose them.
    let playlist_config = if file.exists() {
        match config::Playlist::deserialize(&file_path) {
            Ok(existing) => existing.with_songs(&body.songs),
            Err(e) => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": format!(
                            "The playlist file does not read, so saving would lose its \
                             settings; fix it by hand first: {e}"
                        )
                    })),
                )
                    .into_response());
            }
        }
    } else {
        config::Playlist::new(&body.songs)
    };
    // `auto_play` and `fades` go entry for entry with `songs`: a list of
    // another length would be laid over the entries by position, those past
    // its end keeping their old settings while the ones before lost theirs.
    let lengths = [
        ("auto_play", body.auto_play.as_ref().map(Vec::len)),
        ("fades", body.fades.as_ref().map(Vec::len)),
        ("entry_ambient", body.entry_ambient.as_ref().map(Vec::len)),
    ];
    for (field, length) in lengths {
        if let Some(length) = length.filter(|&length| length != body.songs.len()) {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": format!(
                        "{field} has {length} entries, but there are {} songs: one for each",
                        body.songs.len()
                    )
                })),
            )
                .into_response());
        }
    }
    let playlist_config = match &body.auto_play {
        Some(auto_play) => playlist_config.with_auto_play(auto_play),
        None => playlist_config,
    };
    let playlist_config = match &body.fades {
        Some(fades) => playlist_config.with_fades(fades),
        None => playlist_config,
    };
    let playlist_config = match &body.entry_ambient {
        Some(ambient) => playlist_config.with_entry_ambient(ambient),
        None => playlist_config,
    };
    let playlist_config = match body.ambient {
        Some(ambient) => playlist_config.with_ambient(ambient),
        None => playlist_config,
    };
    check_playlist(&playlist_config, &state.player)?;
    let yaml = match crate::util::to_yaml_string(&playlist_config) {
        // A text file ends with a newline.
        Ok(y) if y.ends_with('\n') => y,
        Ok(y) => y + "\n",
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to serialize playlist: {}", e)})),
            )
                .into_response());
        }
    };

    write_playlist_file(&file, yaml, &super::helpers::project_dir(&state)).await?;

    // Reload songs to pick up the new playlist.
    super::helpers::reload_songs(&state).await?;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "name": name})),
        )
            .into_response(),
    )
}

/// The file a save of the playlist `name` writes, in the playlists
/// directory `dir`, by the one rule of the web UI's save and MCP's
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

/// Refuses (422) a playlist the player would refuse to load, before it is
/// written: a song it does not have, a bad fade or ambient, or a fade
/// leaving out a track its song does not have. So is a fade the player
/// would cut short, longer than the song it plays under, and an ambient
/// show outside the project. The web UI's and MCP's playlist saves.
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
/// runtime: the web UI's and MCP's playlist saves, once checked. The
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

/// DELETE /api/playlists/:name — delete a playlist.
pub(super) async fn delete_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    validate_playlist_name(&name)?;
    let _writing = PLAYLIST_WRITES.lock().await;
    let playlists_dir =
        require_configured_dir(&state.playlists_dir(), "playlists", StatusCode::NOT_FOUND)?;

    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in playlist_path.
    let target = playlist_path(&playlists_dir, &name)?;
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

    // Reload songs to remove the playlist.
    super::helpers::reload_songs(&state).await?;

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
        Err(e) => super::helpers::fault_response(e.kind(), e.to_string()),
    }
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use super::format_gap;
    use crate::config;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_playlists_returns_list() {
        let (state, _dir) = test_state();
        // Create a playlist file with an empty songs list so it loads without
        // requiring actual song directories on disk.
        let playlists_dir = state.playlists_dir().clone().unwrap();
        std::fs::write(playlists_dir.join("mylist.yaml"), "songs: []\n").unwrap();
        // Reload songs so the player picks up the new playlist.
        state.player.reload_songs().await.unwrap();
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
        assert!(arr
            .iter()
            .any(|v| v["name"] == "mylist" && v["reserved"] == false));
        // The player's own playlist says it is: the web UI hides it by that.
        assert!(arr
            .iter()
            .any(|v| v["name"] == "all_songs" && v["reserved"] == true));
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
        let playlists_dir = state.playlists_dir().clone().unwrap();
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
        let written = std::fs::read_to_string(playlists_dir.join("mylist.yaml")).unwrap();
        assert!(written.ends_with('\n'), "{written:?}");
    }

    /// PUTs `{"songs": [<songs>]}` to the playlist `mylist` in a fresh
    /// state with songs A, B and C on disk and a playlists directory holding
    /// `files`; returns the status, the directory, and the songs of
    /// `mylist` as the reloaded player has it.
    async fn put_mylist(
        files: &[(&str, &str)],
        songs: &str,
    ) -> (
        StatusCode,
        std::path::PathBuf,
        Vec<String>,
        tempfile::TempDir,
    ) {
        let (state, dir) = test_state();
        for name in ["A", "B", "C"] {
            let song_dir = state.songs_path().join(format!("song_{name}"));
            std::fs::create_dir(&song_dir).unwrap();
            crate::testutil::write_wav(song_dir.join("kick.wav"), vec![vec![0_i32; 441]], 44100)
                .unwrap();
            std::fs::write(
                song_dir.join("song.yaml"),
                format!("name: Song {name}\ntracks:\n- name: kick\n  file: kick.wav\n"),
            )
            .unwrap();
        }
        let playlists_dir = state.playlists_dir().clone().unwrap();
        for (file, text) in files {
            std::fs::write(playlists_dir.join(file), text).unwrap();
        }
        state.player.reload_songs().await.unwrap();
        let status = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlists/mylist")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"songs": [{songs}]}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status();
        let loaded = state
            .player
            .playlists_snapshot()
            .get("mylist")
            .map(|playlist| playlist.songs().clone())
            .unwrap_or_default();
        (status, playlists_dir, loaded, dir)
    }

    /// A playlist kept as `.yml` is saved into that file, keeping its
    /// settings, not into a new `.yaml` beside it, and the reloaded player
    /// plays the songs saved; with both there, the `.yaml` the loader reads
    /// is saved and the `.yml` left alone.
    #[tokio::test]
    async fn put_playlist_by_name_writes_the_names_file() {
        let old = "songs:\n  - Song A\nambient: false\n";
        let (status, playlists_dir, loaded, _dir) =
            put_mylist(&[("mylist.yml", old)], r#""Song B""#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!playlists_dir.join("mylist.yaml").exists());
        let written = std::fs::read_to_string(playlists_dir.join("mylist.yml")).unwrap();
        assert!(written.contains("Song B"), "{written}");
        assert!(written.contains("ambient: false"), "{written}");
        assert_eq!(loaded, ["Song B"]);

        let files = [("mylist.yaml", "songs:\n  - Song A\n"), ("mylist.yml", old)];
        let (status, playlists_dir, loaded, _dir) = put_mylist(&files, r#""Song C""#).await;
        assert_eq!(status, StatusCode::OK);
        let written = std::fs::read_to_string(playlists_dir.join("mylist.yaml")).unwrap();
        assert!(written.contains("Song C"), "{written}");
        assert_eq!(
            std::fs::read_to_string(playlists_dir.join("mylist.yml")).unwrap(),
            old
        );
        assert_eq!(loaded, ["Song C"]);
    }

    /// A new playlist named as a file there but for case (`set` beside
    /// `Set.yaml`, which a case-insensitive filesystem would open for it)
    /// is refused, and `Set.yaml` left alone.
    #[tokio::test]
    async fn put_playlist_refuses_a_name_differing_only_in_case() {
        let old = "songs:\n  - Song A\n";
        let (state, dir) = test_state();
        let playlists_dir = state.playlists_dir().clone().unwrap();
        std::fs::write(playlists_dir.join("Set.yaml"), old).unwrap();
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlists/set")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"songs": []}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response_body(response).await;
        assert!(
            body.contains("`Set.yaml` already exists; names differ only in case"),
            "{body}"
        );
        assert_eq!(
            std::fs::read_to_string(playlists_dir.join("Set.yaml")).unwrap(),
            old
        );
        assert!(!playlists_dir.join("set.yaml").exists());
        drop(dir);
    }

    /// A delete removes the name's file: its `.yml` when that is all there
    /// is, its `.yaml` when both are there.
    #[tokio::test]
    async fn delete_playlist_by_name_removes_the_names_file() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir().clone().unwrap();
        std::fs::write(playlists_dir.join("old.yml"), "songs: []\n").unwrap();
        std::fs::write(playlists_dir.join("both.yaml"), "songs: []\n").unwrap();
        std::fs::write(playlists_dir.join("both.yml"), "songs: []\n").unwrap();
        let app = router().with_state(state);
        let delete = |uri: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(
                    http::Request::builder()
                        .method("DELETE")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        };

        assert_eq!(delete("/playlists/old").await, StatusCode::OK);
        assert!(!playlists_dir.join("old.yml").exists());
        assert_eq!(delete("/playlists/both").await, StatusCode::OK);
        assert!(!playlists_dir.join("both.yaml").exists());
        assert!(playlists_dir.join("both.yml").exists());
    }

    /// PUTs `songs` (JSON) to the playlist `mylist`, whose file holds
    /// `existing`; returns the status and the file after.
    async fn put_over(existing: &str, songs: &str) -> (StatusCode, String) {
        put_body_over(existing, &format!(r#"{{"songs": {songs}}}"#)).await
    }

    /// PUTs `body` to the playlist `mylist`, whose file holds `existing`;
    /// returns the status and the file after.
    async fn put_body_over(existing: &str, body: &str) -> (StatusCode, String) {
        let (state, _dir) = test_state_with_registry(songs_abc());
        let path = state.playlists_dir().clone().unwrap().join("mylist.yaml");
        std::fs::write(&path, existing).unwrap();
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlists/mylist")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        (response.status(), std::fs::read_to_string(&path).unwrap())
    }

    /// A registry of "Song A", "Song B" and "Song C", each with tracks
    /// `kick` and `snare`.
    fn songs_abc() -> std::sync::Arc<crate::songs::Songs> {
        use crate::songs::{Song, Songs};
        let map = ["Song A", "Song B", "Song C"]
            .into_iter()
            .map(|name| {
                let song = Song::new_for_test(name, &["kick", "snare"]);
                (name.to_string(), std::sync::Arc::new(song))
            })
            .collect();
        std::sync::Arc::new(Songs::new(map))
    }

    #[tokio::test]
    async fn put_playlist_refuses_a_song_it_does_not_have() {
        // #578: saved, and then the playlist vanished, being one the player
        // would not load.
        let existing = "songs:\n- Song A\n";
        let (status, after) = put_over(existing, r#"["Song A", "No Such Song"]"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(after, existing);
    }

    #[tokio::test]
    async fn put_playlist_refuses_lists_that_do_not_match_its_songs() {
        // #578: laid over the entries by position, the first lost its
        // fade_in and the rest kept theirs.
        let existing = "songs:\n- song: Song A\n  fade_in: 1s\n- Song B\n- Song C\n";
        for body in [
            r#"{"songs": ["Song A", "Song B", "Song C"], "fades": [{"fade_out": "9s"}]}"#,
            r#"{"songs": ["Song A"], "auto_play": [true, false]}"#,
        ] {
            let (status, after) = put_body_over(existing, body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert_eq!(after, existing, "{body}");
        }
    }

    #[tokio::test]
    async fn put_playlist_keeps_its_ambient_show_inside_the_project() {
        // #578: saved, and read from wherever it pointed on a switch.
        let existing = "songs:\n- Song A\n";
        for show in [
            "../../../etc/passwd",
            "/etc/passwd",
            "lighting/../../x.light",
        ] {
            let body = format!(r#"{{"songs": ["Song A"], "ambient": {{"show": "{show}"}}}}"#);
            let (status, after) = put_body_over(existing, &body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{show}");
            assert_eq!(after, existing, "{show}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_puts_to_one_playlist_all_save() {
        // #578: two editors saving at once shared the write's sidecar, and
        // all but one got a 500.
        let (state, _dir) = test_state();
        // On disk, so it is still there after each save's rescan.
        let song_dir = state.songs_path().join("song_a");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("kick.wav"), vec![vec![0_i32; 441]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: Song A\ntracks:\n- name: kick\n  file: kick.wav\n",
        )
        .unwrap();
        state.player.reload_songs().await.unwrap();
        let puts: Vec<_> = (0..12)
            .map(|i| {
                let app = router().with_state(state.clone());
                let songs = if i % 2 == 0 {
                    "Song A"
                } else {
                    "Song A\", \"Song A"
                };
                tokio::spawn(async move {
                    app.oneshot(
                        http::Request::builder()
                            .method("PUT")
                            .uri("/playlists/mylist")
                            .header("content-type", "application/json")
                            .body(Body::from(format!(r#"{{"songs": ["{songs}"]}}"#)))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
                    .status()
                })
            })
            .collect();
        for put in puts {
            assert_eq!(put.await.unwrap(), StatusCode::OK);
        }
    }

    /// The playlist file `text` reads as.
    fn read(text: &str) -> config::Playlist {
        ::config::Config::builder()
            .add_source(::config::File::from_str(text, ::config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[tokio::test]
    async fn put_playlist_sets_auto_play_and_ambient() {
        let (status, after) = put_body_over(
            "songs:\n- song: Song A\n  fade_out: 2s\n- Song B\n",
            r#"{"songs": ["Song B", "Song A", "Song C"],
                "auto_play": [true, null, "1500ms"],
                "ambient": {"show": "lighting/walk_in.light", "fade_in": "6s"}}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{after}");
        let gaps: Vec<_> = read(&after)
            .entry_transitions()
            .unwrap()
            .into_iter()
            .map(|t| t.auto_play)
            .collect();
        use std::time::Duration;
        assert_eq!(
            gaps,
            vec![
                Some(Duration::ZERO),
                None,
                Some(Duration::from_millis(1500))
            ]
        );
        // Song A keeps its fade, and turning auto play off leaves no setting.
        assert!(after.contains("fade_out: 2s"), "{after}");
        assert!(!after.contains("auto_play: false"), "{after}");
        assert!(after.contains("show: lighting/walk_in.light"), "{after}");
        assert!(after.contains("fade_in: 6s"), "{after}");
    }

    #[tokio::test]
    async fn put_playlist_keeps_or_clears_ambient() {
        let existing = "ambient: false\nsongs:\n- song: Song A\n  auto_play: 2s\n";
        // Left out: kept, and each entry's auto play with it.
        let (status, after) = put_body_over(existing, r#"{"songs": ["Song A"]}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(after.contains("ambient: false"), "{after}");
        assert!(after.contains("auto_play: 2s"), "{after}");
        // Null: removed, so the profile's plays.
        let (status, after) =
            put_body_over(existing, r#"{"songs": ["Song A"], "ambient": null}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!after.contains("ambient"), "{after}");
    }

    #[tokio::test]
    async fn put_playlist_refuses_a_bad_gap_or_ambient() {
        let existing = "songs:\n- Song A\n";
        for body in [
            r#"{"songs": ["Song A"], "auto_play": ["soon"]}"#,
            r#"{"songs": ["Song A"], "ambient": {"show": "a.light", "delay": "later"}}"#,
            r#"{"songs": ["Song A"], "ambient": {"shw": "a.light"}}"#,
        ] {
            let (status, after) = put_body_over(existing, body).await;
            assert!(status.is_client_error(), "{body}: {status}");
            assert_eq!(after, existing, "{body}");
        }
    }

    #[tokio::test]
    async fn put_playlist_sets_each_entrys_ambient() {
        let (status, after) = put_body_over(
            "songs:\n- Song A\n- song: Song B\n  auto_play: true\n",
            r#"{"songs": ["Song A", "Song B", "Song C"],
                "entry_ambient": [null, false, {"show": "lighting/encore.light", "fade_in": "6s"}]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{after}");
        let written = read(&after);
        let own = config::ambient::PlaylistAmbient::On(config::ambient::Ambient::new(
            Some("lighting/encore.light".to_string()),
            None,
            Some("6s".to_string()),
            None,
        ));
        assert_eq!(
            written.entry_ambient(),
            vec![None, Some(config::ambient::PlaylistAmbient::Off), Some(own)]
        );
        // Song A sets nothing, and is written as its name alone; Song B
        // keeps its auto play.
        assert!(after.contains("- Song A\n"), "{after}");
        assert!(after.contains("auto_play: true"), "{after}");

        // Left out, each entry keeps its own, with its song.
        let (status, again) = put_body_over(&after, r#"{"songs": ["Song C", "Song A"]}"#).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        let kept = read(&again).entry_ambient();
        assert!(
            matches!(kept[0], Some(config::ambient::PlaylistAmbient::On(_))),
            "{again}"
        );
        assert_eq!(kept[1], None, "{again}");
    }

    #[tokio::test]
    async fn put_playlist_refuses_a_bad_entry_ambient() {
        let existing = "songs:\n- Song A\n";
        for body in [
            r#"{"songs": ["Song A"], "entry_ambient": [false, null]}"#,
            r#"{"songs": ["Song A"], "entry_ambient": [{"show": "a.light", "delay": "later"}]}"#,
            r#"{"songs": ["Song A"], "entry_ambient": [{"shw": "a.light"}]}"#,
            r#"{"songs": ["Song A"], "entry_ambient": [{"show": "../x.light"}]}"#,
            r#"{"songs": ["Song A"], "entry_ambient": [{"show": "/etc/passwd"}]}"#,
        ] {
            let (status, after) = put_body_over(existing, body).await;
            assert!(status.is_client_error(), "{body}: {status}");
            assert_eq!(after, existing, "{body}");
        }
    }

    #[tokio::test]
    async fn put_playlist_sets_fades_that_follow_their_songs() {
        let (status, after) = put_body_over(
            "songs:\n- song: Song A\n  fade_in: 2s\n  auto_play: true\n- Song B\n",
            r#"{"songs": ["Song B", "Song A"],
                "fades": [
                  {"fade_in": "4beats"},
                  {"fade_out": {"length": "1measure", "curve": "linear",
                                "exclude": ["kick"], "lights": "0s"},
                   "fade_in": null}
                ]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{after}");
        let written = read(&after);
        assert_eq!(written.songs(), ["Song B", "Song A"]);
        assert_eq!(
            written.entry_fades(),
            vec![
                config::EntryFades {
                    fade_out: None,
                    fade_in: Some(config::FadeConfig::Length("4beats".to_string())),
                },
                config::EntryFades {
                    fade_out: Some(config::FadeConfig::Full {
                        length: "1measure".to_string(),
                        curve: Some("linear".to_string()),
                        exclude: Some(vec!["kick".to_string()]),
                        lights: Some("0s".to_string()),
                    }),
                    fade_in: None,
                },
            ]
        );
        // Song A's auto play stays with it.
        assert!(after.contains("auto_play: true"), "{after}");
    }

    #[tokio::test]
    async fn put_playlist_keeps_or_clears_fades() {
        let existing = "songs:\n- song: Song A\n  fade_out: 2s\n  fade_in: 1s\n";
        // Left out: kept.
        let (status, after) = put_body_over(existing, r#"{"songs": ["Song A"]}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(after.contains("fade_out: 2s"), "{after}");
        assert!(after.contains("fade_in: 1s"), "{after}");
        // Null: removed, leaving the song's name alone.
        let (status, after) = put_body_over(
            existing,
            r#"{"songs": ["Song A"], "fades": [{"fade_out": null, "fade_in": null}]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!after.contains("fade"), "{after}");
        assert!(after.contains("- Song A"), "{after}");
    }

    #[tokio::test]
    async fn put_playlist_refuses_a_bad_fade() {
        let existing = "songs:\n- Song A\n";
        for fade in [
            r#"{"fade_out": "soon"}"#,
            r#"{"fade_out": {"length": "2s", "curve": "wobbly"}}"#,
            r#"{"fade_out": {"length": "2s", "exclude": ["cowbell"]}}"#,
            r#"{"fade_in": {"length": "2s", "exclude": ["cowbell"]}}"#,
            r#"{"fade_in": {"length": "2s", "lights": "1s"}}"#,
            r#"{"fade_out": {"length": "2s", "exlude": ["kick"]}}"#,
            r#"{"fade_ot": "2s"}"#,
        ] {
            let body = format!(r#"{{"songs": ["Song A"], "fades": [{fade}]}}"#);
            let (status, after) = put_body_over(existing, &body).await;
            assert!(status.is_client_error(), "{fade}: {status}");
            assert_eq!(after, existing, "{fade}");
        }
    }

    #[tokio::test]
    async fn get_playlist_reports_its_fades_and_each_songs_tracks() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path().join("song_a");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("kick.wav"), vec![vec![0_i32; 441]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: Song A\ntracks:\n- name: kick\n  file: kick.wav\n",
        )
        .unwrap();
        std::fs::write(
            state.playlists_dir().clone().unwrap().join("testlist.yaml"),
            "songs:\n- song: Song A\n  fade_out: {length: 1measure, exclude: [kick]}\n- Song A\n",
        )
        .unwrap();
        state.player.reload_songs().await.unwrap();
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
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(
            parsed["fades"],
            serde_json::json!([
                {"fade_out": {"length": "1measure", "exclude": ["kick"]}, "fade_in": null},
                {"fade_out": null, "fade_in": null},
            ])
        );
        assert_eq!(parsed["song_tracks"]["Song A"], serde_json::json!(["kick"]));
    }

    #[test]
    fn gaps_are_written_as_a_playlist_would() {
        use std::time::Duration;
        assert_eq!(format_gap(Duration::from_secs(2)), "2s");
        assert_eq!(format_gap(Duration::from_millis(1500)), "1500ms");
    }

    #[tokio::test]
    async fn put_playlist_keeps_its_transitions() {
        let (status, after) = put_over(
            "songs:\n- song: Song A\n  fade_out: 2s\n  fade_in: 1s\n",
            r#"["Song B", "Song A"]"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(after.contains("fade_out: 2s"), "{after}");
        assert!(after.contains("fade_in: 1s"), "{after}");
        assert!(after.contains("- Song B"), "{after}");
    }

    #[tokio::test]
    async fn put_playlist_wont_overwrite_one_that_does_not_read() {
        let broken = "songs:\n- song: Song A\n  fade_ot: 1s\n";
        let (status, after) = put_over(broken, r#"["Song A"]"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(after, broken);
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
    async fn playlists_already_on_disk_stay_savable_and_deletable() {
        // Names from before #578's rules for new names: they load and are
        // listed, and the editor must still save and delete them. A new
        // playlist cannot be given one.
        for (file, uri) in [
            (".warmup.yaml", "/playlists/.warmup"),
            ("Encore .yaml", "/playlists/Encore%20"),
        ] {
            let (state, _dir) = test_state();
            let playlists_dir = state.playlists_dir().clone().unwrap();
            let app = router().with_state(state);
            let send = |method: &str, body: &'static str| {
                let request = http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap();
                app.clone().oneshot(request)
            };
            let created = send("PUT", r#"{"songs": ["Song A"]}"#).await.unwrap();
            assert_eq!(created.status(), StatusCode::BAD_REQUEST, "{file}");
            assert!(!playlists_dir.join(file).exists(), "{file}");

            std::fs::write(playlists_dir.join(file), "songs:\n  - \"Song A\"\n").unwrap();
            let saved = send("PUT", r#"{"songs": ["Song A", "Song A"]}"#)
                .await
                .unwrap();
            assert_eq!(saved.status(), StatusCode::OK, "{file}");
            let deleted = send("DELETE", "").await.unwrap();
            assert_eq!(deleted.status(), StatusCode::OK, "{file}");
            assert!(!playlists_dir.join(file).exists(), "{file}");
        }
    }

    #[tokio::test]
    async fn delete_playlist_by_name_success() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir().clone().unwrap();
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

    /// A switch refused because a song plays is a conflict, told apart from
    /// a playlist not found by the error's kind, not its message.
    #[tokio::test(flavor = "multi_thread")]
    async fn activate_playlist_while_playing_is_a_conflict() {
        let (state, _dir) = test_state_with_audio_device("mock-device");
        state.player.play().await.unwrap();
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/playlists/all_songs/activate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        state.player.stop().await;
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
        let playlists_dir = state.playlists_dir().clone().unwrap();
        // Use an empty song list so the playlist loads without requiring actual songs on disk.
        std::fs::write(playlists_dir.join("testlist.yaml"), "songs: []\n").unwrap();

        state.player.reload_songs().await.unwrap();

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
        assert!(parsed["auto_play"].is_array());
        assert!(parsed["ambient"].is_null());
        assert!(parsed["available_songs"].is_array());
    }

    #[tokio::test]
    async fn get_playlist_reports_its_ambient() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir().clone().unwrap();
        std::fs::write(
            playlists_dir.join("testlist.yaml"),
            "songs: []\nambient:\n  show: a.light\n  delay: 3s\n",
        )
        .unwrap();
        state.player.reload_songs().await.unwrap();
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
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(
            parsed["ambient"],
            serde_json::json!({"show": "a.light", "delay": "3s"})
        );
    }

    #[tokio::test]
    async fn get_playlist_by_name_yml_extension() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir().clone().unwrap();
        // Write a .yml file (not .yaml) with an empty song list.
        std::fs::write(playlists_dir.join("ymllist.yml"), "songs: []\n").unwrap();

        state.player.reload_songs().await.unwrap();

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
