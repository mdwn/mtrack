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
use crate::hub::standin::{Corruption, StandIn};

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
    let Uploaded::Saved { revision, sent, .. } = uploaded else {
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
        matches!(uploaded, Uploaded::Saved { ref revision, sent: 0, .. } if revision.number == 2),
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
    for how in [Corruption::OtherBytes, Corruption::Short, Corruption::Long] {
        let (hub, first_dir, first, second_dir, second) = two_rigs().await;
        write(first_dir.path(), "songs/one/notes.txt", "a new file");
        write(
            first_dir.path(),
            "playlists/set.yaml",
            "kind: playlist\nsongs: []\n",
        );
        first.upload().await.unwrap();
        let before = files(&second_dir);
        let bad = hash_file(&first_dir.path().join("songs/one/notes.txt")).unwrap();
        hub.corrupt(&bad.sha256, how);

        let failed = second.pull(false).await.unwrap_err();
        assert!(
            matches!(&failed, SyncError::Hub(HubError::Corrupt(path)) if path == "songs/one/notes.txt"),
            "{how:?}: {failed}"
        );
        assert_eq!(files(&second_dir), before, "{how:?}");
        assert!(!second.state.journal().exists());
        assert!(
            !file_at(&second.state.staging().join("files"), "songs/one/notes.txt").exists(),
            "{how:?}: what arrived is not kept"
        );
        assert_eq!(second.status().unwrap().revision.unwrap().number, 1);
    }
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
    hub.corrupt(&bad, Corruption::OtherBytes);
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
async fn a_refusal_of_the_rigs_token_is_said_as_one() {
    let hub = StandIn::start("The show").await;
    let dir = a_project();
    let rig = paired(&hub, &dir, "first").await;
    let mut credentials = rig.credentials.clone();
    credentials.token = "not-a-token".to_string();
    rig.state.set_credentials(&credentials).unwrap();
    let rig = Rig::open(&config(&dir)).unwrap();
    let refused = rig.upload().await.unwrap_err();
    assert!(
        matches!(
            refused,
            SyncError::Hub(HubError::Refused { status: 401, .. })
        ),
        "{refused}"
    );
    assert!(!format!("{:?}", rig.credentials).contains("not-a-token"));
}

#[tokio::test]
async fn the_token_goes_to_the_hub_and_not_to_where_files_are_sent() {
    let mut hub = StandIn::start("The show").await;
    hub.store_elsewhere().await;
    let dir = a_project();
    let rig = paired(&hub, &dir, "first").await;
    assert!(matches!(
        rig.upload().await.unwrap(),
        Uploaded::Saved { sent: 4, .. }
    ));
    assert_eq!(hub.uploads().len(), 4, "the files arrived");
    assert!(!hub.storage_saw_token());
}

#[tokio::test]
async fn a_hub_that_is_not_https_is_not_given_a_token() {
    for hub in [
        "http://hub.example",
        "http://127.0.0.1.evil.example",
        "http://127.0.0.1:80@evil.example",
        "http://localhost:@evil.example",
        "https://user:secret@hub.example",
        "ftp://hub.example",
    ] {
        assert!(
            matches!(Client::new(hub, None), Err(HubError::NotHttps(_))),
            "{hub}"
        );
    }
    assert!(Client::new("https://hub.example", None).is_ok());
    assert!(Client::new("HTTPS://hub.example/", None).is_ok());
    assert!(Client::new("http://127.0.0.1:8080", None).is_ok());
}

#[tokio::test]
async fn files_given_up_to_a_pull_and_put_back_by_an_undo_are_still_changed_here() {
    let (_hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    first.upload().await.unwrap();

    write(
        second_dir.path(),
        "songs/one/song.yaml",
        "kind: song\nname: Mine\n",
    );
    let mine = files(&second_dir);
    second.pull(true).await.unwrap();
    second.undo(false).await.unwrap();
    assert_eq!(files(&second_dir), mine);

    let status = second.status().unwrap();
    assert!(status.local_changes, "they were never uploaded");
    assert_eq!(status.revision.unwrap().number, 1);
    // So the next pull asks again before it replaces them.
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::LocalChanges)
    ));
    assert_eq!(files(&second_dir), mine);
}

