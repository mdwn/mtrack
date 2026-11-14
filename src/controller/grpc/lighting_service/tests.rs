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

//! `LightingService`, called as the service: the behaviour of the REST
//! routes it replaced, each held by a test here.

mod facts;
mod fixture;
mod json;
mod mvr;
mod settings;
mod type_paths;
mod type_writes;
mod types;
mod uploads;
mod venue_paths;
mod venue_writes;
mod venues;

use std::path::PathBuf;
use std::sync::Arc;

use tonic::{Code, Request, Status};

use super::LightingServer;
use crate::config;
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};
use crate::webui::api::test_helpers::{test_state, test_state_with_registry};
use crate::webui::server::WebUiState;

/// A project on disk (`mtrack.yaml`, an empty `songs/` and the player's
/// "Song A", held in memory), and the service over its player, which has a
/// config store and is unlocked.
pub(super) struct Project {
    pub state: WebUiState,
    pub dir: tempfile::TempDir,
    pub server: LightingServer,
}

/// The service over `state`, whose player gets a config store over its
/// config file and is unlocked.
pub(super) fn serve(state: WebUiState, dir: tempfile::TempDir) -> Project {
    let store = Arc::new(config::ConfigStore::new(
        config::Player::deserialize(&state.config_path).unwrap(),
        state.config_path.clone(),
    ));
    state.player.set_config_store(store);
    state.player.set_locked(false);
    Project {
        server: LightingServer::new(state.player.clone()),
        state,
        dir,
    }
}

pub(super) fn project() -> Project {
    let (state, dir) = test_state();
    serve(state, dir)
}

const SHOW: &str =
    "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";

impl Project {
    pub fn songs(&self) -> PathBuf {
        self.state.songs_path()
    }

    pub async fn get(&self, name: &str) -> Result<String, Status> {
        self.server
            .get_light_show(Request::new(proto::GetLightShowRequest {
                name: name.to_string(),
            }))
            .await
            .map(|r| r.into_inner().source)
    }

    pub async fn save(&self, name: &str, source: &str) -> Result<(), Status> {
        self.server
            .save_light_show(Request::new(proto::SaveLightShowRequest {
                name: name.to_string(),
                source: source.to_string(),
            }))
            .await
            .map(|_| ())
    }

    pub async fn delete(&self, name: &str) -> Result<(), Status> {
        self.server
            .delete_light_show(Request::new(proto::DeleteLightShowRequest {
                name: name.to_string(),
            }))
            .await
            .map(|_| ())
    }

    pub async fn validate(
        &self,
        source: &str,
        song: Option<&str>,
        file: Option<&str>,
    ) -> Result<proto::ValidateLightShowResponse, Status> {
        self.server
            .validate_light_show(Request::new(proto::ValidateLightShowRequest {
                source: source.to_string(),
                song: song.map(str::to_string),
                file: file.map(str::to_string),
            }))
            .await
            .map(|r| r.into_inner())
    }

    pub async fn groups(&self) -> Result<proto::ListLightingGroupsResponse, Status> {
        self.server
            .list_lighting_groups(Request::new(proto::ListLightingGroupsRequest {}))
            .await
            .map(|r| r.into_inner())
    }

    /// Rewrites the player's config file (the store keeps what it loaded).
    pub fn write_config(&self, yaml: &str) {
        std::fs::write(&self.state.config_path, yaml).unwrap();
    }
}

// ---- ListLightingGroups ----

