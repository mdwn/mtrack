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
use std::{error::Error, io, net::SocketAddr, sync::Arc};

use tokio::task::JoinHandle;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{info, span, Level};

use crate::{
    config,
    player::Player,
    proto::player::v1::{
        player_service_server::{PlayerService, PlayerServiceServer},
        AddProfileRequest, Cue, GetActiveEffectsRequest, GetActiveEffectsResponse,
        GetConfigRequest, GetConfigResponse, GetCuesRequest, GetCuesResponse, NextRequest,
        NextResponse, PlayFromRequest, PlayRequest, PlayResponse, PreviousRequest,
        PreviousResponse, RemoveProfileRequest, StatusRequest, StatusResponse, StopRequest,
        StopResponse, StopSamplesRequest, StopSamplesResponse, SwitchToPlaylistRequest,
        SwitchToPlaylistResponse, UpdateAudioRequest, UpdateConfigResponse,
        UpdateControllersRequest, UpdateDmxRequest, UpdateMidiRequest, UpdateProfileRequest,
        FILE_DESCRIPTOR_SET,
    },
};

/// A controller that controls a player using gRPC.
pub struct Driver {
    /// The player.
    player: Arc<Player>,
    /// The socket address to host the gRPC server on.
    addr: SocketAddr,
}

impl Driver {
    pub fn new(
        config: config::GrpcController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port()).parse()?;

        Ok(Arc::new(Driver { player, addr }))
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let addr = self.addr;
        let player = self.player.clone();

        tokio::spawn(async move {
            let reflection_service = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
                .build_v1()
                .map_err(io::Error::other)?;

            {
                let _enter = span!(Level::INFO, "gRPC Server").entered();
                info!("Starting gRPC server");
            }

            let player_server = match player.config_store() {
                Some(store) => PlayerServer::with_config_store(player, store),
                None => PlayerServer::new(player),
            };

            Server::builder()
                .add_service(reflection_service)
                .add_service(PlayerServiceServer::new(player_server))
                .serve(addr)
                .await
                .map_err(io::Error::other)
        })
    }
}

/// The actual player server.
pub(crate) struct PlayerServer {
    /// The player.
    player: Arc<Player>,
    /// Mutable configuration store for runtime config changes.
    config_store: Option<Arc<crate::config::ConfigStore>>,
}

impl PlayerServer {
    /// Creates a new PlayerServer wrapping the given player.
    pub(crate) fn new(player: Arc<Player>) -> Self {
        Self {
            player,
            config_store: None,
        }
    }

    /// Creates a new PlayerServer with a config store.
    pub(crate) fn with_config_store(
        player: Arc<Player>,
        config_store: Arc<crate::config::ConfigStore>,
    ) -> Self {
        Self {
            player,
            config_store: Some(config_store),
        }
    }

    /// Converts a play/play_from result into a gRPC response.
    #[allow(clippy::result_large_err)]
    fn play_response(
        result: Result<Option<Arc<crate::songs::Song>>, Box<dyn Error>>,
    ) -> Result<Response<PlayResponse>, Status> {
        match result {
            Ok(Some(song)) => Ok(Response::new(PlayResponse {
                song: Some(song.to_proto()?),
            })),
            Ok(None) => Err(Status::failed_precondition("song already playing")),
            Err(e) => Err(Status::failed_precondition(e.to_string())),
        }
    }

    /// Returns a reference to the config store or a NOT_FOUND error.
    #[allow(clippy::result_large_err)]
    fn require_config_store(&self) -> Result<&crate::config::ConfigStore, Status> {
        self.config_store
            .as_deref()
            .ok_or_else(|| Status::unimplemented("config store not available"))
    }
}

