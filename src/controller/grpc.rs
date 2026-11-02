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
        AddProfileRequest, AmbientStatus, AudioHealth, AutoPlayStatus, CancelAutoPlayRequest,
        CancelAutoPlayResponse, Cue, GetActiveEffectsRequest, GetActiveEffectsResponse,
        GetConfigRequest, GetConfigResponse, GetCuesRequest, GetCuesResponse, GetTrackGainsRequest,
        GetTrackGainsResponse, LoopSectionRequest, LoopSectionResponse, NextRequest, NextResponse,
        PlayFromRequest, PlayRequest, PlayResponse, PlaySongFromRequest, PreviousRequest,
        PreviousResponse, RemoveProfileRequest, SectionAckRequest, SectionAckResponse, SeekRequest,
        SeekResponse, SeekToSectionRequest, SetAmbientModeRequest, SetAmbientModeResponse,
        SetTrackGainRequest, SetTrackGainResponse, SetTrackMuteRequest, SetTrackMuteResponse,
        StatusRequest, StatusResponse, StopRequest, StopResponse, StopSamplesRequest,
        StopSamplesResponse, StopSectionLoopRequest, StopSectionLoopResponse,
        SwitchToPlaylistRequest, SwitchToPlaylistResponse, TrackGain, UpdateAudioRequest,
        UpdateConfigResponse, UpdateControllersRequest, UpdateDmxRequest, UpdateMidiRequest,
        UpdateProfileRequest, FILE_DESCRIPTOR_SET,
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
    ///
    /// `Ok(None)` means the player declined to start, and it declines for more
    /// than one reason: a song already playing, an empty playlist, or breaking
    /// out of a looping song to advance. Reporting "song already playing" for
    /// all three sent one investigation a long way in the wrong direction —
    /// a song that failed to load leaves an empty playlist, and the transport
    /// error said nothing about that. The state is cheap to read, so read it.
    #[allow(clippy::result_large_err)]
    async fn play_response(
        &self,
        // Takes the error already flattened: `Box<dyn Error>` is not `Send`,
        // and as a parameter it is captured by this future whatever the body
        // does with it.
        result: Result<Option<Arc<crate::songs::Song>>, String>,
    ) -> Result<Response<PlayResponse>, Status> {
        let song = match result {
            Ok(Some(song)) => song,
            Ok(None) => {
                return Err(Status::failed_precondition(self.declined_reason().await));
            }
            Err(e) => return Err(Status::failed_precondition(e)),
        };
        Ok(Response::new(PlayResponse {
            song: Some(song.to_proto()?),
        }))
    }

    /// Applies a saved config change to the running hardware, then builds
    /// the response: the new checksum, and whether the change is applied.
    ///
    /// The web UI's mutation endpoints have always reloaded; these had not, so
    /// a client could change the audio device, receive a fresh checksum, and
    /// find the running hardware unchanged until the process restarted. The
    /// reply says whether it was applied, deferred until the player is idle,
    /// or not applied because the reload failed, as every surface's does (see
    /// [`crate::player::ConfigApply`]).
    ///
    /// Safe to do from inside a gRPC call because the reload leaves each
    /// controller running whose config has not changed -- otherwise this
    /// would tear down the very transport carrying the response.
    #[allow(clippy::result_large_err)]
    async fn reload_and_respond(
        &self,
        snapshot: config::store::ConfigSnapshot,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let outcome = self.player.apply_config_change().await;
        snapshot_to_update_response(snapshot, outcome)
    }

    /// Why the player declined to start, determined from its current state
    /// rather than assumed.
    async fn declined_reason(&self) -> String {
        if self.player.is_playing().await {
            return "song already playing".to_string();
        }
        if self.player.get_playlist().songs().is_empty() {
            return "the playlist is empty — no songs loaded, which usually \
                    means none passed validation at startup (check the log)"
                .to_string();
        }
        "the player did not start a song".to_string()
    }

    /// Refuses a config write while the player is locked, the way the web
    /// UI's lock middleware does. Playback controls never call this.
    #[allow(clippy::result_large_err)]
    fn require_unlocked(&self) -> Result<(), Status> {
        self.player
            .require_unlocked()
            .map_err(|e| Status::failed_precondition(e.to_string()))
    }

    /// Returns a reference to the config store or a NOT_FOUND error.
    #[allow(clippy::result_large_err)]
    fn require_config_store(&self) -> Result<&crate::config::ConfigStore, Status> {
        self.config_store
            .as_deref()
            .ok_or_else(|| Status::unimplemented("config store not available"))
    }
}