/// With no DMX device, the group names still come back — from the config.
///
/// Authoring on a laptop is the normal case for editing a show, and the
/// groups are declared under `dmx.lighting.groups` in the player config, not
/// by the engine. Answering no groups there emptied the cue-target
/// autocomplete with no error to explain it, because the endpoint only read
/// them off a live engine.
#[tokio::test]
async fn lighting_groups_come_from_this_hosts_profile() {
    let p = project();
    p.write_config(
        r#"songs: songs
profiles:
  - hostname: not-this-host-x9
    dmx:
      universes: []
      lighting:
        groups:
          elsewhere:
            name: elsewhere
            constraints:
              - AllOf: ["x"]
  - dmx:
      universes: []
      lighting:
        groups:
          here:
            name: here
            constraints:
              - AllOf: ["y"]
"#,
    );
    let names: Vec<String> = p
        .groups()
        .await
        .unwrap()
        .groups
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(names, vec!["here"]);
}

#[tokio::test]
async fn lighting_groups_come_from_the_config_without_a_dmx_engine() {
    let p = project();
    p.write_config(
        "songs: songs\ndmx:\n  universes:\n    - universe: 1\n      name: main\n  \
         lighting:\n    groups:\n      front_wash:\n        \
         name: front_wash\n        constraints:\n          - AllOf: [\"wash\"]\n      \
         back_wash:\n        name: back_wash\n        constraints:\n          \
         - AllOf: [\"back\"]\n",
    );
    let answer = p.groups().await.unwrap();
    let names: Vec<&str> = answer.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["back_wash", "front_wash"],
        "the declared groups must be listed even with no engine to resolve them"
    );
    // Flagged as unresolved: the names are knowable without an engine, the
    // fixtures each resolves to are not, and a caller has to be able to tell
    // "no fixtures" from "not asked yet".
    assert_eq!(answer.resolved, Some(false));
    assert!(answer.groups.iter().all(|g| g.fixtures.is_empty()));
    assert_eq!(json::groups(&answer)["resolved"], serde_json::json!(false));
}

/// A config that will not load is reported, not answered as "no groups".
///
/// The fallback exists because an empty list was indistinguishable from a
/// rig with nothing declared. Swallowing a parse error with `.ok()` put that
/// exact ambiguity back one layer down — a missing `dmx.universes` read as
/// an empty autocomplete with no explanation.
#[tokio::test]
async fn a_config_that_does_not_load_is_reported_not_answered_empty() {
    let p = project();
    // `dmx.universes` is required, so this parses as YAML and fails to load
    // as config — the shape most likely to be hit by hand-editing.
    p.write_config(
        "songs: songs\ndmx:\n  lighting:\n    groups:\n      wash:\n        name: wash\n",
    );
    let status = p.groups().await.unwrap_err();
    assert_eq!(status.code(), Code::Internal);
    assert!(
        status.message().contains("could not be read"),
        "the reason must reach the caller: {}",
        status.message()
    );
}

#[tokio::test]
async fn a_player_with_no_config_store_has_no_groups_to_read() {
    let (state, _dir) = test_state();
    let server = LightingServer::new(state.player.clone());
    let status = server
        .list_lighting_groups(Request::new(proto::ListLightingGroupsRequest {}))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::Unimplemented);
}

// ---- GetLightShow ----

#[tokio::test]
async fn a_light_show_is_read() {
    let p = project();
    std::fs::write(p.songs().join("show.light"), "light content").unwrap();
    assert_eq!(p.get("show.light").await.unwrap(), "light content");
}

#[tokio::test]
async fn a_light_show_that_is_not_there_is_not_found_by_name() {
    let p = project();
    let status = p.get("missing.light").await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert!(
        status.message().contains("Lighting file not found"),
        "{}",
        status.message()
    );
    assert!(status.message().contains("missing.light"));
}

#[tokio::test]
async fn an_unreadable_light_show_is_an_internal_error_saying_so() {
    use std::os::unix::fs::PermissionsExt;
    let p = project();
    let file = p.songs().join("broken.light");
    std::fs::write(&file, "content").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = p.get("broken.light").await;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    // Root reads what it likes.
    if let Err(status) = result {
        assert_eq!(status.code(), Code::Internal);
        assert!(
            status.message().contains("Failed to read lighting file"),
            "{}",
            status.message()
        );
    }
}