/// Converts a ConfigError to a gRPC Status.
fn config_error_to_status(e: config::ConfigError) -> Status {
    match e {
        config::ConfigError::StaleChecksum { .. } => Status::failed_precondition(e.to_string()),
        config::ConfigError::InvalidProfileIndex { .. } => Status::out_of_range(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

/// Converts a ConfigSnapshot to an UpdateConfigResponse by serializing to YAML.
fn snapshot_to_update_response(
    snapshot: config::store::ConfigSnapshot,
) -> Result<Response<UpdateConfigResponse>, Status> {
    let yaml = crate::util::to_yaml_string(&snapshot.config)
        .map_err(|e| Status::internal(format!("serialization error: {}", e)))?;
    Ok(Response::new(UpdateConfigResponse {
        yaml,
        checksum: snapshot.checksum,
    }))
}

#[tonic::async_trait]
impl PlayerService for PlayerServer {
    async fn play(&self, _: Request<PlayRequest>) -> Result<Response<PlayResponse>, Status> {
        Self::play_response(self.player.play().await)
    }

    async fn play_from(
        &self,
        request: Request<PlayFromRequest>,
    ) -> Result<Response<PlayResponse>, Status> {
        let start_time = request
            .into_inner()
            .start_time
            .map(std::time::Duration::try_from)
            .transpose()
            .map_err(|e| Status::invalid_argument(format!("Invalid duration: {}", e)))?
            .unwrap_or(std::time::Duration::ZERO);

        Self::play_response(self.player.play_from(start_time).await)
    }

    async fn previous(
        &self,
        _: Request<PreviousRequest>,
    ) -> Result<Response<PreviousResponse>, Status> {
        let current_song = self
            .player
            .get_playlist()
            .current()
            .ok_or_else(|| Status::failed_precondition("playlist is empty"))?;
        let previous_song = self
            .player
            .prev()
            .await
            .ok_or_else(|| Status::failed_precondition("playlist is empty"))?;

        if current_song.name() == previous_song.name() {
            return Err(Status::failed_precondition(
                "can't move to previous song while playing",
            ));
        }

        Ok(Response::new(PreviousResponse {
            song: Some(previous_song.to_proto()?),
        }))
    }

    async fn next(&self, _: Request<NextRequest>) -> Result<Response<NextResponse>, Status> {
        let current_song = self
            .player
            .get_playlist()
            .current()
            .ok_or_else(|| Status::failed_precondition("playlist is empty"))?;
        let next_song = self
            .player
            .next()
            .await
            .ok_or_else(|| Status::failed_precondition("playlist is empty"))?;

        if current_song.name() == next_song.name() {
            return Err(Status::failed_precondition(
                "can't move to next song while playing",
            ));
        }

        Ok(Response::new(NextResponse {
            song: Some(next_song.to_proto()?),
        }))
    }

    async fn stop(&self, _: Request<StopRequest>) -> Result<Response<StopResponse>, Status> {
        match self.player.stop().await {
            Some(song) => Ok(Response::new(StopResponse {
                song: Some(song.to_proto()?),
            })),
            None => Err(Status::failed_precondition("song not playing")),
        }
    }

    async fn switch_to_playlist(
        &self,
        request: Request<SwitchToPlaylistRequest>,
    ) -> Result<Response<SwitchToPlaylistResponse>, Status> {
        let playlist_name = request.into_inner().playlist_name;
        self.player
            .switch_to_playlist(&playlist_name)
            .await
            .map_err(|e| {
                if e.contains("not found") {
                    Status::not_found(e)
                } else {
                    Status::failed_precondition(e)
                }
            })?;
        Ok(Response::new(SwitchToPlaylistResponse {}))
    }

    async fn status(&self, _: Request<StatusRequest>) -> Result<Response<StatusResponse>, Status> {
        let playlist_name = self.player.get_playlist().name().to_string();

        let elapsed = {
            let elapsed = self.player.elapsed().await;
            match elapsed {
                Ok(play_start_time) => match play_start_time {
                    Some(play_start_time) => match prost_types::Duration::try_from(play_start_time)
                    {
                        Ok(play_start_time) => Some(play_start_time),
                        Err(e) => return Err(Status::from_error(Box::new(e))),
                    },
                    None => None,
                },
                Err(e) => return Err(Status::internal(e.to_string())),
            }
        };

        let current_song = match self.player.get_playlist().current() {
            Some(song) => Some(song.to_proto()?),
            None => None,
        };

        Ok(Response::new(StatusResponse {
            playlist_name: playlist_name.to_string(),
            current_song,
            playing: elapsed.is_some(),
            elapsed,
        }))
    }

    async fn get_cues(
        &self,
        _: Request<GetCuesRequest>,
    ) -> Result<Response<GetCuesResponse>, Status> {
        // Get cues from the player
        let cues = self.player.get_cues();
        let proto_cues: Result<Vec<Cue>, Box<Status>> = cues
            .into_iter()
            .map(|(time, index)| {
                Ok(Cue {
                    time: Some(prost_types::Duration::try_from(time).map_err(|e| {
                        Box::new(Status::internal(format!(
                            "Failed to convert duration: {}",
                            e
                        )))
                    })?),
                    index: index as u32,
                })
            })
            .collect();

        Ok(Response::new(GetCuesResponse {
            cues: proto_cues.map_err(|e| *e)?,
        }))
    }

    async fn get_active_effects(
        &self,
        _: Request<GetActiveEffectsRequest>,
    ) -> Result<Response<GetActiveEffectsResponse>, Status> {
        let active_effects = self
            .player
            .format_active_effects()
            .unwrap_or_else(|| "No DMX engine available".to_string());

        Ok(Response::new(GetActiveEffectsResponse { active_effects }))
    }

    async fn stop_samples(
        &self,
        _: Request<StopSamplesRequest>,
    ) -> Result<Response<StopSamplesResponse>, Status> {
        self.player.stop_samples();
        Ok(Response::new(StopSamplesResponse {}))
    }

    async fn get_config(
        &self,
        _: Request<GetConfigRequest>,
    ) -> Result<Response<GetConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let (yaml, checksum) = store.read_yaml().await.map_err(config_error_to_status)?;
        Ok(Response::new(GetConfigResponse { yaml, checksum }))
    }

    async fn update_audio(
        &self,
        request: Request<UpdateAudioRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let audio: Option<config::Audio> = if req.audio_json.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&req.audio_json)
                    .map_err(|e| Status::invalid_argument(format!("invalid audio JSON: {}", e)))?,
            )
        };
        let snapshot = store
            .update_audio(audio, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }

    async fn update_midi(
        &self,
        request: Request<UpdateMidiRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let midi: Option<config::Midi> = if req.midi_json.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&req.midi_json)
                    .map_err(|e| Status::invalid_argument(format!("invalid MIDI JSON: {}", e)))?,
            )
        };
        let snapshot = store
            .update_midi(midi, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }

    async fn update_dmx(
        &self,
        request: Request<UpdateDmxRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let dmx: Option<config::Dmx> = if req.dmx_json.is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(&req.dmx_json)
                    .map_err(|e| Status::invalid_argument(format!("invalid DMX JSON: {}", e)))?,
            )
        };
        let snapshot = store
            .update_dmx(dmx, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }

    async fn update_controllers(
        &self,
        request: Request<UpdateControllersRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let controllers: Vec<config::Controller> = serde_json::from_str(&req.controllers_json)
            .map_err(|e| Status::invalid_argument(format!("invalid controllers JSON: {}", e)))?;
        let snapshot = store
            .update_controllers(controllers, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }

    async fn add_profile(
        &self,
        request: Request<AddProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let profile: config::Profile = serde_json::from_str(&req.profile_json)
            .map_err(|e| Status::invalid_argument(format!("invalid profile JSON: {}", e)))?;
        let snapshot = store
            .add_profile(profile, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }

    async fn update_profile(
        &self,
        request: Request<UpdateProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let profile: config::Profile = serde_json::from_str(&req.profile_json)
            .map_err(|e| Status::invalid_argument(format!("invalid profile JSON: {}", e)))?;
        let snapshot = store
            .update_profile(req.index as usize, profile, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }

    async fn remove_profile(
        &self,
        request: Request<RemoveProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let snapshot = store
            .remove_profile(req.index as usize, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        snapshot_to_update_response(snapshot)
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashMap,
        error::Error,
        net::{Ipv4Addr, SocketAddr, SocketAddrV4},
        path::Path,
        sync::Arc,
        time::Duration,
    };

    use tokio::net::TcpListener;
    use tonic::transport::Channel;

    use crate::{
        config,
        controller::{grpc::Driver, Driver as _},
        playlist,
        playlist::Playlist,
        proto::player::v1::{
            player_service_client::PlayerServiceClient, NextRequest, PlayRequest, PreviousRequest,
            StatusRequest, StopRequest, SwitchToPlaylistRequest,
        },
        songs,
        testutil::eventually,
    };

    use super::Player;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let pl = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), pl);
        let player = Arc::new(Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);
        player.await_hardware_ready().await;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // Get a random port.
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
        let listener = TcpListener::bind(addr).await?;
        let port = listener.local_addr()?.port();
        drop(listener);

        println!("Using port {} for testing.", port);

        let driver = Driver::new(config::GrpcController::new(port), player.clone())?;
        tokio::spawn(driver.monitor_events());
        let mut client: Option<PlayerServiceClient<Channel>> = None;
        for _ in 0..5 {
            match PlayerServiceClient::connect(format!("http://127.0.0.1:{}", port)).await {
                Ok(connected_client) => client = Some(connected_client),
                Err(e) => {
                    println!("Sleeping for 50ms and trying to connect again. {}", e);
                }
            }

            if client.is_some() {
                break;
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Direct the player.
        let mut client = client.expect("client was none");

        // Verify initial playlist name in status
        let resp = client.status(StatusRequest {}).await?;
        let status = resp.into_inner();
        assert_eq!(
            status.playlist_name, "playlist",
            "Initial playlist name should be 'playlist'"
        );

        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        let resp = client.next(NextRequest {}).await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        let resp = client.previous(PreviousRequest {}).await?;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 1");

        println!("Switch to AllSongs");
        let _ = client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: "all_songs".to_string(),
            })
            .await?;
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        // Verify playlist name changed to "all_songs" in status
        let resp = client.status(StatusRequest {}).await?;
        let status = resp.into_inner();
        assert_eq!(
            status.playlist_name, "all_songs",
            "Playlist name should be 'all_songs' after switching"
        );

        let resp = client.next(NextRequest {}).await?;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 10");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 10");

        let resp = client.next(NextRequest {}).await?;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 2");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 2");

        let resp = client.next(NextRequest {}).await?;
        println!("AllSongs -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        let _ = client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: "playlist".to_string(),
            })
            .await?;
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        // Verify playlist name changed back to "playlist" in status
        let resp = client.status(StatusRequest {}).await?;
        let status = resp.into_inner();
        assert_eq!(
            status.playlist_name, "playlist",
            "Playlist name should be 'playlist' after switching back"
        );

        let resp = client.next(NextRequest {}).await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        let resp = client.play(PlayRequest {}).await?;
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().unwrap().name()
            )
            .as_str(),
        );
        let resp = client.status(StatusRequest {}).await?;
        let status = resp.into_inner();
        assert_eq!(
            status.playlist_name, "playlist",
            "Playlist name should still be 'playlist' after playback"
        );
        assert_eq!(status.current_song.unwrap().name, "Song 5");

        // Play a song and cancel it.
        let resp = client.play(PlayRequest {}).await?;
        println!("Play Song 5.");
        eventually(|| device.is_playing(), "Song never started playing");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 5");

        let resp = client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 5");

        // Player should not have moved to the next song.
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 5");

        Ok(())
    }

    /// Helper to set up a player + gRPC client pair on an ephemeral port.
    async fn setup_grpc() -> Result<
        (
            Arc<Player>,
            PlayerServiceClient<Channel>,
            Arc<crate::audio::mock::Device>,
        ),
        Box<dyn Error>,
    > {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let pl = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), pl);
        let player = Arc::new(Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);
        player.await_hardware_ready().await;
        let device = player.audio_device().expect("audio device").to_mock()?;

        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
        let listener = TcpListener::bind(addr).await?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let driver = Driver::new(config::GrpcController::new(port), player.clone())?;
        tokio::spawn(driver.monitor_events());

        let mut client = None;
        for _ in 0..10 {
            match PlayerServiceClient::connect(format!("http://127.0.0.1:{}", port)).await {
                Ok(c) => {
                    client = Some(c);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        Ok((
            player,
            client.expect("gRPC client connection failed"),
            device,
        ))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_stop_when_not_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, _device) = setup_grpc().await?;

        let result = client.stop(StopRequest {}).await;
        assert!(result.is_err(), "stop() when idle should fail");
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().contains("not playing"));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_play_already_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, device) = setup_grpc().await?;

        // First play should succeed.
        let resp = client.play(PlayRequest {}).await?;
        assert!(resp.into_inner().song.is_some());
        eventually(|| device.is_playing(), "Song never started playing");

        // Second play while already playing should fail.
        let result = client.play(PlayRequest {}).await;
        assert!(
            result.is_err(),
            "play() while playing should be a precondition failure"
        );

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_switch_to_invalid_playlist() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, _device) = setup_grpc().await?;

        let result = client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: "nonexistent".to_string(),
            })
            .await;
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::NotFound);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_status_shows_current_song() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, _device) = setup_grpc().await?;

        let resp = client.status(StatusRequest {}).await?;
        let status = resp.into_inner();
        assert!(!status.playing);
        assert!(status.elapsed.is_none());
        assert!(status.current_song.is_some());
        assert_eq!(status.current_song.unwrap().name, "Song 1");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_stop_samples() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::StopSamplesRequest;

        let (_player, mut client, _device) = setup_grpc().await?;

        // stop_samples should always succeed, even with no active samples.
        client.stop_samples(StopSamplesRequest {}).await?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_play_from() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::PlayFromRequest;

        let (_player, mut client, _device) = setup_grpc().await?;

        // play_from with a start time should succeed and return the song.
        let start = prost_types::Duration {
            seconds: 0,
            nanos: 500_000_000,
        };
        let resp = client
            .play_from(PlayFromRequest {
                start_time: Some(start),
            })
            .await?;
        let song = resp.into_inner().song;
        assert!(song.is_some());
        assert_eq!(song.unwrap().name, "Song 1");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_play_from_no_time() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::PlayFromRequest;

        let (_player, mut client, device) = setup_grpc().await?;

        // play_from with no start_time should default to beginning.
        let resp = client
            .play_from(PlayFromRequest { start_time: None })
            .await?;
        assert!(resp.into_inner().song.is_some());
        eventually(|| device.is_playing(), "Song never started playing");

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_next_while_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, device) = setup_grpc().await?;

        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // next while playing should fail.
        let result = client.next(NextRequest {}).await;
        assert!(result.is_err(), "next() while playing should fail");
        assert_eq!(result.unwrap_err().code(), tonic::Code::FailedPrecondition);

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_previous_while_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, device) = setup_grpc().await?;

        // Move to Song 3 first so previous has somewhere to go.
        client.next(NextRequest {}).await?;

        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // previous while playing should fail.
        let result = client.previous(PreviousRequest {}).await;
        assert!(result.is_err(), "previous() while playing should fail");
        assert_eq!(result.unwrap_err().code(), tonic::Code::FailedPrecondition);

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_get_active_effects() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::GetActiveEffectsRequest;

        let (_player, mut client, _device) = setup_grpc().await?;

        // With no DMX engine, should return a "no engine" message.
        let resp = client
            .get_active_effects(GetActiveEffectsRequest {})
            .await?;
        let effects = resp.into_inner().active_effects;
        assert!(!effects.is_empty());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_get_cues() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::GetCuesRequest;

        let (_player, mut client, _device) = setup_grpc().await?;

        // Without a lighting timeline, should return an empty cues list.
        let resp = client.get_cues(GetCuesRequest {}).await?;
        let cues = resp.into_inner().cues;
        assert!(cues.is_empty());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_status_while_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, device) = setup_grpc().await?;

        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let resp = client.status(StatusRequest {}).await?;
        let status = resp.into_inner();
        assert!(status.playing);
        assert!(status.elapsed.is_some());

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_switch_to_playlist() -> Result<(), Box<dyn Error>> {
        let (player, mut client, _device) = setup_grpc().await?;

        // Start on "playlist", switch to "all_songs" successfully.
        assert_eq!(player.get_playlist().name(), "playlist");
        let resp = client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: "all_songs".to_string(),
            })
            .await;
        assert!(
            resp.is_ok(),
            "switch_to_playlist with valid name should succeed"
        );
        assert_eq!(player.get_playlist().name(), "all_songs");

        // Switch back to "playlist".
        client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: "playlist".to_string(),
            })
            .await?;
        assert_eq!(player.get_playlist().name(), "playlist");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_get_config_no_store() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::GetConfigRequest;

        let (_player, mut client, _device) = setup_grpc().await?;

        // The test setup creates a PlayerServer without a config store,
        // so get_config should return UNIMPLEMENTED.
        let result = client.get_config(GetConfigRequest {}).await;
        assert!(
            result.is_err(),
            "get_config without config store should fail"
        );
        let status = result.unwrap_err();
        assert_eq!(
            status.code(),
            tonic::Code::Unimplemented,
            "Expected UNIMPLEMENTED when no config store is available"
        );
        assert!(status.message().contains("config store not available"));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_update_audio_no_store() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::UpdateAudioRequest;

        let (_player, mut client, _device) = setup_grpc().await?;

        // Without a config store, update_audio should return UNIMPLEMENTED.
        let result = client
            .update_audio(UpdateAudioRequest {
                audio_json: String::new(),
                expected_checksum: String::new(),
            })
            .await;
        assert!(
            result.is_err(),
            "update_audio without config store should fail"
        );
        let status = result.unwrap_err();
        assert_eq!(
            status.code(),
            tonic::Code::Unimplemented,
            "Expected UNIMPLEMENTED when no config store is available"
        );
        assert!(status.message().contains("config store not available"));

        Ok(())
    }
}