/// A playback position a request sent, the start of the song when it sent
/// none. A negative one is turned down, as every surface turns it down.
#[allow(clippy::result_large_err)]
fn position_arg(position: Option<prost_types::Duration>) -> Result<std::time::Duration, Status> {
    position
        .map(std::time::Duration::try_from)
        .transpose()
        .map_err(|e| Status::invalid_argument(format!("Invalid duration: {}", e)))
        .map(Option::unwrap_or_default)
}

/// Ambient light's mode and state, as the player's status read it: absent
/// without a DMX engine, or with a read that did not answer in time.
fn ambient_to_proto(
    ambient: Option<crate::player::DmxRead<crate::player::AmbientStatus>>,
) -> Option<AmbientStatus> {
    match ambient? {
        crate::player::DmxRead::Read(ambient) => Some(AmbientStatus {
            mode: ambient.report.mode.as_str().to_string(),
            state: ambient.report.state.to_string(),
            show: ambient.report.show.map(|show| show.display().to_string()),
            source: ambient.source.map(|source| source.as_str().to_string()),
        }),
        _ => None,
    }
}

/// Converts a ConfigError to a gRPC Status.
fn config_error_to_status(e: config::ConfigError) -> Status {
    match e {
        config::ConfigError::StaleChecksum { .. } => Status::failed_precondition(e.to_string()),
        config::ConfigError::InvalidProfileIndex { .. } => Status::out_of_range(e.to_string()),
        config::ConfigError::Validation(_) => Status::invalid_argument(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}

/// Converts a ConfigSnapshot to an UpdateConfigResponse by serializing to
/// YAML, saying what became of the change on the running hardware.
#[allow(clippy::result_large_err)]
fn snapshot_to_update_response(
    snapshot: config::store::ConfigSnapshot,
    outcome: crate::player::ConfigApply,
) -> Result<Response<UpdateConfigResponse>, Status> {
    let yaml = crate::util::to_yaml_string(&snapshot.config)
        .map_err(|e| Status::internal(format!("serialization error: {}", e)))?;
    Ok(Response::new(UpdateConfigResponse {
        yaml,
        checksum: snapshot.checksum,
        applied: outcome.applied(),
        message: outcome.message().unwrap_or_default(),
    }))
}

#[tonic::async_trait]
impl PlayerService for PlayerServer {
    async fn play(&self, _: Request<PlayRequest>) -> Result<Response<PlayResponse>, Status> {
        self.play_response(self.player.play().await.map_err(|e| e.to_string()))
            .await
    }

    async fn play_from(
        &self,
        request: Request<PlayFromRequest>,
    ) -> Result<Response<PlayResponse>, Status> {
        let start_time = position_arg(request.into_inner().start_time)?;

        self.play_response(
            self.player
                .play_from(start_time)
                .await
                .map_err(|e| e.to_string()),
        )
        .await
    }

    async fn play_song_from(
        &self,
        request: Request<PlaySongFromRequest>,
    ) -> Result<Response<PlayResponse>, Status> {
        let req = request.into_inner();
        let start_time = position_arg(req.start_time)?;

        self.play_response(
            self.player
                .play_song_from(&req.song_name, start_time)
                .await
                .map_err(|e| e.to_string()),
        )
        .await
    }

    async fn seek(&self, request: Request<SeekRequest>) -> Result<Response<SeekResponse>, Status> {
        let position = position_arg(request.into_inner().position)?;

        self.player
            .seek_to(position)
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Response::new(SeekResponse {}))
    }

    async fn seek_to_section(
        &self,
        request: Request<SeekToSectionRequest>,
    ) -> Result<Response<SeekResponse>, Status> {
        let req = request.into_inner();
        self.player
            .seek_to_section(&req.section_name)
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Response::new(SeekResponse {}))
    }

    async fn previous(
        &self,
        _: Request<PreviousRequest>,
    ) -> Result<Response<PreviousResponse>, Status> {
        if self.player.is_playing().await && !self.player.is_current_song_looping() {
            return Err(Status::failed_precondition("can't navigate while playing"));
        }
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
            return Err(Status::out_of_range(
                "already at the beginning of the playlist",
            ));
        }

        Ok(Response::new(PreviousResponse {
            song: Some(previous_song.to_proto()?),
        }))
    }

    async fn next(&self, _: Request<NextRequest>) -> Result<Response<NextResponse>, Status> {
        if self.player.is_playing().await && !self.player.is_current_song_looping() {
            return Err(Status::failed_precondition("can't navigate while playing"));
        }
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
            return Err(Status::out_of_range("already at the end of the playlist"));
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
        // The player's one status, its DMX reads bounded: a wedged effects
        // loop leaves the ambient light out rather than hanging the call.
        let status = self.player.status_snapshot().await;

        let elapsed = match status.elapsed {
            Ok(Some(elapsed)) => Some(
                prost_types::Duration::try_from(elapsed)
                    .map_err(|e| Status::from_error(Box::new(e)))?,
            ),
            Ok(None) => None,
            Err(e) => return Err(Status::internal(e)),
        };

        let current_song = match &status.current_song {
            Some(song) => Some(song.to_proto()?),
            None => None,
        };

        // Hardware health travels with playback state so a remote monitor can
        // tell "silent because nothing is playing" from "silent and it should
        // not be" without a second call.
        let audio_health = self
            .player
            .hardware_status()
            .audio_output
            .map(|h| AudioHealth {
                status: h.status.as_str().to_string(),
                callback_alive: h.callback_alive,
                writing_signal: h.writing_signal,
                recoveries: h.recoveries,
                underruns: h.underruns,
                last_error: h.last_error,
            });

        let auto_play = match status.auto_play {
            Some((song, starts_in)) => Some(AutoPlayStatus {
                song,
                starts_in: Some(
                    prost_types::Duration::try_from(starts_in)
                        .map_err(|e| Status::internal(e.to_string()))?,
                ),
            }),
            None => None,
        };

        let ambient = ambient_to_proto(status.ambient);

        Ok(Response::new(StatusResponse {
            playlist_name: status.playlist.name().to_string(),
            current_song,
            playing: status.playing,
            elapsed,
            audio_health,
            ambient,
            auto_play,
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
        self.require_unlocked()?;
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
        self.reload_and_respond(snapshot).await
    }

    async fn update_midi(
        &self,
        request: Request<UpdateMidiRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.require_unlocked()?;
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
        self.reload_and_respond(snapshot).await
    }

    async fn update_dmx(
        &self,
        request: Request<UpdateDmxRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.require_unlocked()?;
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
        self.reload_and_respond(snapshot).await
    }

    async fn update_controllers(
        &self,
        request: Request<UpdateControllersRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.require_unlocked()?;
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let controllers: Vec<config::Controller> = serde_json::from_str(&req.controllers_json)
            .map_err(|e| Status::invalid_argument(format!("invalid controllers JSON: {}", e)))?;
        let snapshot = store
            .update_controllers(controllers, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        self.reload_and_respond(snapshot).await
    }

    async fn add_profile(
        &self,
        request: Request<AddProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.require_unlocked()?;
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let profile: config::Profile = serde_json::from_str(&req.profile_json)
            .map_err(|e| Status::invalid_argument(format!("invalid profile JSON: {}", e)))?;
        let snapshot = store
            .add_profile(profile, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        self.reload_and_respond(snapshot).await
    }

    async fn update_profile(
        &self,
        request: Request<UpdateProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.require_unlocked()?;
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let body: serde_json::Value = serde_json::from_str(&req.profile_json)
            .map_err(|e| Status::invalid_argument(format!("invalid profile JSON: {}", e)))?;
        // Sections the request leaves out keep their current values (#523).
        let profile = store
            .profile_from_update(req.index as usize, body)
            .await
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let snapshot = store
            .update_profile(req.index as usize, profile, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        self.reload_and_respond(snapshot).await
    }

    async fn remove_profile(
        &self,
        request: Request<RemoveProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        self.require_unlocked()?;
        let store = self.require_config_store()?;
        let req = request.into_inner();
        let snapshot = store
            .remove_profile(req.index as usize, &req.expected_checksum)
            .await
            .map_err(config_error_to_status)?;
        self.reload_and_respond(snapshot).await
    }

    async fn loop_section(
        &self,
        request: Request<LoopSectionRequest>,
    ) -> Result<Response<LoopSectionResponse>, Status> {
        let section_name = request.into_inner().section_name;
        self.player
            .loop_section(&section_name)
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Response::new(LoopSectionResponse {}))
    }

    async fn stop_section_loop(
        &self,
        _request: Request<StopSectionLoopRequest>,
    ) -> Result<Response<StopSectionLoopResponse>, Status> {
        self.player.stop_section_loop();
        Ok(Response::new(StopSectionLoopResponse {}))
    }

    async fn section_ack(
        &self,
        _request: Request<SectionAckRequest>,
    ) -> Result<Response<SectionAckResponse>, Status> {
        self.player
            .section_ack()
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        Ok(Response::new(SectionAckResponse {}))
    }

    async fn set_track_gain(
        &self,
        request: Request<SetTrackGainRequest>,
    ) -> Result<Response<SetTrackGainResponse>, Status> {
        let req = request.into_inner();
        let applied = self
            .player
            .set_track_gain(&req.track, req.gain_db as f32)
            .map_err(|e| match e {
                crate::player::TrackGainError::NoAudioProfile => {
                    Status::failed_precondition(e.to_string())
                }
                crate::player::TrackGainError::NonFinite(_)
                | crate::player::TrackGainError::UnknownTrack(_) => {
                    Status::invalid_argument(e.to_string())
                }
            })?;
        Ok(Response::new(SetTrackGainResponse {
            applied_gain_db: applied as f64,
        }))
    }

    async fn set_track_mute(
        &self,
        request: Request<SetTrackMuteRequest>,
    ) -> Result<Response<SetTrackMuteResponse>, Status> {
        let req = request.into_inner();
        let muted = self
            .player
            .set_track_mute(&req.track, req.muted)
            .map_err(|e| match e {
                crate::player::TrackGainError::NoAudioProfile => {
                    Status::failed_precondition(e.to_string())
                }
                crate::player::TrackGainError::NonFinite(_)
                | crate::player::TrackGainError::UnknownTrack(_) => {
                    Status::invalid_argument(e.to_string())
                }
            })?;
        Ok(Response::new(SetTrackMuteResponse { muted }))
    }

    async fn set_ambient_mode(
        &self,
        request: Request<SetAmbientModeRequest>,
    ) -> Result<Response<SetAmbientModeResponse>, Status> {
        let name = request.into_inner().mode;
        let mode = crate::dmx::engine::AmbientMode::parse(&name).ok_or_else(|| {
            Status::invalid_argument(format!(
                "unknown ambient mode '{name}': use auto, on or off"
            ))
        })?;
        let player = self.player.clone();
        tokio::task::spawn_blocking(move || player.set_ambient_mode(mode))
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(SetAmbientModeResponse {
            ambient: ambient_to_proto(self.player.ambient_status().await),
        }))
    }

    async fn cancel_auto_play(
        &self,
        _: Request<CancelAutoPlayRequest>,
    ) -> Result<Response<CancelAutoPlayResponse>, Status> {
        Ok(Response::new(CancelAutoPlayResponse {
            cancelled: self.player.cancel_waiting_auto_play(),
        }))
    }

    async fn get_track_gains(
        &self,
        _: Request<GetTrackGainsRequest>,
    ) -> Result<Response<GetTrackGainsResponse>, Status> {
        let mutes: std::collections::HashMap<String, bool> = self
            .player
            .get_track_mutes()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let gains = self
            .player
            .get_track_gains()
            .unwrap_or_default()
            .into_iter()
            .map(|(track, gain_db)| TrackGain {
                muted: mutes.get(&track).copied().unwrap_or(false),
                track,
                gain_db: gain_db as f64,
            })
            .collect();
        Ok(Response::new(GetTrackGainsResponse { gains }))
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
            player_service_client::PlayerServiceClient, LoopSectionRequest, NextRequest,
            PlayRequest, PreviousRequest, StatusRequest, StopRequest, StopSectionLoopRequest,
            SwitchToPlaylistRequest,
        },
        songs,
        testutil::eventually,
    };

    use super::Player;

    /// An empty playlist is not "a song is already playing".
    ///
    /// `Player::play` answers `Ok(None)` for three different situations and
    /// says which for none of them. Reporting the most common one as fact sent
    /// a hardware investigation a long way in the wrong direction: a song that
    /// failed to load leaves the playlist empty, and the transport error
    /// mentioned nothing of the sort.
    #[tokio::test(flavor = "multi_thread")]
    async fn play_on_an_empty_playlist_says_so() -> Result<(), Box<dyn Error>> {
        let empty = tempfile::tempdir()?;
        let songs = songs::get_all_songs(empty.path())?;
        let mut playlists = HashMap::new();
        playlists.insert("all_songs".to_string(), playlist::from_songs(songs)?);
        let player = Player::new(
            playlists,
            "all_songs".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                empty.path().to_str().expect("path"),
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        let server = super::PlayerServer::new(player);
        let status = crate::proto::player::v1::player_service_server::PlayerService::play(
            &server,
            tonic::Request::new(PlayRequest {}),
        )
        .await
        .expect_err("an empty playlist cannot play");

        assert!(
            status.message().contains("playlist is empty"),
            "expected the empty playlist to be named, got: {}",
            status.message()
        );
        Ok(())
    }

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
        let player = Player::new(
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
        )?;
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
        setup_grpc_with_mappings(HashMap::new()).await
    }

    async fn setup_grpc_with_mappings(
        track_mappings: HashMap<String, Vec<u16>>,
    ) -> Result<
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
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                track_mappings,
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        // Players start locked; these tests exercise the handlers, not the lock.
        player.set_locked(false);
        let device = player.audio_device().expect("audio device").to_mock()?;
        let client = serve_grpc(player.clone()).await?;
        Ok((player, client, device))
    }

    /// Serves `player` over gRPC on an ephemeral port; a client for it.
    async fn serve_grpc(
        player: Arc<Player>,
    ) -> Result<PlayerServiceClient<Channel>, Box<dyn Error>> {
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
        Ok(client.expect("gRPC client connection failed"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_track_gains() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::{GetTrackGainsRequest, SetTrackGainRequest};

        let mappings = HashMap::from([
            ("click".to_string(), vec![1]),
            ("keys".to_string(), vec![2]),
        ]);
        let (_player, mut client, _device) = setup_grpc_with_mappings(mappings).await?;

        // All tracks start at unity.
        let gains = client
            .get_track_gains(GetTrackGainsRequest {})
            .await?
            .into_inner()
            .gains;
        assert_eq!(gains.len(), 2);
        assert!(gains.iter().all(|g| g.gain_db == 0.0));

        // Set returns the applied value.
        let resp = client
            .set_track_gain(SetTrackGainRequest {
                track: "click".to_string(),
                gain_db: -6.0,
            })
            .await?
            .into_inner();
        assert_eq!(resp.applied_gain_db, -6.0);

        // Out-of-range values are clamped.
        let resp = client
            .set_track_gain(SetTrackGainRequest {
                track: "keys".to_string(),
                gain_db: 20.0,
            })
            .await?
            .into_inner();
        assert_eq!(resp.applied_gain_db, 12.0);

        // Get reflects the changes.
        let gains = client
            .get_track_gains(GetTrackGainsRequest {})
            .await?
            .into_inner()
            .gains;
        let by_name: HashMap<String, f64> =
            gains.into_iter().map(|g| (g.track, g.gain_db)).collect();
        assert_eq!(by_name["click"], -6.0);
        assert_eq!(by_name["keys"], 12.0);

        // Unknown track is an invalid argument.
        let status = client
            .set_track_gain(SetTrackGainRequest {
                track: "nope".to_string(),
                gain_db: 0.0,
            })
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);

        // Non-finite gain is an invalid argument.
        let status = client
            .set_track_gain(SetTrackGainRequest {
                track: "click".to_string(),
                gain_db: f64::NAN,
            })
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);

        Ok(())
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
            nanos: 250_000_000,
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

        // play_start_time is set asynchronously after clock.start(), so
        // elapsed may briefly be None even though playing is true. Poll.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let resp = client.status(StatusRequest {}).await?;
            let status = resp.into_inner();
            assert!(status.playing);
            if status.elapsed.is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "elapsed should have a value while playing"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

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

    /// A config change deferred under a song answered as one applied: the
    /// reply says whether the hardware reload ran, and why not when not.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_config_update_says_whether_it_was_applied() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::{player_service_server::PlayerService, UpdateAudioRequest};

        let (player, _client, device) = setup_grpc().await?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: assets/songs\n")?;
        let store = Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path,
        ));
        player.set_config_store(store.clone());
        let server = super::PlayerServer::with_config_store(player.clone(), store.clone());
        let update = |checksum: String| UpdateAudioRequest {
            audio_json: String::new(),
            expected_checksum: checksum,
        };

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        let (_, checksum) = store.read_yaml().await?;
        let deferred = server
            .update_audio(tonic::Request::new(update(checksum)))
            .await?
            .into_inner();
        assert!(
            !deferred.applied,
            "a change under a song said it was applied"
        );
        assert!(deferred.message.contains("idle"), "{}", deferred.message);

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        player.await_hardware_ready().await;
        let (_, checksum) = store.read_yaml().await?;
        let applied = server
            .update_audio(tonic::Request::new(update(checksum)))
            .await?
            .into_inner();
        assert!(applied.applied);
        assert!(applied.message.is_empty(), "{}", applied.message);
        Ok(())
    }

    /// A reload that fails after a save answers with the save, its new
    /// checksum included, and says it was not applied and why, as the web
    /// UI's and MCP's replies do. It used to answer failed_precondition,
    /// leaving the client with the old checksum of a config already saved.
    #[test]
    fn a_config_update_whose_reload_fails_says_so_with_the_save() {
        let snapshot = config::store::ConfigSnapshot {
            config: config::Player::default(),
            yaml: "songs: songs\n".to_string(),
            checksum: "abc".to_string(),
        };
        let reply = super::snapshot_to_update_response(
            snapshot,
            crate::player::ConfigApply::Failed("no store".into()),
        )
        .unwrap()
        .into_inner();
        assert_eq!(reply.checksum, "abc");
        assert!(!reply.applied);
        assert!(reply.message.contains("no store"), "{}", reply.message);
    }

    /// A config change the next start would refuse, a second MIDI
    /// controller, was saved and applied, and mtrack then failed to start.
    /// It is refused as an invalid argument, and the file is left as it was.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_config_update_that_would_not_load_is_an_invalid_argument(
    ) -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::{
            player_service_server::PlayerService, AddProfileRequest, UpdateControllersRequest,
        };

        let (player, _client, _device) = setup_grpc().await?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: assets/songs\n")?;
        let store = Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path.clone(),
        ));
        player.set_config_store(store.clone());
        let server = super::PlayerServer::with_config_store(player.clone(), store.clone());
        let midi = config::Controller::Midi(config::MidiController::new(
            config::midi::note_on(16, 0, 127),
            config::midi::note_on(16, 1, 127),
            config::midi::note_on(16, 2, 127),
            config::midi::note_on(16, 3, 127),
            config::midi::note_on(16, 4, 127),
            config::midi::note_on(16, 5, 127),
        ));
        let controllers = serde_json::to_string(&[&midi, &midi])?;
        let before = std::fs::read_to_string(&path)?;
        let (_, checksum) = store.read_yaml().await?;

        let refused = server
            .update_controllers(tonic::Request::new(UpdateControllersRequest {
                controllers_json: controllers.clone(),
                expected_checksum: checksum.clone(),
            }))
            .await
            .expect_err("two MIDI controllers were accepted");
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);
        assert!(
            refused.message().contains("only one can watch"),
            "{refused}"
        );

        let profile = format!(r#"{{"hostname": "two-midi", "controllers": {controllers}}}"#);
        let refused = server
            .add_profile(tonic::Request::new(AddProfileRequest {
                profile_json: profile,
                expected_checksum: checksum.clone(),
            }))
            .await
            .expect_err("a profile with two MIDI controllers was accepted");
        assert_eq!(refused.code(), tonic::Code::InvalidArgument);

        assert_eq!(std::fs::read_to_string(&path)?, before);
        assert_eq!(store.read_yaml().await?.1, checksum);
        config::Player::deserialize(&path)?;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_loop_section_not_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, _device) = setup_grpc().await?;

        let result = client
            .loop_section(LoopSectionRequest {
                section_name: "verse".to_string(),
            })
            .await;
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().contains("no song is playing"));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_loop_section_not_found() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, device) = setup_grpc().await?;

        // Start playing first.
        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Test songs have no sections, so any name should fail.
        let result = client
            .loop_section(LoopSectionRequest {
                section_name: "nonexistent".to_string(),
            })
            .await;
        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().contains("not found"));

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_stop_section_loop_succeeds() -> Result<(), Box<dyn Error>> {
        let (player, mut client, _device) = setup_grpc().await?;

        // stop_section_loop should always succeed (even with no active loop).
        client.stop_section_loop(StopSectionLoopRequest {}).await?;

        // Verify state was cleared.
        assert!(player.active_section().is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_ambient_mode() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::SetAmbientModeRequest;

        // A DMX engine with no ambient show: on says it has none to play.
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let pl = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert("all_songs".to_string(), playlist::from_songs(songs)?);
        playlists.insert("playlist".to_string(), pl);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                Some(config::Dmx::new(
                    None,
                    None,
                    Some(9090),
                    vec![config::Universe::new(1, "test".to_string())],
                    None,
                )),
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let mut client = serve_grpc(player.clone()).await?;

        let ambient = client.status(StatusRequest {}).await?.into_inner().ambient;
        let ambient = ambient.expect("ambient with a DMX engine");
        assert_eq!(
            (ambient.mode.as_str(), ambient.state.as_str()),
            ("auto", "off")
        );

        let resp = client
            .set_ambient_mode(SetAmbientModeRequest {
                mode: "on".to_string(),
            })
            .await?
            .into_inner();
        let ambient = resp.ambient.expect("ambient");
        assert_eq!(
            (ambient.mode.as_str(), ambient.state.as_str()),
            ("on", "unavailable")
        );
        assert_eq!(player.ambient_mode(), crate::dmx::engine::AmbientMode::On);

        let status = client
            .set_ambient_mode(SetAmbientModeRequest {
                mode: "dim".to_string(),
            })
            .await
            .expect_err("an unknown mode");
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_auto_play_status_and_cancel() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::CancelAutoPlayRequest;

        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let yaml = "songs:\n- Song 1\n- song: Song 3\n  auto_play: 2s\n";
        let playlist_config: config::Playlist = ::config::Config::builder()
            .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
            .build()?
            .try_deserialize()?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert(
            "playlist".to_string(),
            Playlist::new("playlist", &playlist_config, songs)?,
        );
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let mut client = serve_grpc(player.clone()).await?;

        let status = client.status(StatusRequest {}).await?.into_inner();
        assert!(status.auto_play.is_none(), "nothing waiting");
        assert!(status.ambient.is_none(), "no DMX engine");

        client.next(NextRequest {}).await?;
        let auto_play = client
            .status(StatusRequest {})
            .await?
            .into_inner()
            .auto_play
            .expect("waiting");
        assert_eq!(auto_play.song, "Song 3");
        let starts_in = auto_play.starts_in.expect("starts_in");
        assert!(starts_in.seconds <= 2, "{starts_in:?}");

        let resp = client.cancel_auto_play(CancelAutoPlayRequest {}).await?;
        assert!(resp.into_inner().cancelled);
        let status = client.status(StatusRequest {}).await?.into_inner();
        assert!(status.auto_play.is_none(), "called off");
        assert!(!status.playing);
        assert_eq!(status.current_song.expect("song").name, "Song 3");

        let resp = client.cancel_auto_play(CancelAutoPlayRequest {}).await?;
        assert!(!resp.into_inner().cancelled, "nothing left to call off");
        Ok(())
    }

    /// The RPCs that write the config, refused while the player is locked.
    const LOCKED_RPCS: &[&str] = &[
        "UpdateAudio",
        "UpdateMidi",
        "UpdateDmx",
        "UpdateControllers",
        "AddProfile",
        "UpdateProfile",
        "RemoveProfile",
    ];

    /// The RPCs that stay open while locked: reads and playback controls.
    const OPEN_RPCS: &[&str] = &[
        "Play",
        "PlayFrom",
        "PlaySongFrom",
        "Seek",
        "SeekToSection",
        "Previous",
        "Next",
        "Stop",
        "SwitchToPlaylist",
        "Status",
        "GetCues",
        "GetActiveEffects",
        "StopSamples",
        "GetConfig",
        "LoopSection",
        "StopSectionLoop",
        "SectionAck",
        "SetTrackGain",
        "SetTrackMute",
        "SetAmbientMode",
        "CancelAutoPlay",
        "GetTrackGains",
    ];

    /// Every RPC in the proto is either refused or allowed while locked, so
    /// a new one cannot slip past the lock unclassified.
    #[test]
    fn every_rpc_is_classified_for_the_lock() {
        let proto = include_str!("../proto/player/v1/player.proto");
        let rpcs: Vec<&str> = proto
            .lines()
            .filter_map(|l| l.trim().strip_prefix("rpc "))
            .filter_map(|l| l.split('(').next())
            .map(str::trim)
            .collect();
        assert!(!rpcs.is_empty());
        for rpc in &rpcs {
            assert!(
                LOCKED_RPCS.contains(rpc) ^ OPEN_RPCS.contains(rpc),
                "RPC {rpc} must be in exactly one of LOCKED_RPCS or OPEN_RPCS; \
                 a config or file write belongs in LOCKED_RPCS and must call \
                 require_unlocked()"
            );
        }
        for rpc in LOCKED_RPCS.iter().chain(OPEN_RPCS) {
            assert!(rpcs.contains(rpc), "{rpc} is not in the proto");
        }
    }

    /// A locked player refuses every config write as FAILED_PRECONDITION and
    /// leaves the file alone, while reads and playback controls still work.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_player_refuses_config_writes() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::{
            player_service_server::PlayerService, AddProfileRequest, GetConfigRequest,
            RemoveProfileRequest, StopSamplesRequest, UpdateAudioRequest, UpdateControllersRequest,
            UpdateDmxRequest, UpdateMidiRequest, UpdateProfileRequest,
        };
        use tonic::{Request, Status};

        let (player, _client, device) = setup_grpc().await?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: assets/songs\n")?;
        let store = Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path.clone(),
        ));
        player.set_config_store(store.clone());
        let server = super::PlayerServer::with_config_store(player.clone(), store.clone());
        let before = std::fs::read_to_string(&path)?;
        let (_, checksum) = store.read_yaml().await?;
        player.set_locked(true);

        let r = |c: &str| c.to_string();
        let results: Vec<(&str, Result<(), Status>)> = vec![
            (
                "UpdateAudio",
                server
                    .update_audio(Request::new(UpdateAudioRequest {
                        audio_json: String::new(),
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateMidi",
                server
                    .update_midi(Request::new(UpdateMidiRequest {
                        midi_json: String::new(),
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateDmx",
                server
                    .update_dmx(Request::new(UpdateDmxRequest {
                        dmx_json: String::new(),
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateControllers",
                server
                    .update_controllers(Request::new(UpdateControllersRequest {
                        controllers_json: "[]".to_string(),
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
            (
                "AddProfile",
                server
                    .add_profile(Request::new(AddProfileRequest {
                        profile_json: r#"{"hostname": "x"}"#.to_string(),
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateProfile",
                server
                    .update_profile(Request::new(UpdateProfileRequest {
                        index: 0,
                        profile_json: "{}".to_string(),
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
            (
                "RemoveProfile",
                server
                    .remove_profile(Request::new(RemoveProfileRequest {
                        index: 0,
                        expected_checksum: r(&checksum),
                    }))
                    .await
                    .map(|_| ()),
            ),
        ];
        assert_eq!(results.len(), LOCKED_RPCS.len());
        for (rpc, result) in results {
            let status = result.expect_err(rpc);
            assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{rpc}");
            assert!(status.message().contains("locked"), "{rpc}: {status}");
        }
        assert_eq!(std::fs::read_to_string(&path)?, before);

        // Reads and playback controls stay open.
        server.get_config(Request::new(GetConfigRequest {})).await?;
        server.status(Request::new(StatusRequest {})).await?;
        server.play(Request::new(PlayRequest {})).await?;
        eventually(|| device.is_playing(), "Song never started playing");
        server.stop(Request::new(StopRequest {})).await?;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        server.next(Request::new(NextRequest {})).await?;
        server
            .stop_samples(Request::new(StopSamplesRequest {}))
            .await?;
        server
            .stop_section_loop(Request::new(StopSectionLoopRequest {}))
            .await?;
        assert!(player.is_locked());
        Ok(())
    }
}