// ---- Names that leave the songs directory ----

/// A project and, outside its songs directory, a light show and a folder
/// the names below lead to by `..`, by an absolute path and by a link.
struct Outside {
    p: Project,
    /// A folder beside the songs directory (inside the project).
    sibling: PathBuf,
    /// A folder outside the project.
    away: tempfile::TempDir,
}

impl Outside {
    fn new() -> Self {
        let p = project();
        let sibling = p.dir.path().join("sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("secret.light"), "secret content").unwrap();
        let away = tempfile::tempdir().unwrap();
        std::fs::write(away.path().join("secret.light"), "secret content").unwrap();
        let songs = p.songs();
        // A link to a file out of the root, and one to a folder.
        std::os::unix::fs::symlink(away.path().join("secret.light"), songs.join("evil.light"))
            .unwrap();
        std::os::unix::fs::symlink(away.path(), songs.join("escape")).unwrap();
        Self { p, sibling, away }
    }

    /// Every spelling of a file that is outside the songs directory.
    fn names(&self) -> Vec<String> {
        vec![
            "../sibling/secret.light".to_string(),
            "../../etc/passwd".to_string(),
            "..".to_string(),
            "a/../../sibling/secret.light".to_string(),
            self.sibling.join("secret.light").display().to_string(),
            self.away.path().join("secret.light").display().to_string(),
            "/etc/passwd".to_string(),
            "evil.light".to_string(),
            "escape/secret.light".to_string(),
        ]
    }

    fn untouched(&self) {
        for file in [
            self.sibling.join("secret.light"),
            self.away.path().join("secret.light"),
        ] {
            assert_eq!(std::fs::read_to_string(file).unwrap(), "secret content");
        }
        assert!(!self.p.songs().join("../sibling/new.light").exists());
        assert!(!self.away.path().join("new.light").exists());
    }
}

fn is_refusal(status: &Status) -> bool {
    matches!(
        status.code(),
        Code::NotFound | Code::PermissionDenied | Code::InvalidArgument
    )
}

#[tokio::test]
async fn a_light_show_is_never_read_from_outside_the_songs_directory() {
    let o = Outside::new();
    for name in o.names() {
        let status = o.p.get(&name).await.expect_err(&name);
        assert_eq!(status.code(), Code::NotFound, "{name}");
    }
    o.untouched();
}

#[tokio::test]
async fn a_light_show_is_never_written_outside_the_songs_directory() {
    let o = Outside::new();
    let mut names = o.names();
    // Names for a file that is not there yet.
    names.extend([
        "../sibling/new.light".to_string(),
        o.sibling.join("new.light").display().to_string(),
        o.away.path().join("new.light").display().to_string(),
        "escape/new.light".to_string(),
    ]);
    for name in names {
        let status = o.p.save(&name, SHOW).await.expect_err(&name);
        assert!(is_refusal(&status), "{name}: {status:?}");
    }
    o.untouched();
    assert!(!o.sibling.join("new.light").exists());
}

#[tokio::test]
async fn a_light_show_is_never_deleted_outside_the_songs_directory() {
    let o = Outside::new();
    for name in o.names() {
        // `evil.light` is a link in the songs directory: the link goes,
        // never what it leads to (the next test holds that).
        if name == "evil.light" {
            continue;
        }
        let status = o.p.delete(&name).await.expect_err(&name);
        assert!(is_refusal(&status), "{name}: {status:?}");
    }
    o.untouched();
}

#[tokio::test]
async fn a_link_out_of_the_songs_directory_is_deleted_as_the_link() {
    let o = Outside::new();
    o.p.delete("evil.light").await.unwrap();
    assert!(std::fs::symlink_metadata(o.p.songs().join("evil.light")).is_err());
    o.untouched();
}

#[tokio::test]
async fn a_draft_is_never_validated_as_a_file_outside_the_songs_directory() {
    let o = Outside::new();
    for name in o.names() {
        // A name that is a file of the songs directory (the link) is
        // resolved where it is, as saving it would be.
        if name == "evil.light" {
            continue;
        }
        let status =
            o.p.validate(SHOW, None, Some(&name))
                .await
                .expect_err(&name);
        assert!(is_refusal(&status), "{name}: {status:?}");
    }
    o.untouched();
}

// ---- ValidateLightShow ----

#[tokio::test]
async fn a_valid_light_show_is_valid() {
    let p = project();
    let answer = p.validate(SHOW, None, None).await.unwrap();
    assert!(answer.valid);
    assert!(answer.errors.is_empty());
}

/// Validation answers normally with the problems: it is a check, not a
/// refusal.
#[tokio::test]
async fn an_invalid_light_show_answers_with_its_problems() {
    let p = project();
    let answer = p.validate("invalid {{{ content", None, None).await.unwrap();
    assert!(!answer.valid);
    assert!(!answer.errors.is_empty());
}

#[tokio::test]
async fn validation_resolves_includes_from_the_named_file() {
    let p = project();
    let songs = p.songs();
    std::fs::create_dir_all(songs.join("song/lighting")).unwrap();
    std::fs::write(
        songs.join("common.light"),
        "sequence \"wash\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n",
    )
    .unwrap();
    std::fs::write(songs.parent().unwrap().join("outside.light"), "").unwrap();
    let show = "include \"../../common.light\"\nshow \"test\" {\n    @00:00.000\n    sequence \"wash\"\n}\n";

    let ok = p
        .validate(show, None, Some("song/lighting/show.light"))
        .await
        .unwrap();
    assert!(ok.valid, "{:?}", ok.errors);
    // From another folder the same include misses.
    let missed = p
        .validate(show, None, Some("song/show.light"))
        .await
        .unwrap();
    assert!(!missed.valid);
    // And it may not leave the songs folder.
    let outside = p
        .validate(
            "include \"../../outside.light\"\n",
            None,
            Some("song/show.light"),
        )
        .await
        .unwrap();
    assert!(!outside.valid);
    assert!(
        outside
            .errors
            .iter()
            .any(|e| e.contains("outside the songs folder")),
        "{:?}",
        outside.errors
    );
}

/// Measure-based timing does not parse without the song's tempo map; named
/// with its song, it does.
#[tokio::test]
async fn validation_takes_the_named_songs_tempo() {
    let p = project();
    let bars = "show \"bars\" {\n    @1/1\n    lights: static color: \"red\", duration: 1beat\n}\n";
    let without = p.validate(bars, None, None).await.unwrap();
    assert!(!without.valid);
    // A song that is not there is checked as if none was named.
    let unknown = p.validate(bars, Some("No Such Song"), None).await.unwrap();
    assert!(!unknown.valid);
}

// ---- SaveLightShow ----

#[tokio::test]
async fn a_light_show_is_saved() {
    let p = project();
    p.save("new.light", SHOW).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(p.songs().join("new.light")).unwrap(),
        SHOW
    );
    assert_eq!(p.get("new.light").await.unwrap(), SHOW);
}

