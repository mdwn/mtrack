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
use tracing::info;

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

/// A controller that controls a player using MIDI.
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
    ) -> Result<Driver, Box<dyn Error>> {
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port()).parse()?;

        Ok(Driver { player, addr })
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let addr = self.addr;
        let player = self.player.clone();

        tokio::spawn(async move {
            let player = player.clone();
            let reflection_service = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
                .build_v1()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            info!("Starting gRPC server");

            Server::builder()
                .add_service(reflection_service)
                .add_service(PlayerServiceServer::new(PlayerServer {
                    player: player.clone(),
                }))
                .serve(addr)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
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
            let play_start_time = self.player.get_play_start_time().await;
            match play_start_time {
                Some(play_start_time) => match play_start_time.elapsed() {
                    Ok(play_start_time) => match prost_types::Duration::try_from(play_start_time) {
                        Ok(play_start_time) => Some(play_start_time),
                        Err(e) => return Err(Status::from_error(Box::new(e))),
                    },
                    Err(e) => return Err(Status::from_error(Box::new(e))),
                },
                None => None,
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
