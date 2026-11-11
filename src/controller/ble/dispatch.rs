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

//! Turns one request frame into one response frame by calling the player's
//! gRPC service in process, behind the same lock guard and handlers gRPC
//! serves. A server-streaming method is opened the same way, and its
//! response body read message by message instead of whole.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use http_body_util::{BodyExt, Full};
use prost::Message;
use tonic::{codegen::Bytes, Code, Status};
use tower::ServiceExt;

use crate::player::Player;
use crate::proto::ble::v1::{Request, Response};

use super::codec::MAX_FRAME;
use super::stream::{Messages, Replaceable, Step};

use super::super::grpc::{player_service, PlayerServer};
use crate::proto::player::v1::player_service_server::PlayerServiceServer;

/// The `PlayerService` methods a Bluetooth peer may call. Anything else is
/// refused before the service sees it, which also keeps the method name
/// from ever being anything but one of these literals in the request path.
pub(crate) const ALLOWED: &[&str] = &[
    "Play",
    "PlayFrom",
    "PlaySongFrom",
    "Stop",
    "Next",
    "NavigateToSong",
    "Previous",
    "Seek",
    "SeekToSection",
    "SwitchToPlaylist",
    "Status",
    "LoopSection",
    "StopSectionLoop",
    "SectionAck",
    "StopSamples",
    "CancelAutoPlay",
    "ListPlaylists",
    "GetPlaylist",
    "GetTrackGains",
    "SetTrackGain",
    "SetTrackMute",
    "SetAmbientMode",
    "SavePlaylist",
    "DeletePlaylist",
    "SetLock",
    "WatchStage",
];

/// An allowed method that is a server stream: it is answered on the stream
/// characteristic, message by message.
pub(crate) struct StreamMethod {
    /// Its name, which [`ALLOWED`] lists too.
    pub(crate) name: &'static str,
    /// Which of its messages a newer one replaces while it waits for the
    /// radio, told from the serialized message. The rest are never dropped.
    pub(crate) replaceable: Replaceable,
}

/// The allowed methods that are server streams. Every other allowed method
/// is unary; a test holds both halves to the service's descriptor.
pub(crate) const STREAMS: &[StreamMethod] = &[StreamMethod {
    name: "WatchStage",
    replaceable: stage_update_is_a_frame,
}];

/// The allowed server stream called `method`, if it is one.
pub(crate) fn stream_method(method: &str) -> Option<&'static StreamMethod> {
    STREAMS.iter().find(|stream| stream.name == method)
}

/// The tag of `StageUpdate.frame`: field 2, length-delimited.
const STAGE_UPDATE_FRAME_TAG: u8 = (2 << 3) | 2;

/// Whether a serialized `StageUpdate` holds a frame, which stands alone and
/// is worthless once a newer one exists, and not a layout, which every later
/// frame is read against. `StageUpdate` is one oneof and nothing else, so
/// the message the service encodes begins with the tag of the member set: an
/// update with neither (no bytes) is not a frame, and is kept.
fn stage_update_is_a_frame(message: &[u8]) -> bool {
    message.first() == Some(&STAGE_UPDATE_FRAME_TAG)
}

/// The allowed methods that write playlists: refused while the player is
/// locked, which the phone lifts with `SetLock`.
#[cfg(test)]
const LOCKED_WRITES: &[&str] = &["SavePlaylist", "DeletePlaylist"];

/// The service name in a gRPC request path.
const SERVICE: &str = <PlayerServiceServer<PlayerServer> as tonic::server::NamedService>::NAME;

/// The size of a gRPC message's prefix: a compressed flag and a big-endian
/// length.
const PREFIX_LEN: usize = 5;

type Service = super::super::grpc::LockGuard<PlayerServiceServer<PlayerServer>>;

/// Answers request frames from a Bluetooth peer.
pub(crate) struct Dispatcher {
    service: Service,
}

impl Dispatcher {
    pub(crate) fn new(player: Arc<Player>) -> Dispatcher {
        Dispatcher {
            service: player_service(player),
        }
    }