/// The editor shows what is wrong with a show it cannot save as the one
/// message it always showed: each problem (`Invalid light show DSL: ...`),
/// joined by "; ".
#[tokio::test]
async fn a_light_show_that_does_not_parse_is_refused_with_every_problem() {
    let p = project();
    let status = p
        .save("test.light", "invalid {{{ content")
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status.message().starts_with("Invalid light show DSL: "),
        "{}",
        status.message()
    );
    let problems = p
        .validate("invalid {{{ content", None, None)
        .await
        .unwrap()
        .errors;
    let joined = problems
        .iter()
        .map(|e| format!("Invalid light show DSL: {e}"))
        .collect::<Vec<_>>()
        .join("; ");
    assert_eq!(status.message(), joined);
    assert!(!p.songs().join("test.light").exists());
}

#[tokio::test]
async fn a_light_show_goes_in_a_folder_that_is_there() {
    let p = project();
    let status = p.save("nowhere/show.light", SHOW).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

/// Only `.light` files are written as light shows: not a song's config by
/// name.
#[tokio::test]
async fn only_a_light_file_is_written_as_a_light_show() {
    let p = project();
    let songs = p.songs();
    std::fs::create_dir_all(songs.join("b")).unwrap();
    std::fs::write(songs.join("b/song.yaml"), "name: B\n").unwrap();
    let status = p.save("b/song.yaml", SHOW).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(
        std::fs::read_to_string(songs.join("b/song.yaml")).unwrap(),
        "name: B\n"
    );
}

#[tokio::test]
async fn a_write_that_fails_is_an_internal_error() {
    use std::os::unix::fs::PermissionsExt;
    let p = project();
    let sub = p.songs().join("readonly");
    std::fs::create_dir(&sub).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = p.save("readonly/test.light", SHOW).await;
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root writes where it likes.
    if let Err(status) = result {
        assert_eq!(status.code(), Code::Internal);
        assert!(!status.message().is_empty());
    }
}

/// A bar/beat show is checked against its song's tempo when the song's
/// folder is spelled through a link (the songs folder reached through one):
/// the show's path, resolved, and the song's folder are compared resolved.
/// The loader resolves the folders it loads, so this is a song built from an
/// unresolved path.
#[tokio::test]
async fn a_show_finds_the_songs_tempo_through_a_linked_folder() {
    let outer = tempfile::tempdir().unwrap();
    let real = outer.path().join("real");
    std::fs::create_dir_all(real.join("t/lighting")).unwrap();
    std::fs::write(
        real.join("t/song.yaml"),
        "name: T\ntracks: []\ntempo:\n  bpm: 120\n  time_signature: 4/4\n",
    )
    .unwrap();
    let link = outer.path().join("songs-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let song_config = crate::config::Song::deserialize(&link.join("t/song.yaml")).unwrap();
    let song = crate::songs::Song::new(&link.join("t"), &song_config).unwrap();
    assert!(song.lighting_tempo_map().is_some());
    let songs = crate::songs::Songs::new(std::collections::HashMap::from([(
        "T".to_string(),
        std::sync::Arc::new(song),
    )]));
    let (state, dir) = test_state_with_registry(std::sync::Arc::new(songs));
    let p = serve(state, dir);
    // After the config store, which the player takes its song sources from.
    {
        let mut sources = p.state.player.song_sources().unwrap_or_default();
        sources.songs = link;
        p.state.player.set_song_sources(sources);
    }
    let show = "show \"bars\" {\n    @1/1\n    lights: static color: \"red\", duration: 1beat\n}\n";
    p.save("t/lighting/show.light", show).await.unwrap();
}

/// A light show that is a link leading nowhere is not written: the write
/// would make its target, outside the songs folder.
#[tokio::test]
async fn a_link_leading_nowhere_is_not_written_through() {
    let p = project();
    let songs = p.songs();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("x.light");
    std::fs::create_dir_all(songs.join("foo/lighting")).unwrap();
    std::os::unix::fs::symlink(&target, songs.join("foo/lighting/show.light")).unwrap();
    let status = p.save("foo/lighting/show.light", SHOW).await.unwrap_err();
    assert_eq!(status.code(), Code::PermissionDenied);
    assert!(!target.exists(), "written outside");
}

/// A light show saved through a link has its `include`s resolved where the
/// song loader resolves them: beside the link, not beside what it leads to.
#[tokio::test]
async fn includes_are_resolved_beside_a_link() {
    let p = project();
    let songs = p.songs();
    std::fs::create_dir_all(songs.join("a/lighting")).unwrap();
    std::fs::create_dir_all(songs.join("b/lighting")).unwrap();
    let shared =
        "sequence \"wash\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
    std::fs::write(songs.join("a/lighting/common.light"), shared).unwrap();
    std::fs::write(songs.join("a/lighting/show.light"), "").unwrap();
    std::os::unix::fs::symlink(
        "../../a/lighting/show.light",
        songs.join("b/lighting/show.light"),
    )
    .unwrap();
    let show =
        "include \"common.light\"\nshow \"test\" {\n    @00:00.000\n    sequence \"wash\"\n}\n";

    // Beside B's link there is no common.light: the loader would fail.
    let status = p.save("b/lighting/show.light", show).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(
        std::fs::read_to_string(songs.join("a/lighting/show.light")).unwrap(),
        ""
    );
    // Saved as A's, it is found.
    p.save("a/lighting/show.light", show).await.unwrap();
}

// ---- The venues and fixture types directories ----

/// A project whose songs root is the project itself (`songs: "."`, as on the
/// harness), so the light show calls reach the lighting directories; `config`
/// is its mtrack.yaml.
fn light_shows_at_the_root(config: &str) -> Project {
    let p = project();
    p.write_config(config);
    let mut sources = p.state.player.song_sources().unwrap_or_default();
    sources.songs = p.dir.path().to_path_buf();
    p.state.player.set_song_sources(sources);
    for sub in [
        "lighting/venues",
        "lighting/fixture_types",
        "lighting/x",
        "shows",
    ] {
        std::fs::create_dir_all(p.dir.path().join(sub)).unwrap();
    }
    std::fs::write(
        p.dir.path().join("lighting/venues/club.light"),
        "venue \"club\" {\n  fixture \"A\" Par @ 1:1\n}\n",
    )
    .unwrap();
    p
}

#[tokio::test]
async fn a_light_show_is_never_a_venue_or_fixture_type_file() {
    let p = light_shows_at_the_root("songs: \".\"\n");
    let root = p.dir.path();
    let club = root.join("lighting/venues/club.light");
    let before = std::fs::read_to_string(&club).unwrap();
    std::os::unix::fs::symlink(root.join("lighting/venues"), root.join("alias")).unwrap();
    for name in [
        "lighting/venues/club.light",
        "lighting/venues/new.light",
        "lighting/fixture_types/new.light",
        "lighting/x/../venues/new.light",
        "shows/../lighting/venues/club.light",
        "alias/club.light",
        "alias/new.light",
    ] {
        let status = p.save(name, SHOW).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "save {name}");
        assert!(
            status.message().contains("song light shows"),
            "save {name}: {}",
            status.message()
        );
    }
    for name in [
        "lighting/venues/club.light",
        "lighting/x/../venues/club.light",
        "alias/club.light",
    ] {
        let status = p.delete(name).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "delete {name}");
    }
    // Another case is another directory here (a case-sensitive file system),
    // so there is nothing to write into; on one that folds case it is the
    // venues directory, which identity finds.
    for name in ["Lighting/Venues/new.light", "lighting/VENUES/club.light"] {
        assert!(p.save(name, SHOW).await.is_err(), "save {name}");
    }
    assert_eq!(std::fs::read_to_string(&club).unwrap(), before);
    let venues: Vec<_> = std::fs::read_dir(root.join("lighting/venues"))
        .unwrap()
        .collect();
    assert_eq!(venues.len(), 1, "nothing new in the venues directory");
    assert!(!root.join("lighting/fixture_types/new.light").exists());

    // A song's light show elsewhere is still written and deleted.
    p.save("shows/a.light", SHOW).await.unwrap();
    p.delete("shows/a.light").await.unwrap();
}

