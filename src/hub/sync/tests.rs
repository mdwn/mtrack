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
//! A rig against the stand-in hub ([`crate::hub::standin`]), end to end.

use std::path::{Path, PathBuf};

use mtrack_core::project::{hash_file, FileEntry, Manifest};

use super::*;
use crate::hub::standin::StandIn;

fn write(dir: &Path, path: &str, contents: &str) {
    let file = file_at(dir, path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, contents).unwrap();
}

/// A project that loads: one song with one track, in one playlist.
fn a_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "mtrack.yaml",
        "songs: songs\nplaylists_dir: playlists\n",
    );
    write(
        dir.path(),
        "songs/one/song.yaml",
        "kind: song\nname: One\ntracks:\n  - name: click\n    file: click.wav\n",
    );
    crate::testutil::write_wav(
        dir.path().join("songs/one/click.wav"),
        vec![vec![1_i32; 4410]],
        44100,
    )
    .unwrap();
    write(
        dir.path(),
        "playlists/set.yaml",
        "kind: playlist\nsongs:\n  - One\n",
    );
    dir
}

fn config(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("mtrack.yaml")
}

fn files(dir: &tempfile::TempDir) -> Manifest {
    Manifest::of_dir(dir.path()).unwrap()
}

/// Pairs the project in `dir` as a rig named `name`, a person entering the
/// code as soon as it is shown.
async fn paired(hub: &StandIn, dir: &tempfile::TempDir, name: &str) -> Rig {
    pair(&config(dir), &hub.url, name, |started| {
        assert!(hub.codes().contains(&started.user_code));
        hub.approve_all();
    })
    .await
    .unwrap();
    Rig::open(&config(dir)).unwrap()
}

/// A hub, a rig that has uploaded [`a_project`] to it, and a second rig
/// that has pulled it into an empty directory.
async fn two_rigs() -> (StandIn, tempfile::TempDir, Rig, tempfile::TempDir, Rig) {
    let hub = StandIn::start("The show").await;
    let first_dir = a_project();
    let first = paired(&hub, &first_dir, "first").await;
    first.upload().await.unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let second = paired(&hub, &second_dir, "second").await;
    second.pull(false).await.unwrap();
    (hub, first_dir, first, second_dir, second)
}

#[tokio::test]
async fn a_project_goes_from_one_rig_to_another() {
    let hub = StandIn::start("The show").await;
    let first_dir = a_project();
    let first = paired(&hub, &first_dir, "first").await;
    assert_eq!(first.credentials.project.name, "The show");
    assert!(first.status().unwrap().local_changes, "never uploaded");

    let uploaded = first.upload().await.unwrap();
    let Uploaded::Saved { revision, sent } = uploaded else {
        panic!("{uploaded:?}");
    };
    assert_eq!(revision.number, 1);
    assert_eq!(sent, 4, "each file once");
    assert_eq!(hub.uploads().len(), 4);
    assert!(!first.status().unwrap().local_changes);
    assert_eq!(hub.reported("first"), Some(revision.id.clone()));

    let second_dir = tempfile::tempdir().unwrap();
    let second = paired(&hub, &second_dir, "second").await;
    let pulled = second.pull(false).await.unwrap();
    let Pulled::Updated {
        revision: at,
        fetched,
        report,
    } = pulled
    else {
        panic!("{pulled:?}");
    };
    assert_eq!(at, revision);
    assert_eq!(fetched, 4);
    assert_eq!((report.songs, report.playlists), (1, 1), "{report:?}");
    assert!(report.problems.is_empty(), "{report:?}");
    assert_eq!(files(&second_dir), files(&first_dir));
    assert_eq!(hub.reported("second"), Some(revision.id.clone()));
    // The second rig's copy plays: it loads as the player loads it.
    let loaded = mtrack_core::project::check(&config(&second_dir)).unwrap();
    assert_eq!(loaded.songs, 1);
    // And nothing is left over from the pull.
    let status = second.status().unwrap();
    assert!(!status.local_changes);
    assert_eq!(status.revision, Some(revision));
}

