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

use std::time::Duration;

use super::*;

const MINUTE: Duration = Duration::from_secs(60);

fn track() -> Purpose {
    Purpose::Track {
        song: "Song".to_string(),
    }
}

/// An upload announced and sent whole; its id.
fn sent(t: &Transfers, purpose: Purpose, bytes: &[u8]) -> String {
    let id = t
        .begin_upload(purpose, "f.wav".to_string(), bytes.len() as u64)
        .unwrap();
    let receive = t.receive(&id).unwrap();
    std::fs::write(receive.path(), bytes).unwrap();
    receive.finish(bytes.len() as u64);
    id
}

fn files(t: &Transfers) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(t.staging_dir().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn is_track(p: &Purpose) -> bool {
    matches!(p, Purpose::Track { .. })
}

#[test]
fn an_id_is_128_random_bits_in_hex() {
    let t = Transfers::new();
    let mut ids = std::collections::HashSet::new();
    for _ in 0..MAX_PENDING {
        let id = t.begin_upload(track(), "a.wav".into(), 1).unwrap();
        assert_eq!(id.len(), 32, "{id}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "{id}"
        );
        assert!(ids.insert(id));
    }
}

#[test]
fn the_staging_directory_is_private_and_files_are_named_by_the_id_alone() {
    use std::os::unix::fs::PermissionsExt;
    let t = Transfers::new();
    let mode = std::fs::metadata(t.staging_dir().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);

    // Nothing a caller sends is in a path: not the file's name, not the song.
    let hostile = "../../etc/passwd\0 ../x";
    let id = t
        .begin_upload(
            Purpose::Track {
                song: hostile.to_string(),
            },
            hostile.to_string(),
            3,
        )
        .unwrap();
    let receive = t.receive(&id).unwrap();
    assert_eq!(receive.path(), t.staging_dir().unwrap().join(&id));
    std::fs::write(receive.path(), b"abc").unwrap();
    receive.finish(3);
    assert_eq!(files(&t), vec![id.clone()]);
    let claimed = t.claim(std::slice::from_ref(&id), is_track).unwrap();
    assert_eq!(claimed[0].file_name, hostile);
    assert_eq!(
        claimed[0].path().unwrap(),
        t.staging_dir().unwrap().join(&id)
    );
}

#[test]
fn the_staging_directory_goes_with_the_registry() {
    let t = Transfers::new();
    let dir = t.staging_dir().unwrap().to_path_buf();
    assert!(dir.is_dir());
    drop(t);
    assert!(!dir.exists());
}

#[test]
fn an_upload_is_sent_once_and_claimed_once() {
    let t = Transfers::new();
    let id = sent(&t, track(), b"hello");

    // A second send, whole or begun, is refused.
    assert_eq!(t.receive(&id).err(), Some(TransferError::AlreadySent));
    let claimed = t.claim(std::slice::from_ref(&id), is_track).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].read().unwrap(), b"hello");
    assert_eq!(claimed[0].size(), 5);
    drop(claimed);
    // The staged file went with the claim; the id is spent.
    assert!(files(&t).is_empty());
    assert_eq!(
        t.claim(std::slice::from_ref(&id), is_track).err(),
        Some(TransferError::NotFound)
    );
    assert_eq!(t.receive(&id).err(), Some(TransferError::NotFound));
    assert_eq!(t.pending(), 0);
}

#[test]
fn an_upload_being_received_cannot_be_received_again() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 4).unwrap();
    let receive = t.receive(&id).unwrap();
    assert_eq!(t.receive(&id).err(), Some(TransferError::AlreadySent));
    // And it cannot be claimed before it is whole.
    assert!(matches!(
        t.claim(std::slice::from_ref(&id), is_track),
        Err(TransferError::Unusable(_))
    ));
    receive.finish(0);
}

#[test]
fn an_id_that_was_never_issued_is_not_found() {
    let t = Transfers::new();
    assert_eq!(t.receive("nope").err(), Some(TransferError::NotFound));
    assert_eq!(
        t.claim(&["nope".to_string()], is_track).err(),
        Some(TransferError::NotFound)
    );
    assert_eq!(t.download("nope").err(), Some(TransferError::NotFound));
    // Neither is a path the id could be made to spell.
    for hostile in ["../x", "", "/etc/passwd", "a/b", "."] {
        assert_eq!(t.receive(hostile).err(), Some(TransferError::NotFound));
    }
}

#[test]
fn a_claim_needs_the_purpose_and_takes_nothing_when_it_is_wrong() {
    let t = Transfers::new();
    let id = sent(&t, Purpose::Sample, b"x");
    assert!(matches!(
        t.claim(std::slice::from_ref(&id), is_track),
        Err(TransferError::Unusable(_))
    ));
    // Not spent: the right call can still use it.
    assert_eq!(t.pending(), 1);
    assert_eq!(files(&t), vec![id.clone()]);
    assert!(t.claim(&[id], |p| *p == Purpose::Sample).is_ok());
}