#[tokio::test]
async fn a_light_show_is_never_the_hosts_configured_venues_directory() {
    let config = "songs: \".\"\nprofiles:\n  - hostname: not-this-host-x9\n    dmx:\n      universes: []\n      lighting:\n        directories:\n          venues: shows\n  - dmx:\n      universes: []\n      lighting:\n        directories:\n          venues: rig/venues\n";
    let p = light_shows_at_the_root(config);
    std::fs::create_dir_all(p.dir.path().join("rig/venues")).unwrap();
    let status = p.save("rig/venues/new.light", SHOW).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status.message().contains("venues directory (rig/venues)"),
        "{}",
        status.message()
    );
    // The other host's directory, and the default one this host does not
    // read, are not this host's venues.
    p.save("shows/a.light", SHOW).await.unwrap();
    p.save("lighting/venues/new.light", SHOW).await.unwrap();
}

/// A config that does not load is an internal error here as at every
/// endpoint that reads the lighting directories, not the defaults, which may
/// not be this host's.
#[tokio::test]
async fn a_config_that_does_not_load_refuses_a_light_show_as_internal() {
    let p = light_shows_at_the_root("profiles: [\n");
    let status = p.save("shows/a.light", SHOW).await.unwrap_err();
    assert_eq!(status.code(), Code::Internal);
    assert!(
        status.message().contains("the config does not load"),
        "{}",
        status.message()
    );
}