#[tokio::test]
async fn only_what_changed_is_sent_and_fetched() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    assert_eq!(first.upload().await.unwrap(), {
        let revision = first.status().unwrap().revision.unwrap();
        Uploaded::UpToDate(revision)
    });
    assert!(matches!(
        second.pull(false).await.unwrap(),
        Pulled::UpToDate(_)
    ));
    let (sent_before, fetched_before) = (hub.uploads().len(), hub.downloads().len());

    write(
        first_dir.path(),
        "playlists/encore.yaml",
        "kind: playlist\nsongs:\n  - One\n",
    );
    let uploaded = first.upload().await.unwrap();
    // The new playlist has the bytes of the one the hub holds: nothing is
    // sent for it, and it is still a new revision.
    assert!(
        matches!(uploaded, Uploaded::Saved { ref revision, sent: 0 } if revision.number == 2),
        "{uploaded:?}"
    );
    write(
        first_dir.path(),
        "playlists/set.yaml",
        "kind: playlist\nsongs: []\n",
    );
    assert!(matches!(
        first.upload().await.unwrap(),
        Uploaded::Saved { sent: 1, .. }
    ));
    assert_eq!(hub.uploads().len(), sent_before + 1);

    let pulled = second.pull(false).await.unwrap();
    assert!(
        matches!(pulled, Pulled::Updated { ref revision, fetched: 2, .. } if revision.number == 3),
        "{pulled:?}"
    );
    assert_eq!(hub.downloads().len(), fetched_before + 2);
    assert_eq!(files(&second_dir), files(&first_dir));
}

#[tokio::test]
async fn a_pull_is_undone_with_no_hub() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    let before = files(&second_dir);
    std::fs::remove_file(first_dir.path().join("playlists/set.yaml")).unwrap();
    write(first_dir.path(), "songs/one/notes.txt", "louder");
    first.upload().await.unwrap();
    second.pull(false).await.unwrap();
    assert_eq!(files(&second_dir), files(&first_dir));
    assert!(!second_dir.path().join("playlists").exists());
    assert!(second.status().unwrap().undoable);

    drop(hub);
    let back = second.undo(false).await.unwrap();
    assert_eq!(back.map(|revision| revision.number), Some(1));
    assert_eq!(files(&second_dir), before);
    assert!(!second.status().unwrap().local_changes);
}

#[tokio::test]
async fn files_changed_here_stop_a_pull_until_they_are_given_up() {
    let (_hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    first.upload().await.unwrap();

    write(
        second_dir.path(),
        "songs/one/song.yaml",
        "kind: song\nname: Mine\n",
    );
    let before = files(&second_dir);
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::LocalChanges)
    ));
    assert_eq!(files(&second_dir), before, "nothing was touched");

    second.pull(true).await.unwrap();
    assert_eq!(files(&second_dir), files(&first_dir));
    // What was given up is kept for one undo.
    let kept = second.state.previous().join("files");
    assert_eq!(
        std::fs::read_to_string(file_at(&kept, "songs/one/song.yaml")).unwrap(),
        "kind: song\nname: Mine\n"
    );
}

#[tokio::test]
async fn a_file_that_does_not_arrive_whole_leaves_the_project_as_it_was() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "a new file");
    write(
        first_dir.path(),
        "playlists/set.yaml",
        "kind: playlist\nsongs: []\n",
    );
    first.upload().await.unwrap();
    let before = files(&second_dir);
    hub.corrupt(
        &hash_file(&first_dir.path().join("songs/one/notes.txt"))
            .unwrap()
            .sha256,
    );

    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::Hub(HubError::Corrupt(_)))
    ));
    assert_eq!(files(&second_dir), before);
    assert!(!second.state.journal().exists());
    assert!(!file_at(&second.state.staging().join("files"), "songs/one/notes.txt").exists());
    assert_eq!(second.status().unwrap().revision.unwrap().number, 1);
}

#[tokio::test]
async fn a_pull_picks_up_where_one_left_off() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "a new file");
    write(
        first_dir.path(),
        "playlists/set.yaml",
        "kind: playlist\nsongs: []\n",
    );
    first.upload().await.unwrap();
    let bad = hash_file(&first_dir.path().join("songs/one/notes.txt"))
        .unwrap()
        .sha256;
    hub.corrupt(&bad);
    assert!(second.pull(false).await.is_err());
    // The playlist arrived before the bad file stopped the pull.
    let fetched_before = hub.downloads().len();
    assert!(file_at(&second.state.staging().join("files"), "playlists/set.yaml").is_file());

    // The hub serving it right now, only the file that failed is fetched.
    drop(first);
    hub.uncorrupt(&bad);
    let pulled = second.pull(false).await.unwrap();
    assert!(
        matches!(pulled, Pulled::Updated { fetched: 1, .. }),
        "{pulled:?}"
    );
    assert_eq!(hub.downloads().len(), fetched_before + 1);
    assert_eq!(files(&second_dir), files(&first_dir));
}

