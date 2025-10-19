// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
        NextRequest, NextResponse, PlayRequest, PlayResponse, PreviousRequest, PreviousResponse,
        StatusRequest, StatusResponse, StopRequest, StopResponse, SwitchToPlaylistRequest,
        SwitchToPlaylistResponse, FILE_DESCRIPTOR_SET,
    },
};

// Playlist name constants.
const PLAYLIST_NAME: &str = "playlist";
const ALL_SONGS_NAME: &str = "all_songs";

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
            let span = span!(Level::INFO, "gRPC Server");
            let _enter = span.enter();

            let player = player.clone();
            let reflection_service = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
                .build_v1()
                .map_err(io::Error::other)?;

            info!("Starting gRPC server");

            Server::builder()
                .add_service(reflection_service)
                .add_service(PlayerServiceServer::new(PlayerServer {
                    player: player.clone(),
                }))
                .serve(addr)
                .await
                .map_err(io::Error::other)
        })
    }
}

/// The actual player server.
struct PlayerServer {
    /// The player.
    player: Arc<Player>,
}

#[tonic::async_trait]
impl PlayerService for PlayerServer {
    async fn play(&self, _: Request<PlayRequest>) -> Result<Response<PlayResponse>, Status> {
        match self.player.play().await {
            Some(song) => Ok(Response::new(PlayResponse {
                song: Some(song.to_proto()?),
            })),
            None => Err(Status::failed_precondition("song already playing")),
        }
    }

    async fn previous(
        &self,
        _: Request<PreviousRequest>,
    ) -> Result<Response<PreviousResponse>, Status> {
        let current_song = self.player.get_playlist().current();
        let previous_song = self.player.prev().await;

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
        let current_song = self.player.get_playlist().current();
        let next_song = self.player.next().await;

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
        if playlist_name == PLAYLIST_NAME {
            self.player.switch_to_playlist().await;
            return Ok(Response::new(SwitchToPlaylistResponse {}));
        }
        if playlist_name == ALL_SONGS_NAME {
            self.player.switch_to_all_songs().await;
            return Ok(Response::new(SwitchToPlaylistResponse {}));
        }

        Err(Status::unimplemented(format!(
            "only {} and {} are supported for now",
            ALL_SONGS_NAME, PLAYLIST_NAME
        )))
    }

    async fn status(&self, _: Request<StatusRequest>) -> Result<Response<StatusResponse>, Status> {
        let all_songs_playlist = self.player.get_all_songs_playlist();
        let playlist_name = if Arc::ptr_eq(&all_songs_playlist, &self.player.get_playlist()) {
            PLAYLIST_NAME
        } else {
            ALL_SONGS_NAME
        };

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

        Ok(Response::new(StatusResponse {
            playlist_name: playlist_name.to_string(),
            current_song: Some(self.player.get_playlist().current().to_proto()?),
            playing: elapsed.is_some(),
            elapsed,
        }))
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
        controller::{
            grpc::{Driver, ALL_SONGS_NAME, PLAYLIST_NAME},
            Driver as _,
        },
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
        let player = Arc::new(Player::new(
            songs.clone(),
            Playlist::new(
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                vec![config::Controller::Keyboard],
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                None, // lighting configuration
                HashMap::new(),
                "assets/songs",
            ),
        )?);
        let binding = player.audio_device();
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
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        let resp = client.next(NextRequest {}).await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        let resp = client.previous(PreviousRequest {}).await?;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 1");

        println!("Switch to AllSongs");
        let _ = client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: ALL_SONGS_NAME.to_string(),
            })
            .await?;
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        let resp = client.next(NextRequest {}).await?;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().name(), "Song 10");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 10");

        let resp = client.next(NextRequest {}).await?;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().name(), "Song 2");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 2");

        let resp = client.next(NextRequest {}).await?;
        println!("AllSongs -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        let _ = client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: PLAYLIST_NAME.to_string(),
            })
            .await?;
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        let resp = client.next(NextRequest {}).await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        let resp = client.play(PlayRequest {}).await?;
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 3");

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().name() == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().name()
            )
            .as_str(),
        );
        let resp = client.status(StatusRequest {}).await?;
        assert_eq!(resp.into_inner().current_song.unwrap().name, "Song 5");

        // Play a song and cancel it.
        let resp = client.play(PlayRequest {}).await?;
        println!("Play Song 5.");
        eventually(|| device.is_playing(), "Song never started playing");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 5");

        let resp = client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        assert_eq!(resp.into_inner().song.unwrap().name, "Song 5");

        // Player should not have moved to the next song.
        assert_eq!(player.get_playlist().current().name(), "Song 5");

        Ok(())
    }
}