#[tokio::test]
async fn what_a_failed_pull_fetched_does_not_poison_the_next() {
    let (hub, _first_dir, _first, second_dir, second) = two_rigs().await;
    let good = files(&second_dir);

    // A revision whose config does not load: fetched, refused, left staged.
    let mut broken = good.clone();
    broken
        .files
        .insert("mtrack.yaml".to_string(), entry(b"songs: [not, a, path]\n"));
    hub.force_revision(broken, &[b"songs: [not, a, path]\n"]);
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::RevisionDoesNotLoad(_))
    ));

    // The next revision puts the config back and adds a file. The staged
    // broken config is not this pull's, and must not be what is checked.
    let mut fixed = good.clone();
    fixed
        .files
        .insert("songs/one/notes.txt".to_string(), entry(b"fixed"));
    hub.force_revision(fixed.clone(), &[b"fixed"]);
    let pulled = second.pull(false).await.unwrap();
    assert!(
        matches!(pulled, Pulled::Updated { fetched: 1, .. }),
        "{pulled:?}"
    );
    assert_eq!(files(&second_dir), fixed);

    // A staged file where a later revision has a folder is no obstacle.
    let mut as_file = fixed.clone();
    as_file.files.insert("extra".to_string(), entry(b"a file"));
    as_file.files.insert(
        "mtrack.yaml".to_string(),
        entry(b"songs: [still, broken]\n"),
    );
    hub.force_revision(as_file, &[b"a file", b"songs: [still, broken]\n"]);
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::RevisionDoesNotLoad(_))
    ));
    assert!(file_at(&second.state.staging().join("files"), "extra").is_file());
    let mut as_folder = fixed.clone();
    as_folder
        .files
        .insert("extra/inside.txt".to_string(), entry(b"in a folder"));
    hub.force_revision(as_folder.clone(), &[b"in a folder"]);
    second.pull(false).await.unwrap();
    assert_eq!(files(&second_dir), as_folder);
}

#[tokio::test]
async fn a_file_written_while_a_pull_is_fetching_stops_it() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    first.upload().await.unwrap();
    // The player, say, saving a song while the pull fetches.
    hub.write_on_download(
        second_dir.path().join("playlists/set.yaml"),
        "kind: playlist\nsongs: []\n",
    );
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::ChangedMeanwhile)
    ));
    assert!(!second_dir.path().join("songs/one/notes.txt").exists());
    assert_eq!(
        std::fs::read_to_string(second_dir.path().join("playlists/set.yaml")).unwrap(),
        "kind: playlist\nsongs: []\n",
        "what was written is still there"
    );
    assert!(!second.state.journal().exists());
    // And is now a change here, which the next pull asks about.
    assert!(matches!(
        second.pull(false).await,
        Err(SyncError::LocalChanges)
    ));
}