    /// Answers one request frame with one encoded [`Response`]: the
    /// service's, or a status saying why there is none.
    pub(crate) async fn handle(&self, frame: &[u8]) -> Vec<u8> {
        let request = match Request::decode(frame) {
            Ok(request) => request,
            Err(e) => {
                return failure(0, Code::InvalidArgument, format!("bad request: {e}"));
            }
        };
        let id = request.id;
        if !ALLOWED.contains(&request.method.as_str()) {
            return failure(
                id,
                Code::PermissionDenied,
                format!("method {} is not available over Bluetooth", request.method),
            );
        }
        // The link opens a stream with `open` and never asks for one here,
        // where its body would be waited for whole and never end.
        if stream_method(&request.method).is_some() {
            return failure(
                id,
                Code::Unimplemented,
                format!("method {} is a stream", request.method),
            );
        }
        match self.call(&request.method, &request.payload).await {
            Ok(payload) => Response {
                id,
                code: 0,
                message: String::new(),
                payload,
                more: false,
            }
            .encode_to_vec(),
            Err(status) => failure(id, status.code(), status.message().to_string()),
        }
    }

    /// Calls `method`, an allowlisted name, with a serialized request
    /// message: its serialized response message, or the call's status.
    #[allow(clippy::result_large_err)]
    async fn call(&self, method: &str, payload: &[u8]) -> Result<Vec<u8>, Status> {
        let (parts, body) = self.start(method, payload).await?.into_parts();
        let collected = body.collect().await?;

        // A call that fails before it has a message answers with its status
        // in the headers alone; otherwise the status follows the message.
        let status = collected
            .trailers()
            .and_then(Status::from_header_map)
            .or_else(|| Status::from_header_map(&parts.headers))
            .ok_or_else(|| Status::internal("response carried no gRPC status"))?;
        if status.code() != Code::Ok {
            return Err(status);
        }

        let data = collected.to_bytes();
        match data.split_first_chunk::<PREFIX_LEN>() {
            Some(([0, len @ ..], message))
                if u32::from_be_bytes(*len) as usize == message.len() =>
            {
                Ok(message.to_vec())
            }
            _ => Err(Status::internal("malformed gRPC message")),
        }
    }

    /// Sends the service the call of `method`, an allowlisted name, with a
    /// serialized request message, through the lock guard: the head of its
    /// answer and the body still to read.
    #[allow(clippy::result_large_err)]
    async fn start(
        &self,
        method: &str,
        payload: &[u8],
    ) -> Result<http::Response<tonic::body::Body>, Status> {
        let mut body = Vec::with_capacity(PREFIX_LEN + payload.len());
        body.push(0);
        body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        body.extend_from_slice(payload);
        let request = http::Request::post(format!("/{SERVICE}/{method}"))
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| Status::internal(e.to_string()))?;

        match self.service.clone().oneshot(request).await {
            Ok(response) => Ok(response),
            Err(never) => match never {},
        }
    }

    /// Opens `stream` with a serialized request message: the messages the
    /// service sends, or the status it refused the call with. Dropping what
    /// this returns drops the service's stream.
    #[allow(clippy::result_large_err)]
    pub(crate) async fn open(
        &self,
        stream: &'static StreamMethod,
        payload: &[u8],
    ) -> Result<Box<dyn Messages>, Status> {
        let (parts, body) = self.start(stream.name, payload).await?.into_parts();
        // A call refused before its stream opens (by the lock, or for a
        // request message that does not parse) answers with its status in
        // the headers and no body.
        let head = Status::from_header_map(&parts.headers);
        match head {
            Some(status) if status.code() != Code::Ok => Err(status),
            head => Ok(Box::new(Body {
                body,
                buffered: Vec::new(),
                head,
            })),
        }
    }
}

/// The body of a server-streaming call, read a message at a time.
struct Body {
    body: tonic::body::Body,
    /// Bytes of the body read and not yet handed on: the service may put
    /// several messages, or part of one, in a piece of the body.
    buffered: Vec<u8>,
    /// The status the headers carried, for a body that ends with none.
    head: Option<Status>,
}

impl Body {
    /// The next whole message of what is buffered, or why there cannot be
    /// one; `None` when more of the body is needed.
    fn buffered_message(&mut self) -> Option<Step> {
        let ([compressed, len @ ..], rest) = self.buffered.split_first_chunk::<PREFIX_LEN>()?;
        if *compressed != 0 {
            return Some(Step::End(Status::internal("compressed gRPC message")));
        }
        let len = u32::from_be_bytes(*len) as usize;
        // Refused from its length alone, before the rest of it is read: it
        // could not be sent, and nothing a call declares sizes a buffer.
        if len > MAX_FRAME {
            return Some(Step::End(Status::resource_exhausted(format!(
                "a stream message of {len} bytes is too large for Bluetooth"
            ))));
        }
        if rest.len() < len {
            return None;
        }
        let message = rest[..len].to_vec();
        self.buffered.drain(..PREFIX_LEN + len);
        Some(Step::Message(message))
    }

