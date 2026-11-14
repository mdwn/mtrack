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

//! `ConfigService`'s upload calls (`BeginUpload`, `AddTrackFiles`,
//! `AddSampleFile`), called as the service: the behaviour of the REST routes
//! they replaced (the track and sample uploads), each held by a test here,
//! and the new upload's own: the lock, the cap, the id, the claim.

use std::path::PathBuf;
use std::sync::Arc;

use tonic::{Code, Request, Status};

use crate::config;
use crate::controller::grpc::config_service::ConfigServer;
use crate::proto::player::v1::{
    self as proto, config_service_server::ConfigService, UploadPurpose,
};
use crate::webui::api::test_helpers::{create_test_wav, test_state};
use crate::webui::server::WebUiState;
use crate::webui::transfers::Purpose;

/// A project on disk (`mtrack.yaml`, an empty `songs/`), and the service
/// over its player, which has a config store and is unlocked.
struct Project {
    state: WebUiState,
    dir: tempfile::TempDir,
    server: ConfigServer,
}

fn project() -> Project {
    let (state, dir) = test_state();
    let store = Arc::new(config::ConfigStore::new(
        config::Player::deserialize(&state.config_path).unwrap(),
        state.config_path.clone(),
    ));
    state.player.set_config_store(store);
    state.player.set_locked(false);
    Project {
        server: ConfigServer::new(state.player.clone()),
        state,
        dir,
    }
}

fn begin_request(
    purpose: UploadPurpose,
    file_name: &str,
    size: Option<u64>,
    song: Option<&str>,
) -> proto::BeginUploadRequest {
    proto::BeginUploadRequest {
        purpose: purpose.into(),
        file_name: file_name.to_string(),
        size,
        song: song.map(str::to_string),
    }
}

impl Project {
    fn songs(&self) -> PathBuf {
        self.state.songs_path()
    }

    fn samples(&self) -> PathBuf {
        self.dir.path().join("samples")
    }

    async fn begin(
        &self,
        purpose: UploadPurpose,
        file_name: &str,
        size: Option<u64>,
        song: Option<&str>,
    ) -> Result<proto::BeginUploadResponse, Status> {
        self.server
            .begin_upload(Request::new(begin_request(purpose, file_name, size, song)))
            .await
            .map(|r| r.into_inner())
    }

    /// Sends the bytes of an announced upload, as the web server does.
    fn put(&self, id: &str, bytes: &[u8]) {
        let receive = self.state.player.transfers().receive(id).unwrap();
        std::fs::write(receive.path(), bytes).unwrap();
        receive.finish(bytes.len() as u64);
    }

    /// An upload announced through the RPC and sent.
    async fn upload(
        &self,
        purpose: UploadPurpose,
        file_name: &str,
        song: Option<&str>,
        bytes: &[u8],
    ) -> String {
        let begun = self
            .begin(purpose, file_name, Some(bytes.len() as u64), song)
            .await
            .unwrap_or_else(|e| panic!("{file_name}: {e}"));
        self.put(&begun.upload_id, bytes);
        begun.upload_id
    }

    /// A track upload announced and sent.
    async fn track(&self, song: &str, file_name: &str, bytes: &[u8]) -> String {
        self.upload(UploadPurpose::Track, file_name, Some(song), bytes)
            .await
    }