// ---- DeleteLightShow ----

/// A light show that is a link is deleted as the link: what it leads to,
/// another song's config or show, is left.
#[tokio::test]
async fn a_linked_light_show_is_deleted_as_the_link_not_its_target() {
    let p = project();
    let songs = p.songs();
    std::fs::create_dir_all(songs.join("a/lighting")).unwrap();
    std::fs::create_dir_all(songs.join("b/lighting")).unwrap();
    std::fs::write(songs.join("a/lighting/show.light"), "a's show").unwrap();
    std::fs::write(songs.join("b/song.yaml"), "name: B\n").unwrap();
    std::os::unix::fs::symlink("../song.yaml", songs.join("b/lighting/show.light")).unwrap();
    std::os::unix::fs::symlink(
        "../../a/lighting/show.light",
        songs.join("b/lighting/a.light"),
    )
    .unwrap();

    for file in ["show.light", "a.light"] {
        p.delete(&format!("b/lighting/{file}")).await.unwrap();
        assert!(
            std::fs::symlink_metadata(songs.join("b/lighting").join(file)).is_err(),
            "{file}: the link is gone"
        );
    }
    assert_eq!(
        std::fs::read_to_string(songs.join("b/song.yaml")).unwrap(),
        "name: B\n"
    );
    assert_eq!(
        std::fs::read_to_string(songs.join("a/lighting/show.light")).unwrap(),
        "a's show"
    );
}