#[tokio::test]
async fn a_pull_that_happened_is_not_a_failure_because_the_hub_was_not_told() {
    let (hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    hub.fail_reports();
    assert!(matches!(
        first.upload().await.unwrap(),
        Uploaded::Saved { .. }
    ));
    assert!(matches!(
        second.pull(false).await.unwrap(),
        Pulled::Updated { .. }
    ));
    assert_eq!(files(&second_dir), files(&first_dir));
}

#[tokio::test]
async fn a_project_paired_anew_with_another_starts_afresh() {
    let (hub, _first_dir, _first, second_dir, second) = two_rigs().await;
    assert_eq!(second.status().unwrap().revision.unwrap().number, 1);
    let other = hub.add_project("Another show");
    pair(&config(&second_dir), &hub.url, "second", |_| {
        hub.approve_all_for(&other)
    })
    .await
    .unwrap();
    let again = Rig::open(&config(&second_dir)).unwrap();
    assert_eq!(again.credentials.project.name, "Another show");

    let status = again.status().unwrap();
    assert_eq!(status.revision, None, "it is no revision of this project");
    assert!(status.local_changes, "its files were never this project's");
    assert!(!status.undoable);
    // And it uploads as that project's first revision.
    let uploaded = again.upload().await.unwrap();
    assert!(
        matches!(uploaded, Uploaded::Saved { ref revision, .. } if revision.number == 1),
        "{uploaded:?}"
    );

    // Paired again with the same project, what it was recorded as stays.
    pair(&config(&second_dir), &hub.url, "second", |_| {
        hub.approve_all_for(&other)
    })
    .await
    .unwrap();
    let same = Rig::open(&config(&second_dir)).unwrap();
    assert_eq!(same.status().unwrap().revision.unwrap().number, 1);
}

#[tokio::test]
async fn a_folder_that_is_not_a_project_is_not_replaced_even_when_told_to() {
    let (hub, _first_dir, _first, _second_dir, _second) = two_rigs().await;
    let home = tempfile::tempdir().unwrap();
    write(home.path(), "notes/todo.txt", "mine");
    let rig = paired(&hub, &home, "third").await;
    assert!(matches!(
        rig.pull(false).await,
        Err(SyncError::LocalChanges)
    ));
    assert!(matches!(
        rig.pull(true).await,
        Err(SyncError::NotAProject(_))
    ));
    assert_eq!(
        std::fs::read_to_string(home.path().join("notes/todo.txt")).unwrap(),
        "mine"
    );
}

#[tokio::test]
async fn a_project_another_rig_could_not_take_is_not_uploaded() {
    let hub = StandIn::start("The show").await;

    // A song whose track is outside the project's folder.
    let elsewhere = tempfile::tempdir().unwrap();
    crate::testutil::write_wav(
        elsewhere.path().join("click.wav"),
        vec![vec![1_i32; 4410]],
        44100,
    )
    .unwrap();
    let dir = a_project();
    write(
        dir.path(),
        "songs/two/song.yaml",
        &format!(
            "kind: song\nname: Two\ntracks:\n  - name: click\n    file: {}\n",
            elsewhere.path().join("click.wav").display()
        ),
    );
    let rig = paired(&hub, &dir, "first").await;
    let refused = rig.upload().await.unwrap_err();
    assert!(
        matches!(&refused, SyncError::NotUploadable(why) if why.contains("click.wav")),
        "{refused}"
    );

    // Two names that differ only by case, where the disk tells them apart.
    let dir = a_project();
    write(dir.path(), "songs/one/Notes.txt", "a");
    write(dir.path(), "songs/one/notes.txt", "b");
    if files(&dir).files.len() == 6 {
        let rig = paired(&hub, &dir, "other").await;
        let refused = rig.upload().await.unwrap_err();
        assert!(
            matches!(&refused, SyncError::NotUploadable(why) if why.contains("differ only by case")),
            "{refused}"
        );
    }
    assert!(hub.uploads().is_empty());
    assert_eq!(hub.revisions(), 0);
}

#[tokio::test]
async fn a_pairing_refused_in_the_hub_is_said_plainly() {
    let hub = StandIn::start("The show").await;
    let dir = a_project();
    let refused = pair(&config(&dir), &hub.url, "first", |_| hub.deny_all())
        .await
        .unwrap_err();
    assert!(
        matches!(refused, SyncError::NotPairedInTime(_)),
        "{refused}"
    );
    assert!(
        refused.to_string().contains("refused in the hub"),
        "{refused}"
    );
    assert!(matches!(
        Rig::open(&config(&dir)),
        Err(SyncError::NotPaired)
    ));
}

#[tokio::test]
async fn a_pull_cut_short_is_finished_before_the_player_reads_the_project() {
    // Not paired, never pulled: nothing is done, and nothing is made.
    let plain = a_project();
    assert!(!recover(&config(&plain)).unwrap());
    assert!(!plain.path().join(crate::hub::state::DIR).exists());

    let (_hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    first.upload().await.unwrap();
    second.pull(false).await.unwrap();
    // The pull undone, and then an undo of that set down and cut short
    // before any of its moves: the project is at revision 1 with a journal
    // that takes it to revision 2.
    second.undo(false).await.unwrap();
    let previous = apply::undoable(&second.state).unwrap().unwrap();
    let journal = Journal {
        source: Source::Previous,
        manifest: files(&first_dir),
        state: previous.state,
        before: files(&second_dir),
        state_before: second.state.sync_state().unwrap(),
        moved: false,
    };
    apply::begin(&second.state, &journal).unwrap();
    assert!(!second_dir.path().join("songs/one/notes.txt").exists());

    assert!(recover(&config(&second_dir)).unwrap());
    assert_eq!(files(&second_dir), files(&first_dir));
    let status = second.status().unwrap();
    assert_eq!(status.revision.unwrap().number, 2);
    assert!(!status.local_changes);
    assert!(status.undoable);
    assert!(!recover(&config(&second_dir)).unwrap());
}

#[tokio::test]
async fn an_upload_leaves_no_older_pull_to_undo() {
    let (_hub, first_dir, first, second_dir, second) = two_rigs().await;
    write(first_dir.path(), "songs/one/notes.txt", "from the first");
    first.upload().await.unwrap();
    second.pull(false).await.unwrap();
    assert!(second.status().unwrap().undoable);

    write(second_dir.path(), "songs/one/notes.txt", "from the second");
    second.upload().await.unwrap();
    let status = second.status().unwrap();
    assert!(!status.undoable, "the project has moved on from that pull");
    assert_eq!(status.revision.unwrap().number, 3);
    assert!(second.undo(false).await.is_err());
}

#[tokio::test]
async fn a_fetched_song_that_does_not_load_is_named_where_it_now_is() {
    let (hub, _first_dir, _first, second_dir, second) = two_rigs().await;
    let mut manifest = files(&second_dir);
    let broken = b"kind: song\nname: Broken\ntracks:\n  - name: gone\n    file: gone.wav\n";
    manifest
        .files
        .insert("songs/broken/song.yaml".to_string(), entry(broken));
    hub.force_revision(manifest, &[broken]);
    let pulled = second.pull(false).await.unwrap();
    let Pulled::Updated { report, .. } = pulled else {
        panic!("{pulled:?}");
    };
    assert_eq!(report.problems.len(), 1, "{report:?}");
    let named = &report.problems[0].0;
    assert!(named.is_file(), "{}", named.display());
    assert!(
        named.ends_with("songs/broken/song.yaml")
            && !named
                .components()
                .any(|part| part.as_os_str() == crate::hub::state::DIR),
        "{}",
        named.display()
    );
}