    async fn add(&self, song: &str, ids: &[&str]) -> Result<proto::AddTrackFilesResponse, Status> {
        self.server
            .add_track_files(Request::new(proto::AddTrackFilesRequest {
                song: song.to_string(),
                upload_ids: ids.iter().map(|id| id.to_string()).collect(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    /// One track file announced, sent and added.
    async fn add_one(
        &self,
        song: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<proto::AddTrackFilesResponse, Status> {
        let id = self.track(song, file_name, bytes).await;
        self.add(song, &[&id]).await
    }

    async fn add_sample(&self, id: &str) -> Result<proto::AddSampleFileResponse, Status> {
        self.server
            .add_sample_file(Request::new(proto::AddSampleFileRequest {
                upload_id: id.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    /// A sample announced, sent and added.
    async fn add_one_sample(
        &self,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<proto::AddSampleFileResponse, Status> {
        let id = self
            .upload(UploadPurpose::Sample, file_name, None, bytes)
            .await;
        self.add_sample(&id).await
    }

    /// An upload registered with no checks at all, for the claim to meet:
    /// what a caller who got past (or around) `BeginUpload` would hold.
    fn unchecked(&self, purpose: Purpose, file_name: &str, bytes: &[u8]) -> String {
        let transfers = self.state.player.transfers();
        let id = transfers
            .begin_upload(purpose, file_name.to_string(), bytes.len() as u64)
            .unwrap();
        self.put(&id, bytes);
        id
    }

    fn staged(&self) -> usize {
        std::fs::read_dir(self.state.player.transfers().staging_dir().unwrap())
            .unwrap()
            .count()
    }
}

fn track_purpose(song: &str) -> Purpose {
    Purpose::Track {
        song: song.to_string(),
    }
}

// ---- BeginUpload ----

#[tokio::test]
async fn an_upload_is_announced_with_an_id_and_a_web_path() {
    let p = project();
    let begun = p
        .begin(UploadPurpose::Sample, "kick.wav", Some(10), None)
        .await
        .unwrap();
    assert_eq!(begun.upload_id.len(), 32);
    assert!(begun.upload_id.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(begun.path, format!("/uploads/{}", begun.upload_id));
    let other = p
        .begin(UploadPurpose::Sample, "kick.wav", Some(10), None)
        .await
        .unwrap();
    assert_ne!(begun.upload_id, other.upload_id);
}

#[tokio::test]
async fn announcing_an_upload_makes_nothing() {
    let p = project();
    p.begin(UploadPurpose::Track, "t.wav", Some(1), Some("NewSong"))
        .await
        .unwrap();
    p.begin(UploadPurpose::Sample, "kick.wav", Some(1), None)
        .await
        .unwrap();
    assert!(!p.songs().join("NewSong").exists());
    assert!(!p.samples().exists());
    assert_eq!(p.staged(), 0, "no staging file until the bytes arrive");
}

#[tokio::test]
async fn a_purpose_and_a_size_are_required() {
    let p = project();
    let refused = p
        .begin(UploadPurpose::Unspecified, "a.wav", Some(1), None)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    // A purpose number the proto does not name is the same.
    let refused = p
        .server
        .begin_upload(Request::new(proto::BeginUploadRequest {
            purpose: 99,
            file_name: "a.wav".to_string(),
            size: Some(1),
            song: None,
        }))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    for purpose in [
        UploadPurpose::Track,
        UploadPurpose::Sample,
        UploadPurpose::GdtfImport,
        UploadPurpose::MvrInspect,
        UploadPurpose::MvrImport,
    ] {
        let file = if matches!(
            purpose,
            UploadPurpose::MvrInspect | UploadPurpose::MvrImport
        ) {
            "a.mvr"
        } else {
            "a.wav"
        };
        let refused = p.begin(purpose, file, None, Some("S")).await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{purpose:?}");
        assert!(refused.message().contains("size"), "{}", refused.message());
    }
    assert_eq!(p.state.player.transfers().pending(), 0);
}

#[tokio::test]
async fn a_song_is_for_a_track_and_a_track_needs_one() {
    let p = project();
    let refused = p
        .begin(UploadPurpose::Track, "a.wav", Some(1), None)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    for purpose in [
        UploadPurpose::Sample,
        UploadPurpose::GdtfImport,
        UploadPurpose::MvrImport,
    ] {
        let refused = p
            .begin(purpose, "a.wav", Some(1), Some("Song"))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{purpose:?}");
    }
}

#[tokio::test]
async fn a_gdtf_or_mvr_may_declare_the_most_the_transport_allowed_and_no_more() {
    let p = project();
    let cap = 272 * 1024 * 1024;
    for (purpose, file) in [
        (UploadPurpose::GdtfImport, "a.gdtf"),
        (UploadPurpose::MvrInspect, "a.mvr"),
        (UploadPurpose::MvrImport, "a.mvr"),
    ] {
        p.begin(purpose, file, Some(cap), None).await.unwrap();
        let refused = p
            .begin(purpose, file, Some(cap + 1), None)
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{purpose:?}");
        assert!(
            refused.message().contains("272 MiB"),
            "{}",
            refused.message()
        );
    }
}

#[tokio::test]
async fn a_track_or_sample_has_no_cap_beyond_its_declared_size() {
    let p = project();
    let huge = 40 * 1024 * 1024 * 1024u64;
    p.begin(UploadPurpose::Track, "a.wav", Some(huge), Some("S"))
        .await
        .unwrap();
    p.begin(UploadPurpose::Sample, "a.wav", Some(huge), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn at_most_32_uploads_are_pending() {
    let p = project();
    for n in 0..32 {
        p.begin(UploadPurpose::Sample, &format!("s{n}.wav"), Some(1), None)
            .await
            .unwrap_or_else(|e| panic!("{n}: {e}"));
    }
    let refused = p
        .begin(UploadPurpose::Sample, "one-too-many.wav", Some(1), None)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::ResourceExhausted);
}

#[tokio::test]
async fn a_sample_upload_needs_a_config_store() {
    let (state, _dir) = test_state();
    state.player.set_locked(false);
    let server = ConfigServer::new(state.player.clone());
    let refused = server
        .begin_upload(Request::new(begin_request(
            UploadPurpose::Sample,
            "a.wav",
            Some(1),
            None,
        )))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::Unimplemented);
}

/// Every purpose but an MVR inspection (a pure read) is refused while the
/// player is locked, with the lock's own status and message.
#[tokio::test]
async fn while_locked_only_an_mvr_inspection_may_be_announced() {
    let p = project();
    let locked = crate::player::LockedError.to_string();
    let cases = [
        (UploadPurpose::Track, "a.wav", Some("S")),
        (UploadPurpose::Sample, "a.wav", None),
        (UploadPurpose::GdtfImport, "a.gdtf", None),
        (UploadPurpose::MvrInspect, "a.mvr", None),
        (UploadPurpose::MvrImport, "a.mvr", None),
        (UploadPurpose::Unspecified, "a.wav", None),
    ];
    for is_locked in [false, true] {
        p.state.player.set_locked(is_locked);
        for (purpose, file, song) in cases {
            let answer = p.begin(purpose, file, Some(1), song).await;
            let allowed = !is_locked || purpose == UploadPurpose::MvrInspect;
            let valid = purpose != UploadPurpose::Unspecified;
            match answer {
                Ok(_) => assert!(allowed && valid, "{purpose:?} locked={is_locked}"),
                Err(e) if allowed => {
                    assert!(!valid, "{purpose:?} locked={is_locked}: {e}");
                    assert_ne!(e.message(), locked);
                }
                Err(e) => {
                    assert_eq!(e.code(), Code::FailedPrecondition, "{purpose:?}");
                    assert_eq!(e.message(), locked, "{purpose:?}");
                }
            }
        }
    }
    assert_eq!(
        p.state.player.transfers().pending(),
        // Unlocked: five valid purposes; locked: the inspection.
        6
    );
}

/// The lock table, as the lock guard reads it: the claiming calls are
/// refused before they run, the announcing call and the two reads are not.
#[tokio::test]
async fn the_claiming_calls_are_locked_out_and_the_announcement_is_not() {
    use tower::ServiceExt;
    let p = project();
    p.state.player.set_locked(true);
    let locked = crate::player::LockedError.to_string();
    let message_of = |service: &'static str, method: &'static str| {
        let player = p.state.player.clone();
        async move {
            let request = http::Request::post(format!("/player.v1.{service}/{method}"))
                .header("content-type", "application/grpc")
                .body(tonic::body::Body::empty())
                .unwrap();
            let response = match service {
                "ConfigService" => crate::controller::grpc::config_service(player)
                    .oneshot(request)
                    .await
                    .unwrap(),
                _ => crate::controller::grpc::lighting_service(player)
                    .oneshot(request)
                    .await
                    .unwrap(),
            };
            tonic::Status::from_header_map(response.headers()).map(|s| s.message().to_string())
        }
    };
    for (service, method) in [
        ("ConfigService", "AddTrackFiles"),
        ("ConfigService", "AddSampleFile"),
        ("LightingService", "ImportGdtf"),
        ("LightingService", "ImportMvr"),
    ] {
        assert_eq!(
            message_of(service, method).await.expect(method),
            locked,
            "{method}"
        );
    }
    for (service, method) in [
        ("ConfigService", "BeginUpload"),
        ("LightingService", "InspectMvr"),
        ("LightingService", "ExportMvr"),
    ] {
        // Past the guard: whatever it answers, it is not the lock.
        if let Some(message) = message_of(service, method).await {
            assert_ne!(message, locked, "{method}");
        }
    }
}

// ---- The claim ----

#[tokio::test]
async fn an_id_that_is_unknown_is_not_found() {
    let p = project();
    let refused = p.add("Song", &["0123456789abcdef0123456789abcdef"]).await;
    assert_eq!(refused.unwrap_err().code(), Code::NotFound);
    let refused = p.add_sample("0123456789abcdef0123456789abcdef").await;
    assert_eq!(refused.unwrap_err().code(), Code::NotFound);
}

#[tokio::test]
async fn an_upload_is_good_for_its_purpose_and_its_song_alone() {
    let p = project();
    let sample = p.upload(UploadPurpose::Sample, "k.wav", None, b"x").await;
    let refused = p.add("Song", &[&sample]).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    // Not spent: the sample call still takes it.
    p.add_sample(&sample).await.unwrap();

    let track = p.track("Song", "t.wav", &create_test_wav()).await;
    let refused = p.add_sample(&track).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let refused = p.add("Other", &[&track]).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(!p.songs().join("Other").exists());
    p.add("Song", &[&track]).await.unwrap();

    // The GDTF and MVR purposes are not tracks.
    let gdtf = p
        .upload(UploadPurpose::GdtfImport, "a.gdtf", None, b"x")
        .await;
    assert_eq!(
        p.add("Song", &[&gdtf]).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
}

#[tokio::test]
async fn an_upload_is_used_once() {
    let p = project();
    let id = p.track("Song", "t.wav", &create_test_wav()).await;
    p.add("Song", &[&id]).await.unwrap();
    assert_eq!(
        p.add("Song", &[&id]).await.unwrap_err().code(),
        Code::NotFound
    );
    let id = p.upload(UploadPurpose::Sample, "k.wav", None, b"x").await;
    p.add_sample(&id).await.unwrap();
    assert_eq!(p.add_sample(&id).await.unwrap_err().code(), Code::NotFound);
    assert_eq!(p.staged(), 0, "the staged files went with their claims");
}

#[tokio::test]
async fn a_claim_before_the_bytes_arrive_is_refused_and_leaves_the_upload() {
    let p = project();
    let begun = p
        .begin(UploadPurpose::Sample, "k.wav", Some(4), None)
        .await
        .unwrap();
    let refused = p.add_sample(&begun.upload_id).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    p.put(&begun.upload_id, b"data");
    p.add_sample(&begun.upload_id).await.unwrap();
}

#[tokio::test]
async fn a_body_shorter_than_announced_is_refused_and_spent() {
    let p = project();
    let begun = p
        .begin(UploadPurpose::Sample, "k.wav", Some(10), None)
        .await
        .unwrap();
    p.put(&begun.upload_id, b"short");
    let refused = p.add_sample(&begun.upload_id).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(refused.message().contains("10"), "{}", refused.message());
    assert!(!p.samples().join("k.wav").exists());
    assert_eq!(p.staged(), 0);
    assert_eq!(
        p.add_sample(&begun.upload_id).await.unwrap_err().code(),
        Code::NotFound
    );
}

#[tokio::test(start_paused = true)]
async fn a_claim_after_the_upload_expired_is_not_found() {
    let p = project();
    let minutes = |n: u64| std::time::Duration::from_secs(60 * n);

    // Not sent: ten minutes after Begin.
    let unsent = p
        .begin(UploadPurpose::Sample, "k.wav", Some(1), None)
        .await
        .unwrap();
    // Sent: ten minutes after the last byte.
    let sent = p.upload(UploadPurpose::Sample, "j.wav", None, b"x").await;
    tokio::time::advance(minutes(10)).await;
    assert_eq!(
        p.add_sample(&unsent.upload_id).await.unwrap_err().code(),
        Code::NotFound
    );
    assert_eq!(
        p.add_sample(&sent).await.unwrap_err().code(),
        Code::NotFound
    );
    assert!(!p.samples().exists());
    assert_eq!(p.staged(), 0);
}

#[tokio::test]
async fn a_claim_of_several_with_one_bad_takes_none() {
    let p = project();
    let wav = create_test_wav();
    let good = p.track("Song", "a.wav", &wav).await;
    let sample = p.upload(UploadPurpose::Sample, "k.wav", None, b"x").await;
    let refused = p.add("Song", &[&good, &sample]).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(!p.songs().join("Song").exists());
    // Both still good.
    p.add("Song", &[&good]).await.unwrap();
    p.add_sample(&sample).await.unwrap();

    // A name twice in one claim is refused.
    let id = p.track("Twice", "a.wav", &wav).await;
    assert_eq!(
        p.add("Twice", &[&id, &id]).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
}

// ---- Tracks (the PUT /songs/{name}/tracks/{filename} and POST /songs/{name}/tracks routes) ----

#[tokio::test]
async fn a_track_creates_the_song() {
    let p = project();
    let added = p
        .add_one("NewSong", "track1.wav", &create_test_wav())
        .await
        .unwrap();
    assert_eq!(added.song, "NewSong");
    assert_eq!(added.files.len(), 1);
    assert_eq!(added.files[0].file, "track1.wav");
    assert!(!added.files[0].replaced);
    assert!(p.songs().join("NewSong/track1.wav").exists());
    assert!(p.songs().join("NewSong/song.yaml").exists());
}

#[tokio::test]
async fn a_track_sent_again_replaces_the_first_and_says_so() {
    let p = project();
    p.add_one("NewSong", "track1.wav", &create_test_wav())
        .await
        .unwrap();
    let added = p
        .add_one("NewSong", "track1.wav", &create_test_wav())
        .await
        .unwrap();
    assert!(added.files[0].replaced);
}

/// #674: an upload's new song is named by the bulk import's rule: its
/// folder's name, unless a song configured under the songs root has it
/// (here one written since the last reload), then the first free suffix.
#[tokio::test]
async fn a_new_song_is_not_named_as_a_song_already_configured() {
    let p = project();
    let other = p.songs().join("Other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("song.yaml"), "name: Fresh\ntracks: []\n").unwrap();
    let added = p
        .add_one("Fresh", "track1.wav", &create_test_wav())
        .await
        .unwrap();
    // The answer gives the name the song has, for the client to go on.
    assert_eq!(added.song, "Fresh (2)");
    let config = config::Song::deserialize(&p.songs().join("Fresh/song.yaml")).unwrap();
    assert_eq!(config.name(), "Fresh (2)");
}

/// An extension in capitals is the same type: `Bass.WAV` is taken and
/// becomes a track, `Notes.MID` and `Show.LIGHT` too.
#[tokio::test]
async fn an_extension_is_read_in_any_case() {
    let p = project();
    p.add_one("NewSong", "Bass.WAV", &create_test_wav())
        .await
        .unwrap();
    let config = std::fs::read_to_string(p.songs().join("NewSong/song.yaml")).unwrap();
    assert!(config.contains("Bass.WAV"), "{config}");

    let midi =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid")).unwrap();
    p.add_one("MidiSong", "Notes.MID", &midi).await.unwrap();
    assert!(p.songs().join("MidiSong/Notes.MID").exists());

    let show =
        "show \"Loud\" {\n    @00:00.000\n    front: static color: \"blue\", duration: 5s\n}\n";
    p.add_one("LightSong", "Show.LIGHT", show.as_bytes())
        .await
        .unwrap();
    assert!(p.songs().join("LightSong/Show.LIGHT").exists());
}

#[tokio::test]
async fn a_midi_and_a_light_file_are_added_by_name() {
    let p = project();
    let midi =
        std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid")).unwrap();
    let added = p.add_one("MidiSong", "notes.mid", &midi).await.unwrap();
    assert_eq!(added.files[0].file, "notes.mid");
    assert!(p.songs().join("MidiSong/notes.mid").exists());
    let added = p
        .add_one("LightSong", "show.light", b"show \"Test\" {}\n")
        .await
        .unwrap();
    assert_eq!(added.files[0].file, "show.light");
    assert!(p.songs().join("LightSong/show.light").exists());
}

#[tokio::test]
async fn several_files_create_the_song_in_one_call() {
    let p = project();
    let wav = create_test_wav();
    let one = p.track("MultiSong", "track1.wav", &wav).await;
    let two = p.track("MultiSong", "track2.wav", &wav).await;
    let added = p.add("MultiSong", &[&one, &two]).await.unwrap();
    assert_eq!(added.song, "MultiSong");
    let files: Vec<&str> = added.files.iter().map(|f| f.file.as_str()).collect();
    assert_eq!(files, vec!["track1.wav", "track2.wav"]);
    assert!(p.songs().join("MultiSong/track1.wav").exists());
    assert!(p.songs().join("MultiSong/track2.wav").exists());
    assert!(p.songs().join("MultiSong/song.yaml").exists());
}

#[tokio::test]
async fn no_files_is_refused() {
    let p = project();
    let refused = p.add("EmptySong", &[]).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains("No files uploaded"));
    assert!(!p.songs().join("EmptySong").exists());
}

#[tokio::test]
async fn a_track_is_added_to_an_existing_song() {
    let p = project();
    let song_dir = p.songs().join("ExistingSong");
    std::fs::create_dir(&song_dir).unwrap();
    crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
        .unwrap();
    p.add_one("ExistingSong", "track2.wav", &create_test_wav())
        .await
        .unwrap();
    assert!(song_dir.join("track1.wav").exists());
    assert!(song_dir.join("track2.wav").exists());
    assert!(song_dir.join("song.yaml").exists());
}

/// Beside a `song.yaml` of another kind (`kind: playlist`), a create is
/// refused and a track upload writes no config: neither writes over it.
#[tokio::test]
async fn a_track_keeps_a_song_yaml_of_another_kind() {
    let p = project();
    let song_dir = p.songs().join("Pl");
    std::fs::create_dir(&song_dir).unwrap();
    let playlist = "kind: playlist\nsongs: []\n";
    std::fs::write(song_dir.join("song.yaml"), playlist).unwrap();

    let created = p
        .server
        .create_song(Request::new(proto::CreateSongRequest {
            name: "Pl".to_string(),
            yaml: "name: Pl\ntracks: []\n".to_string(),
        }))
        .await
        .unwrap_err();
    assert_eq!(created.code(), Code::AlreadyExists);

    p.add_one("Pl", "guitar.wav", &create_test_wav())
        .await
        .unwrap();
    assert!(song_dir.join("guitar.wav").exists());
    assert_eq!(
        std::fs::read_to_string(song_dir.join("song.yaml")).unwrap(),
        playlist
    );
}

/// A track uploaded to a song whose config is `song.yml` leaves that config
/// alone and writes no second `song.yaml`.
#[tokio::test]
async fn a_track_to_a_song_yml_song_writes_no_song_yaml() {
    let p = project();
    let song_dir = p.songs().join("YmlSong");
    std::fs::create_dir(&song_dir).unwrap();
    let config = "name: YmlSong\ntracks: []\n";
    std::fs::write(song_dir.join("song.yml"), config).unwrap();
    p.state.player.reload_songs().await.unwrap();

    p.add_one("YmlSong", "guitar.wav", &create_test_wav())
        .await
        .unwrap();
    assert!(!song_dir.join("song.yaml").exists());
    assert_eq!(
        std::fs::read_to_string(song_dir.join("song.yml")).unwrap(),
        config
    );
}

#[tokio::test]
async fn a_track_keeps_the_config_a_song_was_created_with() {
    let p = project();
    let yaml = "name: My Custom Song\ntracks:\n  - name: Lead Guitar\n    file: guitar.wav\n";
    p.server
        .create_song(Request::new(proto::CreateSongRequest {
            name: "CustomSong".to_string(),
            yaml: yaml.to_string(),
        }))
        .await
        .unwrap();
    p.add_one("CustomSong", "guitar.wav", &create_test_wav())
        .await
        .unwrap();
    let content = std::fs::read_to_string(p.songs().join("CustomSong/song.yaml")).unwrap();
    assert!(content.contains("My Custom Song"));
    assert!(content.contains("Lead Guitar"));
}

/// A track uploaded to a flat-layout song (`<root>/flat.yaml`) writes no
/// `song.yaml` into the songs root: the root holds that song's config.
#[tokio::test]
async fn a_track_to_a_flat_song_writes_no_root_song_yaml() {
    let p = project();
    let root = p.songs();
    std::fs::write(root.join("flat.yaml"), "name: Flat\ntracks: []\n").unwrap();
    p.state.player.reload_songs().await.unwrap();

    p.add_one("Flat", "guitar.wav", &create_test_wav())
        .await
        .unwrap();
    assert!(!root.join("song.yaml").exists());
}

#[tokio::test]
async fn an_unsupported_file_type_is_refused() {
    let p = project();
    let refused = p
        .begin(UploadPurpose::Track, "file.txt", Some(4), Some("TestSong"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains("Unsupported file type"));
    // The claim says the same of an upload that was not checked.
    let id = p.unchecked(track_purpose("TestSong"), "file.txt", b"data");
    let refused = p.add("TestSong", &[&id]).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains("Unsupported file type"));
    assert!(!p.songs().join("TestSong").exists());
    assert_eq!(p.staged(), 0, "the upload is spent");
}

/// What a caller may name: the song, and the file. Each is held to the
/// route's rules at BeginUpload, and again at the claim, which is all that
/// stands between an upload registered another way and the disk.
#[tokio::test]
async fn names_that_are_not_safe_are_refused_at_both_steps() {
    let p = project();
    let wav = create_test_wav();
    let outside = p.dir.path().join("outside-marker");
    let hostile_songs = [
        "..",
        "../..",
        "../../etc",
        "/etc",
        "/etc/passwd",
        "a/b",
        "a\\b",
        "a\0b",
        ".hidden",
        " lead",
        "trail ",
        ".",
        "",
    ];
    for song in hostile_songs {
        let refused = p
            .begin(UploadPurpose::Track, "t.wav", Some(1), Some(song))
            .await;
        assert!(refused.is_err(), "begin accepted song {song:?}");
        let id = p.unchecked(track_purpose(song), "t.wav", &wav);
        let refused = p.add(song, &[&id]).await;
        assert!(refused.is_err(), "claim accepted song {song:?}");
        assert_eq!(
            refused.unwrap_err().code(),
            Code::InvalidArgument,
            "{song:?}"
        );
    }
    let hostile_files = [
        "..",
        "../t.wav",
        "../../t.wav",
        "/etc/t.wav",
        "a/t.wav",
        "a\\t.wav",
        "t\0.wav",
        ".hidden.wav",
        " t.wav",
        "t.wav ",
        "",
        ".",
        "CON.txt",
        "noext",
    ];
    for file in hostile_files {
        let refused = p
            .begin(UploadPurpose::Track, file, Some(1), Some("Song"))
            .await;
        assert!(refused.is_err(), "begin accepted file {file:?}");
        let id = p.unchecked(track_purpose("Song"), file, &wav);
        let refused = p.add("Song", &[&id]).await;
        assert!(refused.is_err(), "claim accepted file {file:?}");
        assert_eq!(
            refused.unwrap_err().code(),
            Code::InvalidArgument,
            "{file:?}"
        );
    }
    assert!(
        !p.songs().join("Song").exists(),
        "no song made for a bad file"
    );
    assert!(!outside.exists());
    assert_eq!(
        std::fs::read_dir(p.songs()).unwrap().count(),
        0,
        "nothing was made in the songs directory"
    );
    assert_eq!(p.staged(), 0);
}

/// A song directory that is a symlink out of the project is refused, at the
/// announcement and at the claim, and nothing is written through it.
#[tokio::test]
async fn a_song_folder_that_links_out_of_the_project_is_refused() {
    let p = project();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), p.songs().join("Link")).unwrap();

    let refused = p
        .begin(UploadPurpose::Track, "t.wav", Some(1), Some("Link"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied);
    let id = p.unchecked(track_purpose("Link"), "t.wav", &create_test_wav());
    let refused = p.add("Link", &[&id]).await.unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied);
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_new_song_may_not_be_a_hidden_folder() {
    let p = project();
    let refused = p
        .begin(UploadPurpose::Track, "t.wav", Some(16), Some(".hidden"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(
        !p.songs().join(".hidden").exists(),
        ".hidden song dir created"
    );
}

// ---- Samples (the PUT /samples/upload/{filename} route) ----

#[tokio::test]
async fn a_sample_is_stored_in_a_samples_directory_made_for_it() {
    let p = project();
    let added = p
        .add_one_sample("kick.wav", &create_test_wav())
        .await
        .unwrap();
    assert_eq!(added.file, "kick.wav");
    assert_eq!(added.path, "samples/kick.wav");
    // In samples/ next to the config.
    assert_eq!(
        std::fs::read(p.samples().join("kick.wav")).unwrap(),
        create_test_wav()
    );
}

/// A sample's extension is read in any case: `Kick.WAV` is audio.
#[tokio::test]
async fn a_sample_extension_is_read_in_any_case() {
    let p = project();
    p.add_one_sample("Kick.WAV", &create_test_wav())
        .await
        .unwrap();
    assert!(p.samples().join("Kick.WAV").exists());
}

#[tokio::test]
async fn a_sample_sent_again_replaces_the_first() {
    let p = project();
    p.add_one_sample("kick.wav", b"RIFF-one").await.unwrap();
    p.add_one_sample("kick.wav", b"RIFF-two!").await.unwrap();
    assert_eq!(
        std::fs::read(p.samples().join("kick.wav")).unwrap(),
        b"RIFF-two!"
    );
}

#[tokio::test]
async fn a_sample_that_is_not_audio_is_refused() {
    let p = project();
    let refused = p
        .begin(UploadPurpose::Sample, "readme.txt", Some(4), None)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains("Unsupported audio file type"));
    let id = p.unchecked(Purpose::Sample, "readme.txt", b"data");
    let refused = p.add_sample(&id).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains("Unsupported audio file type"));
    assert!(!p.samples().join("readme.txt").exists());
}

/// A new sample's name is held to the rules for new names; one already
/// there is replaced under its own, as the route did.
#[tokio::test]
async fn a_new_sample_name_is_held_to_the_new_name_rules() {
    let p = project();
    let wav = create_test_wav();
    for name in [".hidden.wav", " lead.wav", "tab\t.wav", "nl\n.wav"] {
        let refused = p.begin(UploadPurpose::Sample, name, Some(1), None).await;
        assert_eq!(
            refused.unwrap_err().code(),
            Code::InvalidArgument,
            "{name:?}"
        );
        let id = p.unchecked(Purpose::Sample, name, &wav);
        assert_eq!(
            p.add_sample(&id).await.unwrap_err().code(),
            Code::InvalidArgument,
            "{name:?}"
        );
    }
    // One that was there before the rules is replaced, not refused.
    std::fs::create_dir_all(p.samples()).unwrap();
    std::fs::write(p.samples().join(" old.wav"), b"old").unwrap();
    p.add_one_sample(" old.wav", b"new").await.unwrap();
    assert_eq!(std::fs::read(p.samples().join(" old.wav")).unwrap(), b"new");
}

#[tokio::test]
async fn sample_names_that_are_not_safe_are_refused_at_both_steps() {
    let p = project();
    for name in [
        "..",
        "../kick.wav",
        "../../etc/passwd",
        "/etc/kick.wav",
        "a/kick.wav",
        "a\\kick.wav",
        "kick\0.wav",
        "",
        ".",
        "..%2F..%2Fetc%2Fpasswd",
    ] {
        let refused = p.begin(UploadPurpose::Sample, name, Some(1), None).await;
        assert!(refused.is_err(), "begin accepted {name:?}");
        let id = p.unchecked(Purpose::Sample, name, b"bad");
        let refused = p.add_sample(&id).await;
        assert!(refused.is_err(), "claim accepted {name:?}");
        assert_eq!(
            refused.unwrap_err().code(),
            Code::InvalidArgument,
            "{name:?}"
        );
    }
    assert!(!p.samples().exists(), "no samples directory for a bad name");
    assert_eq!(p.staged(), 0);
}