/// The editor deletes a `Show.LIGHT` as it does a `show.light`; a file that
/// is no light show is still refused.
#[tokio::test]
async fn a_light_show_is_deleted_whatever_the_case_of_its_extension() {
    let p = project();
    let show = p.songs().join("Show.LIGHT");
    std::fs::write(&show, "content").unwrap();
    let notes = p.songs().join("notes.txt");
    std::fs::write(&notes, "content").unwrap();

    p.delete("Show.LIGHT").await.unwrap();
    let status = p.delete("notes.txt").await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(!show.exists());
    assert!(notes.exists());
}

#[tokio::test]
async fn a_light_show_that_is_not_there_cannot_be_deleted() {
    let p = project();
    let status = p.delete("missing.light").await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

// ---- The refusals as statuses ----

#[test]
fn a_refusal_keeps_its_message_and_a_test_its_reason() {
    use super::{answer_status, REFUSAL_REASON_KEY};
    use crate::webui::api::lighting_writes::Answer;
    use axum::http::StatusCode;

    for (http, code) in [
        (StatusCode::BAD_REQUEST, Code::InvalidArgument),
        (StatusCode::FORBIDDEN, Code::PermissionDenied),
        (StatusCode::NOT_FOUND, Code::NotFound),
        (StatusCode::CONFLICT, Code::FailedPrecondition),
        (StatusCode::LOCKED, Code::FailedPrecondition),
        (StatusCode::INTERNAL_SERVER_ERROR, Code::Internal),
    ] {
        let status = answer_status(Answer::new(http, "why"));
        assert_eq!(status.code(), code, "{http}");
        assert_eq!(status.message(), "why");
        assert!(status.metadata().get(REFUSAL_REASON_KEY).is_none());
    }
    let status = answer_status(Answer {
        status: StatusCode::BAD_REQUEST,
        body: serde_json::json!({"errors": ["one", "two"]}),
    });
    assert_eq!(status.message(), "one; two");
    let status = answer_status(Answer {
        status: StatusCode::CONFLICT,
        body: serde_json::json!({"error": "playing", "reason": "playing"}),
    });
    assert_eq!(
        status.metadata().get(REFUSAL_REASON_KEY).unwrap(),
        "playing"
    );
}

// ---- The lock ----

/// A locked player is refused the light show writes and the fixture test's
/// start before their handlers run, and served the reads, the validation,
/// the evaluation and the release of a test.
#[tokio::test]
async fn a_locked_player_serves_the_reads_and_not_the_writes() {
    use tower::ServiceExt;
    let p = project();
    p.state.player.set_locked(true);
    let code_of = |method: &'static str| {
        let player = p.state.player.clone();
        async move {
            let request = http::Request::post(format!("/player.v1.LightingService/{method}"))
                .header("content-type", "application/grpc")
                .body(tonic::body::Body::empty())
                .unwrap();
            let response = crate::controller::grpc::lighting_service(player)
                .oneshot(request)
                .await
                .unwrap();
            tonic::Status::from_header_map(response.headers())
                .map(|s| (s.code(), s.message().to_string()))
        }
    };
    let locked = crate::player::LockedError.to_string();
    for method in ["SaveLightShow", "DeleteLightShow", "StartFixtureTest"] {
        let (code, message) = code_of(method).await.expect(method);
        assert_eq!(code, Code::FailedPrecondition, "{method}");
        assert_eq!(message, locked, "{method}");
    }
    for method in [
        "GetLightShow",
        "ValidateLightShow",
        "GetLightingReadiness",
        "GetLightingFit",
        "EvaluateSongLighting",
        "ListLightingGroups",
        "StopFixtureTest",
        "GetFixtureTestOptions",
    ] {
        // Past the guard: whatever it answers, it is not the lock.
        if let Some((_, message)) = code_of(method).await {
            assert_ne!(message, locked, "{method}");
        }
    }
}