#[test]
fn an_upload_not_sent_yet_is_not_claimable_and_stays() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 1).unwrap();
    assert!(matches!(
        t.claim(std::slice::from_ref(&id), is_track),
        Err(TransferError::Unusable(_))
    ));
    assert_eq!(t.pending(), 1);
    assert!(t.receive(&id).is_ok());
}

#[test]
fn a_body_shorter_than_announced_is_spent_when_claimed() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 10).unwrap();
    let receive = t.receive(&id).unwrap();
    std::fs::write(receive.path(), b"short").unwrap();
    receive.finish(5);
    let refused = t.claim(std::slice::from_ref(&id), is_track).unwrap_err();
    assert!(
        matches!(&refused, TransferError::Unusable(why) if why.contains("10 bytes") && why.contains("5")),
        "{refused:?}"
    );
    // The file is deleted whether the claim succeeds or not.
    assert!(files(&t).is_empty());
    assert_eq!(t.pending(), 0);
}

#[test]
fn a_claim_of_several_takes_all_or_none_and_each_once() {
    let t = Transfers::new();
    let a = sent(&t, track(), b"a");
    let b = sent(&t, track(), b"b");
    let other = sent(&t, Purpose::Sample, b"s");

    // One that is not for the call: nothing is taken.
    assert!(t
        .claim(&[a.clone(), other.clone(), b.clone()], is_track)
        .is_err());
    assert_eq!(t.pending(), 3);
    // One twice in a list is refused too.
    assert!(matches!(
        t.claim(&[a.clone(), a.clone()], is_track),
        Err(TransferError::Unusable(_))
    ));
    assert_eq!(t.pending(), 3);

    let claimed = t.claim(&[a, b], is_track).unwrap();
    assert_eq!(claimed.len(), 2);
    assert_eq!(claimed[0].read().unwrap(), b"a");
    assert_eq!(claimed[1].read().unwrap(), b"b");
    drop(claimed);
    assert_eq!(t.pending(), 1);
    assert_eq!(files(&t), vec![other]);
}

#[test]
fn a_receive_dropped_before_it_finishes_discards_the_upload() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 4).unwrap();
    let receive = t.receive(&id).unwrap();
    std::fs::write(receive.path(), b"ab").unwrap();
    drop(receive);
    assert_eq!(t.pending(), 0);
    assert!(files(&t).is_empty());
    assert_eq!(t.receive(&id).err(), Some(TransferError::NotFound));
}

#[test]
fn at_most_32_are_pending_uploads_and_downloads_together() {
    let t = Transfers::new();
    let mut ids = Vec::new();
    for _ in 0..(MAX_PENDING - 1) {
        ids.push(t.begin_upload(track(), "a.wav".into(), 1).unwrap());
    }
    // A download counts.
    t.stage_download("a.mvr".to_string(), b"x").unwrap();
    assert_eq!(
        t.begin_upload(track(), "a.wav".into(), 1).err(),
        Some(TransferError::Full)
    );
    assert_eq!(
        t.stage_download("b.mvr".to_string(), b"x").err(),
        Some(TransferError::Full)
    );
    // Room again when one is claimed.
    let id = sent_in_place(&t, &ids[0]);
    drop(t.claim(&[id], is_track).unwrap());
    assert!(t.begin_upload(track(), "a.wav".into(), 1).is_ok());
    assert_eq!(
        t.begin_upload(track(), "a.wav".into(), 1).err(),
        Some(TransferError::Full)
    );
}

/// Sends the byte the announced upload `id` said it would.
fn sent_in_place(t: &Transfers, id: &str) -> String {
    let receive = t.receive(id).unwrap();
    std::fs::write(receive.path(), b"x").unwrap();
    receive.finish(1);
    id.to_string()
}

#[tokio::test(start_paused = true)]
async fn an_upload_not_sent_is_dropped_ten_minutes_after_it_began() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 1).unwrap();
    tokio::time::advance(10 * MINUTE - Duration::from_secs(1)).await;
    assert!(t.receive(&id).is_ok(), "still good a second before");

    let id = t.begin_upload(track(), "a.wav".into(), 1).unwrap();
    tokio::time::advance(10 * MINUTE).await;
    // The claim, the send and the sweep all find it gone.
    assert_eq!(
        t.claim(std::slice::from_ref(&id), is_track).err(),
        Some(TransferError::NotFound)
    );
    assert_eq!(t.receive(&id).err(), Some(TransferError::NotFound));
}