fn entry(bytes: &[u8]) -> FileEntry {
    use sha2::{Digest, Sha256};
    FileEntry {
        sha256: mtrack_core::project::hex(&Sha256::digest(bytes)),
        size: bytes.len() as u64,
    }
}

#[tokio::test]
async fn a_revision_that_names_a_path_outside_the_project_is_refused_whole() {
    let (hub, _first_dir, _first, second_dir, second) = two_rigs().await;
    let before = files(&second_dir);
    for path in [
        "../escaped.txt",
        "/tmp/escaped.txt",
        ".mtrack-hub/rig.json",
        "songs/../../x",
    ] {
        let mut manifest = before.clone();
        manifest.files.insert(path.to_string(), entry(b"gotcha"));
        hub.force_revision(manifest, &[b"gotcha"]);
        assert!(
            matches!(second.pull(false).await, Err(SyncError::BadRevision(_))),
            "{path}"
        );
        assert_eq!(files(&second_dir), before, "{path}");
    }
    assert!(!second_dir
        .path()
        .parent()
        .unwrap()
        .join("escaped.txt")
        .exists());
    assert_eq!(hub.downloads().len(), 4, "nothing was even fetched");
}

#[tokio::test]
async fn a_revision_that_does_not_load_is_not_put_in_place() {
    let (hub, _first_dir, _first, second_dir, second) = two_rigs().await;
    let before = files(&second_dir);

    let mut broken = before.clone();
    broken
        .files
        .insert("mtrack.yaml".to_string(), entry(b"songs: [not, a, path]\n"));
    hub.force_revision(broken, &[b"songs: [not, a, path]\n"]);
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::RevisionDoesNotLoad(_))
    ));
    assert_eq!(files(&second_dir), before);
    assert!(!second.state.journal().exists());

    let mut no_config = before.clone();
    no_config.files.remove("mtrack.yaml");
    hub.force_revision(no_config, &[]);
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::BadRevision(_))
    ));
    assert_eq!(files(&second_dir), before);
}

#[tokio::test]
async fn one_rigs_upload_cannot_silently_undo_anothers() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    first.upload().await.unwrap();

    write(second_dir.path(), "songs/one/notes.txt", "from the second");
    assert!(matches!(
        second.upload().await,
        Err(SyncError::Hub(HubError::NotCurrent))
    ));
    assert_eq!(hub.revisions(), 2);
    assert!(second.status().unwrap().local_changes);
}

#[tokio::test]
async fn what_the_hub_will_not_take_is_said_and_nothing_is_recorded() {
    let hub = StandIn::start("The show").await;
    let dir = a_project();
    let rig = paired(&hub, &dir, "first").await;
    hub.refuse_next("songs/one/song.yaml: no such track file");
    let refused = rig.upload().await.unwrap_err();
    assert!(
        matches!(&refused, SyncError::Hub(HubError::InvalidProject(why)) if why.contains("no such track")),
        "{refused}"
    );
    assert_eq!(hub.revisions(), 0);
    assert_eq!(rig.status().unwrap().revision, None);
}

#[tokio::test]
async fn a_project_that_does_not_load_here_is_not_sent() {
    let hub = StandIn::start("The show").await;
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mtrack.yaml", "songs: [not, a, path]\n");
    let rig = paired(&hub, &dir, "first").await;
    assert!(matches!(rig.upload().await, Err(SyncError::DoesNotLoad(_))));
    assert!(hub.uploads().is_empty());
}

#[tokio::test]
async fn a_project_that_was_never_paired_says_so() {
    let dir = a_project();
    assert!(matches!(
        Rig::open(&config(&dir)),
        Err(SyncError::NotPaired)
    ));
}

#[tokio::test]
async fn there_is_nothing_to_pull_from_an_empty_project() {
    let hub = StandIn::start("The show").await;
    let dir = tempfile::tempdir().unwrap();
    let rig = paired(&hub, &dir, "first").await;
    assert!(matches!(
        rig.pull(false).await,
        Err(SyncError::NothingToPull)
    ));
}

#[tokio::test]
async fn a_token_the_hub_does_not_know_is_refused() {
    let hub = StandIn::start("The show").await;
    let dir = a_project();
    let rig = paired(&hub, &dir, "first").await;
    let mut credentials = rig.credentials.clone();
    credentials.token = "not-a-token".to_string();
    rig.state.set_credentials(&credentials).unwrap();
    let rig = Rig::open(&config(&dir)).unwrap();
    assert!(matches!(
        rig.upload().await,
        Err(SyncError::Hub(HubError::Refused { status: 401, .. }))
    ));
}
