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
//! serves.

use std::sync::Arc;

use http_body_util::{BodyExt, Full};
use prost::Message;
use tonic::{codegen::Bytes, Code, Status};
use tower::ServiceExt;

use crate::player::Player;
use crate::proto::ble::v1::{Request, Response};

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
];

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
        match self.call(&request.method, &request.payload).await {
            Ok(payload) => Response {
                id,
                code: 0,
                message: String::new(),
                payload,
            }
            .encode_to_vec(),
            Err(status) => failure(id, status.code(), status.message().to_string()),
        }
    }

    /// Calls `method`, an allowlisted name, with a serialized request
    /// message: its serialized response message, or the call's status.
    #[allow(clippy::result_large_err)]
    async fn call(&self, method: &str, payload: &[u8]) -> Result<Vec<u8>, Status> {
        let mut body = Vec::with_capacity(PREFIX_LEN + payload.len());
        body.push(0);
        body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        body.extend_from_slice(payload);
        let request = http::Request::post(format!("/{SERVICE}/{method}"))
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| Status::internal(e.to_string()))?;

        let response = match self.service.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let (parts, body) = response.into_parts();
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
}

/// An encoded [`Response`] carrying a failed status.
pub(super) fn failure(id: u32, code: Code, message: String) -> Vec<u8> {
    Response {
        id,
        code: code as u32,
        message,
        payload: Vec::new(),
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

        let open: HashSet<&str> = open_on(Surface::Grpc).collect();
        for method in ALLOWED {
            assert!(open.contains(method), "{method} is refused while locked");
        }
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
}