#[tokio::test(start_paused = true)]
async fn a_received_upload_is_dropped_ten_minutes_after_the_last_byte() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 3).unwrap();
    // Begun nine minutes before the bytes arrive: the clock restarts.
    tokio::time::advance(9 * MINUTE).await;
    let receive = t.receive(&id).unwrap();
    std::fs::write(receive.path(), b"abc").unwrap();
    receive.finish(3);

    tokio::time::advance(9 * MINUTE).await;
    assert_eq!(files(&t), vec![id.clone()], "eighteen minutes after Begin");
    tokio::time::advance(MINUTE).await;
    assert_eq!(
        t.claim(std::slice::from_ref(&id), is_track).err(),
        Some(TransferError::NotFound)
    );
    assert!(files(&t).is_empty(), "the file went with it");
}

#[tokio::test(start_paused = true)]
async fn an_upload_being_received_is_not_dropped_mid_transfer() {
    let t = Transfers::new();
    let id = t.begin_upload(track(), "a.wav".into(), 3).unwrap();
    let receive = t.receive(&id).unwrap();
    std::fs::write(receive.path(), b"ab").unwrap();
    tokio::time::advance(60 * MINUTE).await;
    t.sweep();
    assert_eq!(t.pending(), 1);
    std::fs::write(receive.path(), b"abc").unwrap();
    receive.finish(3);
    assert!(t.claim(&[id], is_track).is_ok());
}

#[tokio::test(start_paused = true)]
async fn the_timer_sweeps_with_nobody_asking() {
    let t = Transfers::new();
    let id = sent(&t, track(), b"abc");
    assert_eq!(files(&t), vec![id]);
    // Let the sweeper start its wait, then pass the lifetime and a sweep.
    tokio::task::yield_now().await;
    tokio::time::advance(10 * MINUTE + SWEEP_EVERY * 2).await;
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
    assert!(files(&t).is_empty());
    assert_eq!(t.pending(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_download_is_fetched_repeatedly_until_ten_minutes_after_it_was_staged() {
    let t = Transfers::new();
    let id = t.stage_download("kellys.mvr".to_string(), b"mvr").unwrap();
    for _ in 0..3 {
        let (path, name) = t.download(&id).unwrap();
        assert_eq!(name, "kellys.mvr");
        assert_eq!(std::fs::read(path).unwrap(), b"mvr");
        tokio::time::advance(3 * MINUTE).await;
    }
    // Fetching did not extend it: nine minutes in, one more minute ends it.
    assert!(t.download(&id).is_ok());
    tokio::time::advance(MINUTE).await;
    assert_eq!(t.download(&id).err(), Some(TransferError::NotFound));
    assert!(files(&t).is_empty());
}

#[test]
fn a_download_is_not_an_upload_and_an_upload_is_not_a_download() {
    let t = Transfers::new();
    let download = t.stage_download("a.mvr".to_string(), b"x").unwrap();
    let upload = sent(&t, track(), b"x");
    assert_eq!(t.receive(&download).err(), Some(TransferError::NotFound));
    assert_eq!(
        t.claim(std::slice::from_ref(&download), |_| true).err(),
        Some(TransferError::NotFound)
    );
    assert_eq!(t.download(&upload).err(), Some(TransferError::NotFound));
}

#[test]
fn a_claimed_file_is_written_with_the_permissions_of_a_new_file() {
    use std::os::unix::fs::PermissionsExt;
    let t = Transfers::new();
    let id = sent(&t, track(), b"abc");
    let claimed = t.claim(&[id], is_track).unwrap();
    let out = tempfile::tempdir().unwrap();
    let dest = out.path().join("kick.wav");
    claimed[0].write_to(&dest).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
    // Not the staging file's 0600 (the umask decides; a new file is not
    // private to its owner by being staged).
    let staged = std::fs::metadata(claimed[0].path().unwrap())
        .unwrap()
        .permissions()
        .mode();
    let made = std::fs::metadata(&dest).unwrap().permissions().mode();
    let fresh = std::fs::File::create(out.path().join("fresh")).unwrap();
    let expected = fresh.metadata().unwrap().permissions().mode();
    assert_eq!(made & 0o777, expected & 0o777);
    let _ = staged;
}

#[test]
fn content_disposition_is_safe_for_awkward_names() {
    assert_eq!(
        content_disposition("kellys.mvr"),
        "attachment; filename=\"kellys.mvr\""
    );
    let awkward = content_disposition("Bühne \"1\".mvr");
    assert!(awkward.contains("filename=\"B_hne _1_.mvr\""), "{awkward}");
    assert!(
        awkward.contains("filename*=UTF-8''B%C3%BChne%20%221%22.mvr"),
        "{awkward}"
    );
}