    async fn step(&mut self) -> Step {
        loop {
            if let Some(step) = self.buffered_message() {
                return step;
            }
            let frame =
                match self.body.frame().await {
                    Some(Ok(frame)) => frame,
                    Some(Err(status)) => return Step::End(status),
                    // No trailers: the status, if any, was in the headers.
                    None => {
                        return Step::End(self.head.take().unwrap_or_else(|| {
                            Status::internal("response carried no gRPC status")
                        }))
                    }
                };
            match frame.into_data() {
                Ok(data) => self.buffered.extend_from_slice(&data),
                Err(frame) => {
                    if let Ok(trailers) = frame.into_trailers() {
                        return Step::End(Status::from_header_map(&trailers).unwrap_or_else(
                            || Status::internal("response carried no gRPC status"),
                        ));
                    }
                }
            }
        }
    }
}

impl Messages for Body {
    fn next(&mut self) -> BoxFuture<'_, Step> {
        Box::pin(self.step())
    }
}

/// An encoded [`Response`] carrying a failed status.
pub(super) fn failure(id: u32, code: Code, message: String) -> Vec<u8> {
    Response {
        id,
        code: code as u32,
        message,
        payload: Vec::new(),
        more: false,
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::player::lock_policy::{open_on, Surface};
    use crate::proto::player::v1::{
        player_service_server::PlayerService, GetPlaylistRequest, GetPlaylistResponse,
        StatusRequest, StatusResponse, FILE_DESCRIPTOR_SET,
    };

    use super::*;

    const FAILED_PRECONDITION: u32 = Code::FailedPrecondition as u32;
    const PERMISSION_DENIED: u32 = Code::PermissionDenied as u32;
    const INVALID_ARGUMENT: u32 = Code::InvalidArgument as u32;

    async fn setup() -> (Arc<Player>, Dispatcher, Arc<crate::audio::mock::Device>) {
        let player = crate::testutil::player::assets_player()
            .midi("mock-midi-device")
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        // Players start locked; most of these tests exercise the handlers.
        player.set_locked(false);
        let device = player
            .audio_device()
            .expect("audio device")
            .to_mock()
            .expect("mock device");
        (player.clone(), Dispatcher::new(player), device)
    }

    async fn call(dispatcher: &Dispatcher, id: u32, method: &str, payload: Vec<u8>) -> Response {
        let frame = Request {
            id,
            method: method.to_string(),
            payload,
            cancel: false,
        }
        .encode_to_vec();
        Response::decode(dispatcher.handle(&frame).await.as_slice()).expect("a response")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_matches_the_service_called_directly() {
        let (player, dispatcher, _device) = setup().await;

        let response = call(&dispatcher, 1, "Status", StatusRequest {}.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        let through_ble = StatusResponse::decode(response.payload.as_slice()).expect("status");

        let direct = PlayerServer::new(player)
            .status(tonic::Request::new(StatusRequest {}))
            .await
            .expect("status")
            .into_inner();
        assert_eq!(through_ble, direct);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_then_stop() {
        let (_player, dispatcher, device) = setup().await;

        let response = call(&dispatcher, 1, "Play", Vec::new()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        crate::testutil::eventually(|| device.is_playing(), "Song never started playing");

        let response = call(&dispatcher, 2, "Stop", Vec::new()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        crate::testutil::eventually(|| !device.is_playing(), "Song never stopped");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_call_carries_grpcs_code_and_message() {
        let (player, dispatcher, _device) = setup().await;

        let response = call(&dispatcher, 5, "Stop", Vec::new()).await;
        assert_eq!(response.code, FAILED_PRECONDITION);
        assert!(response.payload.is_empty());
        let direct = PlayerServer::new(player)
            .stop(tonic::Request::new(
                crate::proto::player::v1::StopRequest {},
            ))
            .await
            .expect_err("not playing");
        assert_eq!(response.message, direct.message());
        assert!(response.message.contains("not playing"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_method_off_the_allowlist_is_refused_unseen() {
        let (_player, dispatcher, _device) = setup().await;

        for method in [
            "UpdateAudio",
            "NoSuchMethod",
            "play",
            "Play ",
            "Play/../Stop",
            "",
        ] {
            let response = call(&dispatcher, 9, method, Vec::new()).await;
            assert_eq!(response.code, PERMISSION_DENIED, "{method:?}");
            assert_eq!(response.id, 9);
            assert!(response.message.contains(method), "{}", response.message);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn garbage_is_invalid_argument_with_id_zero() {
        let (_player, dispatcher, _device) = setup().await;

        // A length-delimited field that claims more bytes than there are.
        let response = Response::decode(dispatcher.handle(&[0x12, 0xff, 0x01]).await.as_slice())
            .expect("a response");
        assert_eq!(response.id, 0);
        assert_eq!(response.code, INVALID_ARGUMENT);
        assert!(!response.message.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_id_is_echoed() {
        let (_player, dispatcher, _device) = setup().await;

        for id in [0, 1, 7, u32::MAX] {
            let ok = call(&dispatcher, id, "Status", Vec::new()).await;
            assert_eq!((ok.id, ok.code), (id, 0));
            let failed = call(&dispatcher, id, "Stop", Vec::new()).await;
            assert_eq!((failed.id, failed.code), (id, FAILED_PRECONDITION));
            let refused = call(&dispatcher, id, "UpdateAudio", Vec::new()).await;
            assert_eq!((refused.id, refused.code), (id, PERMISSION_DENIED));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_malformed_payload_is_the_services_refusal() {
        let (_player, dispatcher, _device) = setup().await;

        let response = call(&dispatcher, 3, "GetPlaylist", vec![0xff]).await;
        assert_ne!(response.code, 0);
        assert_eq!(response.id, 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reads_a_playlist() {
        let (_player, dispatcher, _device) = setup().await;

        let request = GetPlaylistRequest {
            name: "playlist".to_string(),
        };
        let response = call(&dispatcher, 4, "GetPlaylist", request.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        let playlist = GetPlaylistResponse::decode(response.payload.as_slice()).expect("playlist");
        assert_eq!(playlist.entries.len(), 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn track_levels_and_the_ambient_mode_are_answered() {
        use crate::proto::player::v1::{
            GetTrackGainsRequest, GetTrackGainsResponse, SetAmbientModeRequest,
            SetTrackGainRequest, SetTrackGainResponse, SetTrackMuteRequest,
        };

        let player = crate::testutil::player::assets_player()
            .midi("mock-midi-device")
            .track_mappings(std::collections::HashMap::from([
                ("click".to_string(), vec![1]),
                ("keys".to_string(), vec![2]),
            ]))
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        player.set_locked(false);
        let dispatcher = Dispatcher::new(player.clone());

        let response = call(
            &dispatcher,
            1,
            "GetTrackGains",
            GetTrackGainsRequest {}.encode_to_vec(),
        )
        .await;
        assert_eq!(response.code, 0, "{}", response.message);
        let gains = GetTrackGainsResponse::decode(response.payload.as_slice()).expect("gains");
        assert_eq!(gains.gains.len(), 2);

        let set = SetTrackGainRequest {
            track: "click".to_string(),
            gain_db: -6.0,
        };
        let response = call(&dispatcher, 2, "SetTrackGain", set.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        let applied = SetTrackGainResponse::decode(response.payload.as_slice()).expect("applied");
        assert_eq!(applied.applied_gain_db, -6.0);

        let response = call(
            &dispatcher,
            3,
            "GetTrackGains",
            GetTrackGainsRequest {}.encode_to_vec(),
        )
        .await;
        let gains = GetTrackGainsResponse::decode(response.payload.as_slice()).expect("gains");
        let click = gains
            .gains
            .iter()
            .find(|g| g.track == "click")
            .expect("click");
        assert_eq!((click.gain_db, click.muted), (-6.0, false));

        let mute = SetTrackMuteRequest {
            track: "click".to_string(),
            muted: true,
        };
        let response = call(&dispatcher, 4, "SetTrackMute", mute.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        assert_eq!(player.get_track_mutes().map(|m| m.len()), Some(2));

        // The service's own refusals come through, not the allowlist's.
        let unknown = SetTrackGainRequest {
            track: "nope".to_string(),
            gain_db: 0.0,
        };
        let response = call(&dispatcher, 5, "SetTrackGain", unknown.encode_to_vec()).await;
        assert_ne!(response.code, PERMISSION_DENIED);
        assert_ne!(response.code, 0);

        let bad = SetAmbientModeRequest {
            mode: "sideways".to_string(),
        };
        let response = call(&dispatcher, 6, "SetAmbientMode", bad.encode_to_vec()).await;
        assert_eq!(response.code, INVALID_ARGUMENT);
        assert!(!response.message.is_empty());

        // With no DMX engine a good mode is still answered, not refused.
        let good = SetAmbientModeRequest {
            mode: "auto".to_string(),
        };
        let response = call(&dispatcher, 7, "SetAmbientMode", good.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_player_still_answers_every_allowed_method() {
        let (player, dispatcher, _device) = setup().await;
        player.set_locked(true);

        let response = call(&dispatcher, 1, "Status", Vec::new()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        // Not playing is the handler's answer, not the lock's.
        let response = call(&dispatcher, 2, "Stop", Vec::new()).await;
        assert_eq!(response.code, FAILED_PRECONDITION);
        assert!(
            response.message.contains("not playing"),
            "{}",
            response.message
        );

        // Every other allowed method is left open by the lock policy; the
        // playlist writes are the exception, and are refused.
        let open: HashSet<&str> = open_on(Surface::Grpc).collect();
        for method in ALLOWED {
            if LOCKED_WRITES.contains(method) {
                assert!(!open.contains(method), "{method} is open while locked");
            } else {
                assert!(open.contains(method), "{method} is refused while locked");
            }
        }
        for method in LOCKED_WRITES {
            let response = call(&dispatcher, 3, method, Vec::new()).await;
            assert_eq!(response.code, FAILED_PRECONDITION, "{method}");
            assert_eq!(
                response.message,
                crate::player::LockedError.to_string(),
                "{method}"
            );
        }
        // SetLock is how a phone gets out of it.
        let response = call(&dispatcher, 4, "SetLock", Vec::new()).await;
        assert_eq!(response.code, 0, "{}", response.message);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_phone_unlocks_then_edits_a_playlist_and_locks_again() {
        use crate::proto::player::v1::{
            DeletePlaylistRequest, DeletePlaylistResponse, SavePlaylistRequest, SetLockRequest,
            SetLockResponse,
        };

        let project = crate::testutil::player::playlist_project()
            .await
            .expect("project");
        let player = project.player.clone();
        player.set_locked(true);
        let dispatcher = Dispatcher::new(player.clone());
        let save = SavePlaylistRequest {
            name: "set".to_string(),
            songs: vec!["Song 3".to_string(), "Song 1".to_string()],
            create: true,
            ..Default::default()
        };

        let response = call(&dispatcher, 1, "SavePlaylist", save.encode_to_vec()).await;
        assert_eq!(response.code, FAILED_PRECONDITION);
        assert!(!project.file("set").exists());

        let unlock = SetLockRequest { locked: false };
        let response = call(&dispatcher, 2, "SetLock", unlock.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        let set = SetLockResponse::decode(response.payload.as_slice()).expect("lock");
        assert!(!set.locked);
        assert!(!player.is_locked(), "the flag the web UI reads");

        let response = call(&dispatcher, 3, "SavePlaylist", save.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        let saved = GetPlaylistResponse::decode(response.payload.as_slice()).expect("playlist");
        assert_eq!(saved.name, "set");
        assert_eq!(saved.entries.len(), 2);
        assert!(project.file("set").exists());

        // The service's own refusals are not the allowlist's.
        let again = call(&dispatcher, 4, "SavePlaylist", save.encode_to_vec()).await;
        assert_eq!(again.code, Code::AlreadyExists as u32);

        let delete = DeletePlaylistRequest {
            name: "set".to_string(),
        };
        let response = call(&dispatcher, 5, "DeletePlaylist", delete.encode_to_vec()).await;
        assert_eq!(response.code, 0, "{}", response.message);
        DeletePlaylistResponse::decode(response.payload.as_slice()).expect("empty");
        assert!(!project.file("set").exists());

        let lock = SetLockRequest { locked: true };
        let response = call(&dispatcher, 6, "SetLock", lock.encode_to_vec()).await;
        assert!(
            SetLockResponse::decode(response.payload.as_slice())
                .unwrap()
                .locked
        );
        assert!(player.is_locked());
        let response = call(&dispatcher, 7, "DeletePlaylist", delete.encode_to_vec()).await;
        assert_eq!(response.code, FAILED_PRECONDITION);
    }

    #[test]
    fn every_allowed_method_is_a_method_of_the_service() {
        let set = prost_types::FileDescriptorSet::decode(FILE_DESCRIPTOR_SET).expect("descriptor");
        let service = set
            .file
            .iter()
            .filter(|file| file.package() == "player.v1")
            .flat_map(|file| &file.service)
            .find(|service| service.name() == "PlayerService")
            .expect("PlayerService");
        let methods: HashSet<&str> = service.method.iter().map(|m| m.name()).collect();
        for method in ALLOWED {
            assert!(
                methods.contains(method),
                "{method} is not a PlayerService method"
            );
        }
        let unique: HashSet<&&str> = ALLOWED.iter().collect();
        assert_eq!(unique.len(), ALLOWED.len(), "a method is listed twice");
    }

    /// The stream table and the service agree on which allowed methods are
    /// server streams: one the table missed would be collected whole and
    /// never answered, and a unary one listed would never be answered on
    /// `response`.
    #[test]
    fn the_stream_table_lists_exactly_the_allowed_server_streams() {
        let set = prost_types::FileDescriptorSet::decode(FILE_DESCRIPTOR_SET).expect("descriptor");
        let streaming: HashSet<&str> = set
            .file
            .iter()
            .filter(|file| file.package() == "player.v1")
            .flat_map(|file| &file.service)
            .filter(|service| service.name() == "PlayerService")
            .flat_map(|service| &service.method)
            .inspect(|method| {
                assert!(
                    !method.client_streaming() || !ALLOWED.contains(&method.name()),
                    "{} streams from the client, which Bluetooth cannot carry",
                    method.name()
                )
            })
            .filter(|method| method.server_streaming())
            .map(|method| method.name())
            .collect();
        for method in ALLOWED {
            assert_eq!(
                stream_method(method).is_some(),
                streaming.contains(method),
                "{method}"
            );
        }
        for stream in STREAMS {
            assert!(
                ALLOWED.contains(&stream.name),
                "{} is not allowed",
                stream.name
            );
        }
        assert!(stream_method("WatchStage").is_some());
    }

    /// The service's own encoding of the stage's messages, told apart by
    /// their first byte.
    #[test]
    fn a_stage_frame_is_replaceable_and_a_layout_is_not() {
        use crate::proto::player::v1::{
            stage_update::Update, StageFixture, StageFrame, StageLayout, StageUpdate,
        };

        let replaceable = stream_method("WatchStage").expect("a stream").replaceable;
        let update = |update| StageUpdate { update }.encode_to_vec();

        let frame = update(Some(Update::Frame(StageFrame {
            channels: vec![1, 2, 3],
            layout: 4,
            active_effects: vec!["chase".to_string()],
            ..Default::default()
        })));
        assert!(replaceable(&frame));
        // A frame that says nothing at all is still a frame.
        let empty_frame = update(Some(Update::Frame(StageFrame::default())));
        assert_eq!(empty_frame, [0x12, 0x00]);
        assert!(replaceable(&empty_frame));

        let layout = update(Some(Update::Layout(StageLayout {
            fixtures: vec![StageFixture {
                name: "par".to_string(),
                ..Default::default()
            }],
            id: 1,
            ..Default::default()
        })));
        assert!(!replaceable(&layout));
        let empty_layout = update(Some(Update::Layout(StageLayout::default())));
        assert_eq!(empty_layout, [0x0A, 0x00]);
        assert!(!replaceable(&empty_layout));

        // An update with neither is kept: nothing says it may be dropped.
        assert!(update(None).is_empty());
        assert!(!replaceable(&[]));
    }

    /// The link opens streams; asked for one as a unary call, the
    /// dispatcher says no rather than wait for a body that never ends.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_asked_for_as_a_unary_call_is_refused() {
        let (_player, dispatcher, _device) = setup().await;
        let response = tokio::time::timeout(
            crate::testutil::PATIENCE,
            call(&dispatcher, 6, "WatchStage", Vec::new()),
        )
        .await
        .expect("an answer");
        assert_eq!(
            (response.id, response.code),
            (6, Code::Unimplemented as u32)
        );
    }

    /// The stage stream opens through the lock guard, locked or not, and is
    /// read a message at a time; dropping it is all it takes to end it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_opens_through_the_service_and_is_read_message_by_message() {
        use crate::proto::player::v1::{stage_update::Update, StageUpdate, WatchStageRequest};

        let (player, dispatcher, _device) = setup().await;
        let stream = stream_method("WatchStage").expect("a stream");
        for locked in [false, true] {
            player.set_locked(locked);
            let mut messages = dispatcher
                .open(stream, &WatchStageRequest { max_rate: 0 }.encode_to_vec())
                .await
                .expect("the stream opens");
            let mut next = async || match messages.next().await {
                Step::Message(message) => StageUpdate::decode(message.as_slice())
                    .expect("an update")
                    .update
                    .expect("a layout or a frame"),
                Step::End(status) => panic!("the stream ended: {status}"),
            };
            assert!(matches!(next().await, Update::Layout(_)));
            assert!(matches!(next().await, Update::Frame(_)));
        }

        // A request the service cannot read is its refusal, not a stream.
        let refused = dispatcher
            .open(stream, &[0xff])
            .await
            .err()
            .expect("refused");
        assert_ne!(refused.code(), Code::Ok);
    }

    /// A body as tonic may cut it: several messages in a piece, a message
    /// across pieces, and the status in the trailers.
    #[tokio::test]
    async fn a_stream_body_is_read_whatever_its_pieces() {
        let prefixed = |message: &[u8]| {
            let mut out = vec![0];
            out.extend_from_slice(&(message.len() as u32).to_be_bytes());
            out.extend_from_slice(message);
            out
        };
        let mut bytes = prefixed(b"one");
        bytes.extend(prefixed(b""));
        bytes.extend(prefixed(b"three"));

        // Three messages in one piece, then the trailers.
        let mut trailers = http::HeaderMap::new();
        trailers.insert("grpc-status", "5".parse().unwrap());
        trailers.insert("grpc-message", "gone".parse().unwrap());
        let whole = Full::new(Bytes::from(bytes.clone()))
            .map_err(|never| match never {})
            .with_trailers(async move { Some(Ok::<_, Status>(trailers)) });
        let mut body = Body {
            body: tonic::body::Body::new(whole),
            buffered: Vec::new(),
            head: None,
        };
        let mut read = Vec::new();
        let status = loop {
            match body.step().await {
                Step::Message(message) => read.push(message),
                Step::End(status) => break status,
            }
        };
        assert_eq!(read, [b"one".to_vec(), b"".to_vec(), b"three".to_vec()]);
        assert_eq!((status.code(), status.message()), (Code::NotFound, "gone"));

        // The same bytes a few at a time: a message is whole or not there.
        let mut body = Body {
            body: tonic::body::Body::empty(),
            buffered: Vec::new(),
            head: None,
        };
        let mut read = Vec::new();
        for piece in bytes.chunks(3) {
            body.buffered.extend_from_slice(piece);
            while let Some(step) = body.buffered_message() {
                match step {
                    Step::Message(message) => read.push(message),
                    Step::End(status) => panic!("ended: {status}"),
                }
            }
        }
        assert_eq!(read, [b"one".to_vec(), b"".to_vec(), b"three".to_vec()]);
        assert!(body.buffered.is_empty());

        // A body that ends with no trailers has the status of its headers,
        // and one with none at all is an error, not a stream that ended well.
        body.head = Some(Status::ok(""));
        assert!(matches!(body.step().await, Step::End(status) if status.code() == Code::Ok));
        assert!(matches!(body.step().await, Step::End(status) if status.code() == Code::Internal));

        // A message that could never be sent is refused from its length.
        let mut huge = vec![0];
        huge.extend_from_slice(&(MAX_FRAME as u32 + 1).to_be_bytes());
        body.buffered = huge;
        assert!(matches!(
            body.buffered_message(),
            Some(Step::End(status)) if status.code() == Code::ResourceExhausted
        ));
        // Nor is a compressed one read: none was asked for.
        body.buffered = vec![1, 0, 0, 0, 0];
        assert!(matches!(
            body.buffered_message(),
            Some(Step::End(status)) if status.code() == Code::Internal
        ));
    }
}
