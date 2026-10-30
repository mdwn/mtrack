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
use super::helpers::{
    require_configured_dir, resolve_resource_path, validate_resource_name,
    validate_resource_name_to_write,
};
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
    let registry = state.player.songs();
    let all_songs: Vec<String> = registry
        .sorted_list()
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    // Each song's tracks, for picking the ones a fade leaves out.
    let song_tracks: serde_json::Map<String, serde_json::Value> = registry
        .sorted_list()
        .iter()
        .map(|s| (s.name().to_string(), json!(s.output_track_names())))
        .collect();
    // Each entry's auto play as the editor writes it: `true` for at once,
    // a gap, or null for none.
    let auto_play: Vec<serde_json::Value> = pl
        .entry_auto_play()
        .into_iter()
        .map(|gap| match gap {
            None => serde_json::Value::Null,
            Some(gap) if gap.is_zero() => json!(true),
            Some(gap) => json!(format_gap(gap)),
        })
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "name": name,
            "songs": pl.songs(),
            "auto_play": auto_play,
            "fades": pl.entry_fades(),
            "ambient": pl.ambient(),
            "available_songs": all_songs,
            "song_tracks": song_tracks,
        })),
    )
        .into_response()
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

#[derive(serde::Deserialize)]
pub(super) struct PlaylistBody {
    songs: Vec<String>,
    /// Each song's auto play, in the order of `songs`; absent keeps each
    /// entry's as it was.
    #[serde(default)]
    auto_play: Option<Vec<Option<config::AutoPlayConfig>>>,
    /// Each song's fade out and fade in, in the order of `songs`, null for
    /// none; absent keeps each entry's as it was.
    #[serde(default)]
    fades: Option<Vec<config::EntryFades>>,
    /// The playlist's ambient light: absent keeps it, null removes it (the
    /// profile's plays), `false` turns it off, a map overrides settings.
    #[serde(default, deserialize_with = "present")]
    ambient: Option<Option<config::ambient::PlaylistAmbient>>,
}

/// A field that is there, null or not: `Some(None)` for null, against the
/// `None` of a field left out.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

/// Held by each playlist save and delete from reading the file to reloading
/// the player, so two at once (two editor tabs, say) take turns: a save reads
/// the file to keep what the editor does not send, and one reading it while
/// another rewrote it found it short and refused (#578).
static PLAYLIST_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// PUT /api/playlists/:name — create or update a playlist.
pub(super) async fn put_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Json(body): Json<PlaylistBody>,
) -> impl IntoResponse {
    validate_playlist_name(&name)?;
    let _writing = PLAYLIST_WRITES.lock().await;
    let playlists_dir =
        require_configured_dir(&state.playlists_dir, "playlists", StatusCode::NOT_FOUND)?;

    // Before `resolve_resource_path`, which canonicalizes the directory and
    // fails if it is missing -- a creation after that call can never run.
    // Created only when it is inside the project: a `playlists_dir:` pointing
    // elsewhere is the operator's to set up, like `songs` and the rest.
    let playlists_dir = super::helpers::ensure_configured_dir(&playlists_dir, &state).await?;

    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in resolve_resource_path.
    let file_path = resolve_resource_path(&playlists_dir, &name, "yaml")?;
    validate_resource_name_to_write(&name, "playlist", file_path.is_file())?;

    // The editor knows only the songs: an existing playlist keeps its
    // transitions, and each entry its own with the song. One that does not
    // read is not overwritten, which would lose them.
    let playlist_config = if file_path.is_file() {
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
    let playlist_config = match body.ambient {
        Some(ambient) => playlist_config.with_ambient(ambient),
        None => playlist_config,
    };
    // What the player would refuse to load is refused here, before the file
    // is written: a song it does not have, a bad fade or ambient, or a fade
    // leaving out a track its song does not have. So is a fade the player
    // would cut short, longer than the song it plays under, and an ambient
    // show outside the project.
    if let Err(e) = crate::playlist::validate_for_write(&playlist_config, &state.player.songs()) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": e.to_string()})),
        )
            .into_response());
    }
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

    // Write atomically, off the async runtime.
    let fp = file_path.clone();
    let yaml_owned = yaml;
    super::helpers::spawn_blocking_io("write playlist", move || {
        config_io::staged_write(&fp, &yaml_owned)
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
    let _writing = PLAYLIST_WRITES.lock().await;
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
        let written = std::fs::read_to_string(playlists_dir.join("mylist.yaml")).unwrap();
        assert!(written.ends_with('\n'), "{written:?}");
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
        let path = state.playlists_dir.clone().unwrap().join("mylist.yaml");
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
        let song_dir = state.songs_path.join("song_a");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("kick.wav"), vec![vec![0_i32; 441]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: Song A\ntracks:\n- name: kick\n  file: kick.wav\n",
        )
        .unwrap();
        state.player.reload_songs(&state.songs_path, None, None);
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
        let song_dir = state.songs_path.join("song_a");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("kick.wav"), vec![vec![0_i32; 441]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: Song A\ntracks:\n- name: kick\n  file: kick.wav\n",
        )
        .unwrap();
        std::fs::write(
            state.playlists_dir.clone().unwrap().join("testlist.yaml"),
            "songs:\n- song: Song A\n  fade_out: {length: 1measure, exclude: [kick]}\n- Song A\n",
        )
        .unwrap();
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
            let playlists_dir = state.playlists_dir.clone().unwrap();
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
        assert!(parsed["auto_play"].is_array());
        assert!(parsed["ambient"].is_null());
        assert!(parsed["available_songs"].is_array());
    }

    #[tokio::test]
    async fn get_playlist_reports_its_ambient() {
        let (state, _dir) = test_state();
        let playlists_dir = state.playlists_dir.clone().unwrap();
        std::fs::write(
            playlists_dir.join("testlist.yaml"),
            "songs: []\nambient:\n  show: a.light\n  delay: 3s\n",
        )
        .unwrap();
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