/// Over a real gRPC connection, behind the lock: the same service answers a
/// read while locked, refuses the write, and answers it once unlocked.
#[tokio::test(flavor = "multi_thread")]
async fn the_served_service_is_locked_and_unlocked_by_the_player() {
    use crate::proto::player::v1::lighting_service_client::LightingServiceClient;
    let p = project();
    std::fs::write(p.songs().join("show.light"), SHOW).unwrap();
    let driver = crate::controller::grpc::Driver::new(
        config::GrpcController::new(0),
        p.state.player.clone(),
    )
    .unwrap();
    drop(crate::controller::Driver::monitor_events(driver.as_ref()));
    let port = driver.bound.port().await;
    let mut client = LightingServiceClient::connect(format!("http://127.0.0.1:{port}"))
        .await
        .unwrap();

    p.state.player.set_locked(true);
    let read = client
        .get_light_show(proto::GetLightShowRequest {
            name: "show.light".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.source, SHOW);
    let refused = client
        .save_light_show(proto::SaveLightShowRequest {
            name: "other.light".to_string(),
            source: SHOW.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(!p.songs().join("other.light").exists());

    p.state.player.set_locked(false);
    client
        .save_light_show(proto::SaveLightShowRequest {
            name: "other.light".to_string(),
            source: SHOW.to_string(),
        })
        .await
        .unwrap();
    assert!(p.songs().join("other.light").exists());
}
