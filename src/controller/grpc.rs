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
mod playlists;
mod stage;

use crate::config::store::ConfigSection;
use std::{error::Error, io, net::SocketAddr, sync::Arc};

use tokio::task::JoinHandle;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{info, span, Level};

use crate::util::FaultKind;

use crate::{
    config,
    player::{Player, PLAY_DECLINED_KEY},
    proto::player::v1::{
        player_service_server::{PlayerService, PlayerServiceServer},
        AddProfileRequest, AmbientStatus, AudioHealth, AutoPlayStatus, CancelAutoPlayRequest,
        CancelAutoPlayResponse, Cue, DeletePlaylistRequest, DeletePlaylistResponse,
        GetActiveEffectsRequest, GetActiveEffectsResponse, GetConfigRequest, GetConfigResponse,
        GetCuesRequest, GetCuesResponse, GetPlaylistRequest, GetPlaylistResponse,
        GetTrackGainsRequest, GetTrackGainsResponse, ListPlaylistsRequest, ListPlaylistsResponse,
        LoopSectionRequest, LoopSectionResponse, NavigateToSongRequest, NavigateToSongResponse,
        NextRequest, NextResponse, PlayFromRequest, PlayRequest, PlayResponse, PlaySongFromRequest,
        PlaylistEntry, PreviousRequest, PreviousResponse, RemoveProfileRequest,
        SavePlaylistRequest, SectionAckRequest, SectionAckResponse, SeekRequest, SeekResponse,
        SeekToSectionRequest, SetAmbientModeRequest, SetAmbientModeResponse, SetLockRequest,
        SetLockResponse, SetTrackGainRequest, SetTrackGainResponse, SetTrackMuteRequest,
        SetTrackMuteResponse, Song, StageFrame, StageLayout, StageUpdate, StatusRequest,
        StatusResponse, StopRequest, StopResponse, StopSamplesRequest, StopSamplesResponse,
        StopSectionLoopRequest, StopSectionLoopResponse, SwitchToPlaylistRequest,
        SwitchToPlaylistResponse, TrackGain, UpdateAudioRequest, UpdateConfigResponse,
        UpdateControllersRequest, UpdateDmxRequest, UpdateMidiRequest, UpdateProfileRequest,
        WatchStageRequest, FILE_DESCRIPTOR_SET,
    },
};

/// A controller that controls a player using gRPC.
pub struct Driver {
    /// The player.
    player: Arc<Player>,
    /// The socket address to host the gRPC server on.
    addr: SocketAddr,
    /// Where the server is listening, once it is.
    bound: super::BoundAddr,
}

impl Driver {
    pub fn new(
        config: config::GrpcController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port()).parse()?;

        Ok(Arc::new(Driver {
            player,
            addr,
            bound: super::BoundAddr::new(),
        }))
    }
}

impl super::Driver for Driver {
    fn bound_addr(&self) -> Option<super::BoundAddr> {
        Some(self.bound.clone())
    }

    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let addr = self.addr;
        let player = self.player.clone();
        let bound = self.bound.clone();

        tokio::spawn(async move {
            let reflection_service = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
                .build_v1()
                .map_err(io::Error::other)?;

            // Bound here rather than by `serve`, to know the address: the
            // port the OS picked, for a port of 0. TCP_NODELAY as `serve` sets.
            let listener = tokio::net::TcpListener::bind(addr).await?;
            let addr = listener.local_addr()?;
            {
                let _enter = span!(Level::INFO, "gRPC Server").entered();
                info!("Starting gRPC server on {addr}");
            }
            bound.set(addr);

            Server::builder()
                .add_service(reflection_service)
                .add_service(player_service(player))
                .serve_with_incoming(
                    tonic::transport::server::TcpIncoming::from(listener).with_nodelay(Some(true)),
                )
                .await
                .map_err(io::Error::other)
        })
    }
}

/// The position in the song a status reports, as the wire carries it: the
/// player's one reading, shared by every surface that serves a status.
#[allow(clippy::result_large_err)]
pub(crate) fn elapsed_to_proto(
    elapsed: Result<Option<std::time::Duration>, String>,
) -> Result<Option<prost_types::Duration>, Status> {
    match elapsed {
        Ok(Some(elapsed)) => Ok(Some(
            prost_types::Duration::try_from(elapsed)
                .map_err(|e| Status::from_error(Box::new(e)))?,
        )),
        Ok(None) => Ok(None),
        Err(e) => Err(Status::internal(e)),
    }
}

/// The song waiting to start by itself and how long until it does, as the
/// wire carries it.
#[allow(clippy::result_large_err)]
pub(crate) fn auto_play_to_proto(
    auto_play: Option<(String, std::time::Duration)>,
) -> Result<Option<AutoPlayStatus>, Status> {
    auto_play
        .map(|(song, starts_in)| {
            Ok(AutoPlayStatus {
                song,
                starts_in: Some(
                    prost_types::Duration::try_from(starts_in)
                        .map_err(|e| Status::internal(e.to_string()))?,
                ),
            })
        })
        .transpose()
}

/// The player's gRPC service, behind the lock: what both the gRPC
/// controller and the web UI's gRPC-Web route serve.
pub(crate) fn player_service(player: Arc<Player>) -> LockGuard<PlayerServiceServer<PlayerServer>> {
    let server = match player.config_store() {
        Some(store) => PlayerServer::with_config_store(player.clone(), store),
        None => PlayerServer::new(player.clone()),
    };
    LockGuard {
        inner: PlayerServiceServer::new(server),
        player,
    }
}

/// Refuses every RPC the lock policy does not leave open while the player
/// is locked, before it reaches its handler, as MCP's `call_tool` gate and
/// the web UI's lock middleware do. A new RPC is refused while locked until
/// [`crate::player::lock_policy::OPEN_WHILE_LOCKED`] lists it.
#[derive(Clone)]
pub(crate) struct LockGuard<S> {
    inner: S,
    player: Arc<Player>,
}

impl<S> tonic::server::NamedService for LockGuard<S>
where
    S: tonic::server::NamedService,
{
    const NAME: &'static str = S::NAME;
}

impl<S, B> tower::Service<http::Request<B>> for LockGuard<S>
where
    S: tower::Service<http::Request<B>, Response = http::Response<tonic::body::Body>>
        + tonic::server::NamedService,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = futures_util::future::BoxFuture<'static, Result<S::Response, S::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        use futures_util::FutureExt;
        // `/<service name>/<RPC>`: the RPC is what the lock policy names.
        let path = request.uri().path();
        let rpc = path
            .strip_prefix('/')
            .and_then(|path| path.strip_prefix(S::NAME))
            .and_then(|path| path.strip_prefix('/'))
            .unwrap_or(path);
        if let Err(e) = self
            .player
            .check_lock(crate::player::lock_policy::Surface::Grpc, rpc)
        {
            let refused = Status::failed_precondition(e.to_string()).into_http();
            return futures_util::future::ready(Ok(refused)).boxed();
        }
        self.inner.call(request).boxed()
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
                let declined = self.player.play_declined().await;
                let mut status = Status::failed_precondition(declined.to_string());
                // The reason by key too, so a client tells a song in the
                // way from the other refusals without matching words.
                if let Ok(key) = declined.key().parse() {
                    status.metadata_mut().insert(PLAY_DECLINED_KEY, key);
                }
                return Err(status);
            }
            Err(e) => return Err(Status::failed_precondition(e)),
        };
        Ok(Response::new(PlayResponse {
            song: Some(song.to_proto()?),
        }))
    }

    /// Saves `section` from its JSON (empty for `null`), as MCP's tools
    /// and the store parse it, then applies it.
    async fn update_section(
        &self,
        section: ConfigSection,
        json: &str,
        checksum: &str,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let store = self.require_config_store()?;
        let body = if json.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(json).map_err(|e| {
                Status::invalid_argument(format!("invalid {} JSON: {e}", section.name()))
            })?
        };
        let snapshot = store
            .update_section(section, body, checksum)
            .await
            .map_err(config_error_to_status)?;
        self.reload_and_respond(snapshot).await
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

    /// Returns a reference to the config store or a NOT_FOUND error.
    #[allow(clippy::result_large_err)]
    fn require_config_store(&self) -> Result<&crate::config::ConfigStore, Status> {
        self.config_store
            .as_deref()
            .ok_or_else(|| Status::unimplemented("config store not available"))
    }
}

/// One open stage stream: what it last sent, and what it waits on. It is
/// the stream's whole state, so dropping the stream (the client went away)
/// ends everything at once.
struct StageWatch {
    player: Arc<Player>,
    /// The sampler's snapshots; `None` until the player has a state sender.
    states: Option<tokio::sync::watch::Receiver<Arc<crate::state::StateSnapshot>>>,
    /// The player's layout revision, and the one the layout was built at.
    revisions: tokio::sync::watch::Receiver<u64>,
    revision: u64,
    /// The least time between two sends; zero for the sampler's own rate.
    interval: std::time::Duration,
    /// The layout the client holds; `None` before the first. Its id is how
    /// many this stream has sent, so a frame can say which it is read
    /// against.
    layout: Option<StageLayout>,
    /// The last frame sent, so one that says the same is not sent again.
    frame: Option<StageFrame>,
    /// The frame that follows a layout just sent.
    queued: Option<StageFrame>,
    sent_at: Option<tokio::time::Instant>,
    done: bool,
}

impl StageWatch {
    fn new(player: Arc<Player>, max_rate: u32) -> Self {
        Self {
            states: player.state_rx(),
            revisions: player.stage_layout_rx(),
            revision: 0,
            interval: match max_rate {
                0 => std::time::Duration::ZERO,
                rate => std::time::Duration::from_secs(1) / rate,
            },
            layout: None,
            frame: None,
            queued: None,
            sent_at: None,
            done: false,
            player,
        }
    }

    /// The stream itself.
    fn into_stream(self) -> futures_util::stream::BoxStream<'static, Result<StageUpdate, Status>> {
        use futures_util::StreamExt;
        futures_util::stream::unfold(self, |mut watch| async move {
            watch.next().await.map(|update| (update, watch))
        })
        .boxed()
    }

    /// The next message: the layout and a frame at once on open, then a
    /// frame for each snapshot that says something new, with a layout before
    /// it when the venue changed or the snapshot no longer fits the one sent.
    async fn next(&mut self) -> Option<Result<StageUpdate, Status>> {
        use crate::proto::player::v1::stage_update::Update;
        if self.done {
            return None;
        }
        if let Some(frame) = self.queued.take() {
            return Some(Ok(StageUpdate {
                update: Some(Update::Frame(frame)),
            }));
        }
        loop {
            // The first pass sends what there is now; the rest wait for a
            // change, and then for the caller's interval to pass, so the
            // snapshot read below is the newest of however many arrived.
            if self.layout.is_some() {
                if !self.changed().await {
                    return None;
                }
                if let Some(sent_at) = self.sent_at {
                    tokio::time::sleep_until(sent_at + self.interval).await;
                }
            }

            // The revision is read before the layout is built, so a venue
            // change during the build is seen as one after it.
            let revision = *self.revisions.borrow_and_update();
            let snapshot = self.snapshot();
            let current = self
                .layout
                .as_ref()
                .filter(|layout| revision == self.revision && stage::fits(&snapshot, layout));
            if let Some(layout) = current {
                let frame = stage::stage_frame(&snapshot, layout);
                if self.frame.as_ref() == Some(&frame) {
                    continue;
                }
                self.frame = Some(frame.clone());
                self.sent_at = Some(tokio::time::Instant::now());
                return Some(Ok(StageUpdate {
                    update: Some(Update::Frame(frame)),
                }));
            }

            let mut layout = match self.build_layout(snapshot.clone()).await {
                Ok(layout) => layout,
                Err(status) => {
                    self.done = true;
                    return Some(Err(status));
                }
            };
            // Never 0, which a client reads as no id at all.
            let sent = self.layout.as_ref().map_or(0, |layout| layout.id);
            layout.id = sent.wrapping_add(1).max(1);
            // A frame always follows a layout, whatever the last one said:
            // it is read against this layout, not that one.
            let frame = stage::stage_frame(&snapshot, &layout);
            self.revision = revision;
            self.frame = Some(frame.clone());
            self.queued = Some(frame);
            self.layout = Some(layout.clone());
            self.sent_at = Some(tokio::time::Instant::now());
            return Some(Ok(StageUpdate {
                update: Some(Update::Layout(layout)),
            }));
        }
    }

    /// Waits for a new snapshot or a new layout revision. False when the
    /// player is gone and neither will come.
    async fn changed(&mut self) -> bool {
        let states = &mut self.states;
        let snapshot = async {
            match states {
                Some(states) => states.changed().await.is_ok(),
                None => std::future::pending().await,
            }
        };
        let (state_closed, alive) = tokio::select! {
            ok = snapshot => (!ok, true),
            revision = self.revisions.changed() => (false, revision.is_ok()),
        };
        if state_closed {
            // The player was given another state sender: follow that one.
            self.states = self.player.state_rx();
        }
        alive
    }

    /// The newest snapshot, marked seen. A player with no state sender (no
    /// sampler was ever wired) has an empty stage; one given a sender later
    /// is picked up at the next layout revision.
    fn snapshot(&mut self) -> Arc<crate::state::StateSnapshot> {
        if self.states.is_none() {
            self.states = self.player.state_rx();
        }
        match self.states.as_mut() {
            Some(states) => states.borrow_and_update().clone(),
            None => Arc::default(),
        }
    }

    /// The layout for `snapshot`, from the lighting system the player has
    /// now: read again each time, since a hardware reload replaces the
    /// engine. On the blocking pool, because the lighting system's lock is
    /// held across disk reads while a venue reloads.
    async fn build_layout(
        &self,
        snapshot: Arc<crate::state::StateSnapshot>,
    ) -> Result<StageLayout, Status> {
        let player = self.player.clone();
        tokio::task::spawn_blocking(move || {
            let handles = player.broadcast_handles();
            let metadata = crate::webui::state::stage_metadata(
                handles
                    .as_ref()
                    .and_then(|handles| handles.lighting_system.as_ref()),
            );
            stage::stage_layout(&metadata, &snapshot)
        })
        .await
        .map_err(|e| Status::internal(format!("the stage layout could not be read: {e}")))
    }
}

/// The song a navigation landed on, as the player answers it: the one it
/// moved to, or the one it stays on (playing, or at the list's end), as
/// MCP answers too. Only an empty playlist is a failure.
#[allow(clippy::result_large_err)]
fn navigated(song: Option<Arc<crate::songs::Song>>) -> Result<Song, Status> {
    song.ok_or_else(|| Status::failed_precondition("playlist is empty"))?
        .to_proto()
        .map_err(Status::from)
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
/// without a DMX engine; `available: false` with a read that did not answer.
fn ambient_to_proto(
    ambient: Option<crate::player::DmxRead<crate::player::AmbientStatus>>,
) -> Option<AmbientStatus> {
    use crate::webui::wire::messages::AmbientState;
    Some(match ambient?.view() {
        AmbientState::Read(ambient) => AmbientStatus {
            mode: ambient.mode.as_str().to_string(),
            state: ambient.state,
            show: ambient.show,
            source: ambient.source.map(|source| source.as_str().to_string()),
            available: None,
            reason: None,
        },
        AmbientState::Unread(unread) => AmbientStatus {
            mode: String::new(),
            state: String::new(),
            show: None,
            source: None,
            available: Some(unread.available),
            reason: Some(unread.reason),
        },
    })
}

/// A fault as a gRPC status: the one table from [`FaultKind`] to a code.
fn fault_status(kind: FaultKind, message: String) -> Status {
    match kind {
        FaultKind::NotFound => Status::not_found(message),
        FaultKind::Precondition => Status::failed_precondition(message),
        FaultKind::Invalid => Status::invalid_argument(message),
        FaultKind::Internal => Status::internal(message),
    }
}

/// Converts a ConfigError to a gRPC Status.
fn config_error_to_status(e: config::ConfigError) -> Status {
    fault_status(e.kind(), e.to_string())
}

/// Converts a ConfigSnapshot to an UpdateConfigResponse by serializing to
/// YAML, saying what became of the change on the running hardware.
#[allow(clippy::result_large_err)]
fn snapshot_to_update_response(
    snapshot: config::store::ConfigSnapshot,
    outcome: crate::player::ConfigChange,
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
        Ok(Response::new(PreviousResponse {
            song: Some(navigated(self.player.prev().await)?),
        }))
    }

    async fn next(&self, _: Request<NextRequest>) -> Result<Response<NextResponse>, Status> {
        Ok(Response::new(NextResponse {
            song: Some(navigated(self.player.next().await)?),
        }))
    }

    async fn navigate_to_song(
        &self,
        request: Request<NavigateToSongRequest>,
    ) -> Result<Response<NavigateToSongResponse>, Status> {
        use crate::playlist::EntryError;

        let req = request.into_inner();
        let expected = Some(req.song_name.as_str()).filter(|name| !name.is_empty());
        let song = self
            .player
            .navigate_to_song(req.position as usize, expected)
            .await
            .map_err(|e| match e {
                EntryError::OutOfRange { .. } => Status::out_of_range(e.to_string()),
                EntryError::Changed { .. } | EntryError::Unloadable(_) => {
                    Status::failed_precondition(e.to_string())
                }
            })?;
        Ok(Response::new(NavigateToSongResponse {
            song: Some(song.to_proto()?),
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
            .map_err(|e| fault_status(e.kind(), e.to_string()))?;
        Ok(Response::new(SwitchToPlaylistResponse {}))
    }

    async fn status(&self, _: Request<StatusRequest>) -> Result<Response<StatusResponse>, Status> {
        // The player's one status, its DMX reads bounded: a wedged effects
        // loop leaves the ambient light out rather than hanging the call.
        let status = self.player.playback_status().await;

        let elapsed = elapsed_to_proto(status.elapsed)?;

        let current_song = match &status.current_song {
            Some(song) => Some(song.to_proto()?),
            None => None,
        };

        // Hardware health travels with playback state so a remote monitor can
        // tell "silent because nothing is playing" from "silent and it should
        // not be" without a second call.
        let audio_health = status.audio_health.map(|h| AudioHealth {
            status: h.status.as_str().to_string(),
            callback_alive: h.callback_alive,
            writing_signal: h.writing_signal,
            recoveries: h.recoveries,
            underruns: h.underruns,
            last_error: h.last_error,
        });

        let auto_play = auto_play_to_proto(status.auto_play)?;

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
        let active_effects = self.player.active_effects_summary();

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
        let req = request.into_inner();
        self.update_section(
            ConfigSection::Audio,
            &req.audio_json,
            &req.expected_checksum,
        )
        .await
    }

    async fn update_midi(
        &self,
        request: Request<UpdateMidiRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let req = request.into_inner();
        self.update_section(ConfigSection::Midi, &req.midi_json, &req.expected_checksum)
            .await
    }

    async fn update_dmx(
        &self,
        request: Request<UpdateDmxRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let req = request.into_inner();
        self.update_section(ConfigSection::Dmx, &req.dmx_json, &req.expected_checksum)
            .await
    }

    async fn update_controllers(
        &self,
        request: Request<UpdateControllersRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
        let req = request.into_inner();
        self.update_section(
            ConfigSection::Controllers,
            &req.controllers_json,
            &req.expected_checksum,
        )
        .await
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
        self.reload_and_respond(snapshot).await
    }

    async fn update_profile(
        &self,
        request: Request<UpdateProfileRequest>,
    ) -> Result<Response<UpdateConfigResponse>, Status> {
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
            .map_err(|e| fault_status(e.kind(), e.to_string()))?;
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
            .map_err(|e| fault_status(e.kind(), e.to_string()))?;
        Ok(Response::new(SetTrackMuteResponse { muted }))
    }

    async fn set_ambient_mode(
        &self,
        request: Request<SetAmbientModeRequest>,
    ) -> Result<Response<SetAmbientModeResponse>, Status> {
        let name = request.into_inner().mode;
        let mode = crate::dmx::engine::AmbientMode::parse_or_error(&name)
            .map_err(Status::invalid_argument)?;
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

    async fn list_playlists(
        &self,
        _: Request<ListPlaylistsRequest>,
    ) -> Result<Response<ListPlaylistsResponse>, Status> {
        Ok(Response::new(ListPlaylistsResponse {
            names: self.player.list_playlists(),
            active: self.player.get_playlist().name().to_string(),
        }))
    }

    async fn get_playlist(
        &self,
        request: Request<GetPlaylistRequest>,
    ) -> Result<Response<GetPlaylistResponse>, Status> {
        let name = request.into_inner().name;
        let playlist = if name.is_empty() {
            self.player.get_playlist()
        } else {
            self.player
                .playlists_snapshot()
                .get(&name)
                .cloned()
                .ok_or_else(|| Status::not_found(format!("Playlist '{name}' not found")))?
        };
        Ok(Response::new(playlist_response(&playlist)?))
    }

    async fn save_playlist(
        &self,
        request: Request<SavePlaylistRequest>,
    ) -> Result<Response<GetPlaylistResponse>, Status> {
        let req = request.into_inner();
        let name = req.name.clone();
        let store = self.require_config_store()?;
        playlists::save(&self.player, store, req).await?;
        // As GetPlaylist reports it now, after the reload.
        let playlist = self
            .player
            .playlists_snapshot()
            .get(&name)
            .cloned()
            .ok_or_else(|| Status::not_found(format!("Playlist '{name}' not found")))?;
        Ok(Response::new(playlist_response(&playlist)?))
    }

    async fn delete_playlist(
        &self,
        request: Request<DeletePlaylistRequest>,
    ) -> Result<Response<DeletePlaylistResponse>, Status> {
        let store = self.require_config_store()?;
        playlists::delete(&self.player, store, &request.into_inner().name).await?;
        Ok(Response::new(DeletePlaylistResponse {}))
    }

    async fn set_lock(
        &self,
        request: Request<SetLockRequest>,
    ) -> Result<Response<SetLockResponse>, Status> {
        // What the web UI's lock toggle does, and all it does.
        self.player.set_locked(request.into_inner().locked);
        Ok(Response::new(SetLockResponse {
            locked: self.player.is_locked(),
        }))
    }

    type WatchStageStream = futures_util::stream::BoxStream<'static, Result<StageUpdate, Status>>;

    /// A player with no lighting (no DMX engine, or no sampler wired) is not
    /// an error, as it is not for `GetActiveEffects`: the stream opens with
    /// an empty layout and an empty frame, and says more if lighting comes
    /// up under it.
    async fn watch_stage(
        &self,
        request: Request<WatchStageRequest>,
    ) -> Result<Response<Self::WatchStageStream>, Status> {
        let watch = StageWatch::new(self.player.clone(), request.into_inner().max_rate);
        Ok(Response::new(watch.into_stream()))
    }
}

/// A playlist as `GetPlaylist` reports it.
#[allow(clippy::result_large_err)]
fn playlist_response(playlist: &crate::playlist::Playlist) -> Result<GetPlaylistResponse, Status> {
    let entries = playlist
        .songs()
        .iter()
        .map(|song_name| {
            let duration = playlist
                .get_song(song_name)
                .map(|song| prost_types::Duration::try_from(song.length()))
                .transpose()
                .map_err(|e| Status::internal(e.to_string()))?;
            Ok(PlaylistEntry {
                song_name: song_name.clone(),
                duration,
            })
        })
        .collect::<Result<Vec<_>, Status>>()?;
    Ok(GetPlaylistResponse {
        name: playlist.name().to_string(),
        entries,
        position: playlist.entry().map(|index| index as u32),
    })
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, sync::Arc, time::Duration};

    use tonic::transport::Channel;

    use crate::{
        config,
        controller::{grpc::Driver, Driver as _},
        playlist,
        proto::player::v1::{
            player_service_client::PlayerServiceClient, GetPlaylistRequest, ListPlaylistsRequest,
            LoopSectionRequest, NextRequest, PlayRequest, PreviousRequest, StatusRequest,
            StopRequest, StopSectionLoopRequest, SwitchToPlaylistRequest,
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
        let player = crate::testutil::player::assets_player()
            .midi("mock-midi-device")
            .build()?;
        player.await_hardware_ready().await;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // Direct the player.
        let mut client = serve_grpc(player.clone()).await?;

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
        let player = crate::testutil::player::assets_player()
            .midi("mock-midi-device")
            .track_mappings(track_mappings)
            .build()?;
        player.await_hardware_ready().await;
        // Players start locked; these tests exercise the handlers, not the lock.
        player.set_locked(false);
        let device = player.audio_device().expect("audio device").to_mock()?;
        let client = serve_grpc(player.clone()).await?;
        Ok((player, client, device))
    }

    /// Serves `player` over gRPC on a port the OS picks; a client for it.
    async fn serve_grpc(
        player: Arc<Player>,
    ) -> Result<PlayerServiceClient<Channel>, Box<dyn Error>> {
        // Port 0: the server binds one the OS picks, and says which.
        let driver = Driver::new(config::GrpcController::new(0), player.clone())?;
        drop(driver.monitor_events());
        let port = driver.bound.port().await;
        Ok(PlayerServiceClient::connect(format!("http://127.0.0.1:{port}")).await?)
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
    async fn test_grpc_list_playlists() -> Result<(), Box<dyn Error>> {
        let (player, mut client, _device) = setup_grpc().await?;

        let listed = client.list_playlists(ListPlaylistsRequest {}).await?;
        let listed = listed.into_inner();
        assert_eq!(listed.names, player.list_playlists());
        assert_eq!(listed.names, ["all_songs", "playlist"]);
        assert_eq!(listed.active, "playlist");

        client
            .switch_to_playlist(SwitchToPlaylistRequest {
                playlist_name: "all_songs".to_string(),
            })
            .await?;
        let listed = client.list_playlists(ListPlaylistsRequest {}).await?;
        assert_eq!(listed.into_inner().active, "all_songs");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_get_playlist() -> Result<(), Box<dyn Error>> {
        let (player, mut client, _device) = setup_grpc().await?;

        // An empty name is the active playlist.
        let active = client
            .get_playlist(GetPlaylistRequest {
                name: String::new(),
            })
            .await?
            .into_inner();
        assert_eq!(active.name, "playlist");
        let songs: Vec<&str> = active
            .entries
            .iter()
            .map(|e| e.song_name.as_str())
            .collect();
        assert_eq!(songs, *player.get_playlist().songs());
        assert!(active.entries.iter().all(|e| e.duration.is_some()));
        assert_eq!(active.position, Some(0));

        // Moving the playlist moves the position.
        client.next(NextRequest {}).await?;
        let moved = client
            .get_playlist(GetPlaylistRequest {
                name: "playlist".to_string(),
            })
            .await?
            .into_inner();
        assert_eq!(moved.position, Some(1));

        // A playlist by name, whether or not it is active.
        let all = client
            .get_playlist(GetPlaylistRequest {
                name: "all_songs".to_string(),
            })
            .await?
            .into_inner();
        assert_eq!(all.name, "all_songs");
        assert_eq!(all.entries.len(), 10);
        assert_eq!(all.position, Some(0));

        let status = client
            .get_playlist(GetPlaylistRequest {
                name: "nonexistent".to_string(),
            })
            .await
            .expect_err("an unknown playlist is not found");
        assert_eq!(status.code(), tonic::Code::NotFound);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_play_already_playing() -> Result<(), Box<dyn Error>> {
        let (_player, mut client, device) = setup_grpc().await?;

        // First play should succeed.
        let resp = client.play(PlayRequest {}).await?;
        assert!(resp.into_inner().song.is_some());
        eventually(|| device.is_playing(), "Song never started playing");

        // Second play while already playing should fail, naming why by key.
        let status = client.play(PlayRequest {}).await.unwrap_err();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            status
                .metadata()
                .get(crate::player::PLAY_DECLINED_KEY)
                .map(|v| v.to_str().unwrap()),
            Some("already_playing")
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

    /// While a song plays the player stays on it, and the reply says so:
    /// the song answered is the one the player is on, as MCP answers.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_next_while_playing() -> Result<(), Box<dyn Error>> {
        let (player, mut client, device) = setup_grpc().await?;

        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let current = player.get_playlist().current().unwrap().name().to_string();
        let result = client.next(NextRequest {}).await?.into_inner();
        assert_eq!(result.song.unwrap().name, current);
        assert_eq!(player.get_playlist().current().unwrap().name(), current);

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    fn navigate_request(
        position: u32,
        name: &str,
    ) -> crate::proto::player::v1::NavigateToSongRequest {
        crate::proto::player::v1::NavigateToSongRequest {
            position,
            song_name: name.to_string(),
        }
    }

    /// Navigating to an entry makes it current and does not play it.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_navigate_to_song_stopped() -> Result<(), Box<dyn Error>> {
        let (player, mut client, device) = setup_grpc().await?;

        let names: Vec<String> = player.get_playlist().songs().clone();
        let last = names.len() - 1;
        assert_ne!(player.get_playlist().entry(), Some(last));

        let result = client
            .navigate_to_song(navigate_request(last as u32, &names[last]))
            .await?
            .into_inner();
        assert_eq!(result.song.unwrap().name, names[last]);
        assert_eq!(player.get_playlist().entry(), Some(last));
        assert!(!device.is_playing());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_navigate_to_song_refusals() -> Result<(), Box<dyn Error>> {
        let (player, mut client, _device) = setup_grpc().await?;
        let before = player.get_playlist().entry();
        let first = player.get_playlist().songs()[0].clone();

        // Past the end: OUT_OF_RANGE, as nothing else in the service has a
        // position to be out of.
        let status = client
            .navigate_to_song(navigate_request(999, ""))
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::OutOfRange);

        // A name that is not the entry's: the playlist changed.
        let status = client
            .navigate_to_song(navigate_request(1, "no such song"))
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().contains("playlist has changed"));
        assert_ne!(player.get_playlist().songs()[1], first);
        assert_eq!(player.get_playlist().entry(), before);
        Ok(())
    }

    /// While a song plays the player stays on it, as for Next.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_navigate_to_song_while_playing() -> Result<(), Box<dyn Error>> {
        let (player, mut client, device) = setup_grpc().await?;

        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let current = player.get_playlist().current().unwrap().name().to_string();
        let other = player.get_playlist().songs()[1].clone();
        let result = client
            .navigate_to_song(navigate_request(1, &other))
            .await?
            .into_inner();
        assert_eq!(result.song.unwrap().name, current);
        assert_eq!(player.get_playlist().current().unwrap().name(), current);

        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grpc_previous_while_playing() -> Result<(), Box<dyn Error>> {
        let (player, mut client, device) = setup_grpc().await?;

        // Move to Song 3 first so previous has somewhere to go.
        client.next(NextRequest {}).await?;

        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let current = player.get_playlist().current().unwrap().name().to_string();
        let result = client.previous(PreviousRequest {}).await?.into_inner();
        assert_eq!(result.song.unwrap().name, current);
        assert_eq!(player.get_playlist().current().unwrap().name(), current);

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
        let deadline = std::time::Instant::now() + crate::testutil::PATIENCE;
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
            crate::player::ConfigApply::Failed("no store".into()).into(),
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
        let player = crate::testutil::player::assets_player()
            .dmx(Some(config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![config::Universe::new(1, "test".to_string())],
                None,
            )))
            .ready()
            .await?;
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

        let player = crate::testutil::player::assets_player()
            .playlist_yaml("songs:\n- Song 1\n- song: Song 3\n  auto_play: 2s\n")
            .ready()
            .await?;
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

    /// The RPCs in the proto.
    fn proto_rpcs() -> Vec<&'static str> {
        include_str!("../proto/player/v1/player.proto")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("rpc "))
            .filter_map(|l| l.split('(').next())
            .map(str::trim)
            .collect()
    }

    /// Every RPC the lock policy leaves open is in the proto.
    #[test]
    fn every_open_rpc_is_in_the_proto() {
        let rpcs = proto_rpcs();
        assert!(!rpcs.is_empty());
        for rpc in crate::player::lock_policy::open_on(crate::player::lock_policy::Surface::Grpc) {
            assert!(rpcs.contains(&rpc), "{rpc} is not in the proto");
        }
    }

    /// Every RPC in the proto the lock policy does not leave open is
    /// refused by the service's lock guard while the player is locked,
    /// before its handler runs, so a new RPC is locked out until listed.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_lock_guard_refuses_every_rpc_not_left_open() -> Result<(), Box<dyn Error>> {
        use tower::ServiceExt;
        let (player, _client, _device) = setup_grpc().await?;
        player.set_locked(true);
        let open: Vec<&str> =
            crate::player::lock_policy::open_on(crate::player::lock_policy::Surface::Grpc)
                .collect();
        let locked: Vec<&str> = proto_rpcs()
            .into_iter()
            .filter(|rpc| !open.contains(rpc))
            .collect();
        assert!(locked.contains(&"UpdateAudio"));
        for rpc in locked {
            use tonic::server::NamedService;
            let service = <super::PlayerServiceServer<super::PlayerServer> as NamedService>::NAME;
            let request = http::Request::post(format!("/{service}/{rpc}"))
                .header("content-type", "application/grpc")
                .body(tonic::body::Body::empty())?;
            let response = super::player_service(player.clone())
                .oneshot(request)
                .await?;
            let status = tonic::Status::from_header_map(response.headers()).expect(rpc);
            assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{rpc}");
            assert_eq!(
                status.message(),
                crate::player::LockedError.to_string(),
                "{rpc}"
            );
        }
        Ok(())
    }

    /// A locked player, served over gRPC, refuses every config write as
    /// FAILED_PRECONDITION and leaves the file alone, while reads and
    /// playback controls still work.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_player_refuses_config_writes() -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::{
            AddProfileRequest, GetConfigRequest, RemoveProfileRequest, StopSamplesRequest,
            UpdateAudioRequest, UpdateControllersRequest, UpdateDmxRequest, UpdateMidiRequest,
            UpdateProfileRequest,
        };
        use tonic::Status;

        let (player, _client, device) = setup_grpc().await?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: assets/songs\n")?;
        let store = Arc::new(config::ConfigStore::new(
            config::Player::deserialize(&path)?,
            path.clone(),
        ));
        player.set_config_store(store.clone());
        // Served after the store is set, so the service has it.
        let mut client = serve_grpc(player.clone()).await?;
        let before = std::fs::read_to_string(&path)?;
        let (_, checksum) = store.read_yaml().await?;
        player.set_locked(true);

        let r = |c: &str| c.to_string();
        let results: Vec<(&str, Result<(), Status>)> = vec![
            (
                "UpdateAudio",
                client
                    .update_audio(UpdateAudioRequest {
                        audio_json: String::new(),
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateMidi",
                client
                    .update_midi(UpdateMidiRequest {
                        midi_json: String::new(),
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateDmx",
                client
                    .update_dmx(UpdateDmxRequest {
                        dmx_json: String::new(),
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateControllers",
                client
                    .update_controllers(UpdateControllersRequest {
                        controllers_json: "[]".to_string(),
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "AddProfile",
                client
                    .add_profile(AddProfileRequest {
                        profile_json: r#"{"hostname": "x"}"#.to_string(),
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "UpdateProfile",
                client
                    .update_profile(UpdateProfileRequest {
                        index: 0,
                        profile_json: "{}".to_string(),
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
            (
                "RemoveProfile",
                client
                    .remove_profile(RemoveProfileRequest {
                        index: 0,
                        expected_checksum: r(&checksum),
                    })
                    .await
                    .map(|_| ()),
            ),
        ];
        for (rpc, result) in results {
            let status = result.expect_err(rpc);
            assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{rpc}");
            assert!(status.message().contains("locked"), "{rpc}: {status}");
        }
        assert_eq!(std::fs::read_to_string(&path)?, before);

        // Reads and playback controls stay open.
        client.get_config(GetConfigRequest {}).await?;
        client.status(StatusRequest {}).await?;
        client.play(PlayRequest {}).await?;
        eventually(|| device.is_playing(), "Song never started playing");
        client.stop(StopRequest {}).await?;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        client.next(NextRequest {}).await?;
        client.stop_samples(StopSamplesRequest {}).await?;
        client.stop_section_loop(StopSectionLoopRequest {}).await?;
        assert!(player.is_locked());
        Ok(())
    }

    type StageStream = <super::PlayerServer as super::PlayerService>::WatchStageStream;
    type StateTx = Arc<tokio::sync::watch::Sender<Arc<crate::state::StateSnapshot>>>;

    /// A snapshot of fixtures, each with channels and their values.
    fn stage_of(fixtures: &[(&str, &[(&str, u8)])]) -> Arc<crate::state::StateSnapshot> {
        Arc::new(crate::state::StateSnapshot {
            fixtures: fixtures
                .iter()
                .map(|(name, channels)| crate::state::FixtureSnapshot {
                    name: name.to_string(),
                    channels: channels.iter().map(|(c, v)| (c.to_string(), *v)).collect(),
                    cells: Default::default(),
                    strobe_hz: None,
                })
                .collect(),
            ..Default::default()
        })
    }

    /// A player whose stage is `first`, its stage stream at `max_rate`, and
    /// the sender the sampler would push snapshots through.
    async fn watch_stage(
        first: Arc<crate::state::StateSnapshot>,
        max_rate: u32,
    ) -> (Arc<Player>, StateTx, StageStream) {
        let player = crate::testutil::player::memory_player(&["Song A"]).build();
        let (state_tx, _state_rx) = tokio::sync::watch::channel(first);
        player.set_state_tx(state_tx);
        let state_tx = player.state_tx().expect("the sender just set");
        let stream = open_stage(&player, max_rate).await;
        (player, state_tx, stream)
    }

    async fn open_stage(player: &Arc<Player>, max_rate: u32) -> StageStream {
        use crate::proto::player::v1::WatchStageRequest;
        let server = super::PlayerServer::new(player.clone());
        super::PlayerService::watch_stage(
            &server,
            tonic::Request::new(WatchStageRequest { max_rate }),
        )
        .await
        .expect("the stream opens")
        .into_inner()
    }

    async fn next_update(
        stream: &mut StageStream,
    ) -> crate::proto::player::v1::stage_update::Update {
        use futures_util::StreamExt;
        tokio::time::timeout(crate::testutil::PATIENCE, stream.next())
            .await
            .expect("the stream said nothing")
            .expect("the stream ended")
            .expect("the stream failed")
            .update
            .expect("an update")
    }

    async fn next_layout(stream: &mut StageStream) -> crate::proto::player::v1::StageLayout {
        match next_update(stream).await {
            crate::proto::player::v1::stage_update::Update::Layout(layout) => layout,
            other => panic!("expected a layout, got {other:?}"),
        }
    }

    async fn next_frame(stream: &mut StageStream) -> crate::proto::player::v1::StageFrame {
        match next_update(stream).await {
            crate::proto::player::v1::stage_update::Update::Frame(frame) => frame,
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    /// A client joining an idle rig sees it at once: nothing has to change
    /// first.
    #[tokio::test]
    async fn the_stage_stream_opens_with_the_layout_and_then_a_frame() {
        let first = stage_of(&[
            ("par", &[("red", 9), ("dimmer", 200)]),
            ("bar", &[("dimmer", 7)]),
        ]);
        let (_player, _state_tx, mut stream) = watch_stage(first, 0).await;

        let layout = next_layout(&mut stream).await;
        let names: Vec<&str> = layout.fixtures.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["bar", "par"]);
        assert_eq!(layout.fixtures[1].channels, ["dimmer", "red"]);
        assert_eq!(layout.venue, None, "no lighting system, no venue");
        assert_eq!(layout.id, 1, "the stream's first layout");
        let frame = next_frame(&mut stream).await;
        assert_eq!(frame.channels, [7, 200, 9]);
        assert_eq!(frame.layout, 1);
    }

    /// A snapshot that changes something is a frame; one that says what the
    /// last frame said is nothing, though the sampler published it.
    #[tokio::test]
    async fn the_stage_stream_sends_a_frame_for_a_change_and_nothing_for_none() {
        let (_player, state_tx, mut stream) =
            watch_stage(stage_of(&[("par", &[("red", 1)])]), 0).await;
        next_layout(&mut stream).await;
        assert_eq!(next_frame(&mut stream).await.channels, [1]);

        state_tx.send(stage_of(&[("par", &[("red", 2)])])).unwrap();
        let frame = next_frame(&mut stream).await;
        assert_eq!(
            (frame.channels.as_slice(), frame.layout),
            ([2].as_slice(), 1)
        );

        // The same again, then a change: the next message is the change, so
        // the repeat was never sent.
        state_tx.send(stage_of(&[("par", &[("red", 2)])])).unwrap();
        let waited = tokio::time::timeout(Duration::from_millis(50), next_update(&mut stream));
        assert!(waited.await.is_err(), "a repeat is not a frame");
        state_tx.send(stage_of(&[("par", &[("red", 3)])])).unwrap();
        assert_eq!(next_frame(&mut stream).await.channels, [3]);
    }

    /// A frame is bytes in the layout's order, so a fixture or a channel the
    /// layout does not name is a new layout first.
    #[tokio::test]
    async fn the_stage_stream_sends_a_layout_before_a_frame_that_no_longer_fits() {
        let (_player, state_tx, mut stream) =
            watch_stage(stage_of(&[("par", &[("red", 1)])]), 0).await;
        next_layout(&mut stream).await;
        next_frame(&mut stream).await;

        // A new fixture.
        state_tx
            .send(stage_of(&[
                ("par", &[("red", 1)]),
                ("bar", &[("dimmer", 5)]),
            ]))
            .unwrap();
        let layout = next_layout(&mut stream).await;
        let names: Vec<&str> = layout.fixtures.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["bar", "par"]);
        assert_eq!(next_frame(&mut stream).await.channels, [5, 1]);

        // A new channel.
        state_tx
            .send(stage_of(&[
                ("par", &[("red", 1), ("green", 2)]),
                ("bar", &[("dimmer", 5)]),
            ]))
            .unwrap();
        let layout = next_layout(&mut stream).await;
        assert_eq!(layout.fixtures[1].channels, ["green", "red"]);
        assert_eq!(next_frame(&mut stream).await.channels, [5, 2, 1]);

        // A fixture gone.
        state_tx
            .send(stage_of(&[("bar", &[("dimmer", 6)])]))
            .unwrap();
        assert_eq!(next_layout(&mut stream).await.fixtures.len(), 1);
        assert_eq!(next_frame(&mut stream).await.channels, [6]);
    }

    /// The venue changing under the player (a venue or fixture type reload,
    /// a hardware reload) is a layout, and the frame that is read against
    /// it, though no value moved.
    #[tokio::test]
    async fn the_stage_stream_sends_the_layout_again_when_its_revision_moves() {
        let (player, state_tx, mut stream) =
            watch_stage(stage_of(&[("par", &[("red", 1)])]), 0).await;
        let first = next_layout(&mut stream).await;
        next_frame(&mut stream).await;

        player.bump_stage_layout();
        let second = next_layout(&mut stream).await;
        assert_eq!((first.id, second.id), (1, 2));
        assert_eq!(second.fixtures, first.fixtures);
        // The frame after it is read against the new layout, and says so,
        // as every frame does until the next layout.
        let frame = next_frame(&mut stream).await;
        assert_eq!(
            (frame.channels.as_slice(), frame.layout),
            ([1].as_slice(), 2)
        );
        state_tx.send(stage_of(&[("par", &[("red", 9)])])).unwrap();
        let frame = next_frame(&mut stream).await;
        assert_eq!(
            (frame.channels.as_slice(), frame.layout),
            ([9].as_slice(), 2)
        );

        // A layout the snapshot forces is numbered the same way.
        state_tx
            .send(stage_of(&[("bar", &[("dimmer", 3)])]))
            .unwrap();
        assert_eq!(next_layout(&mut stream).await.id, 3);
        assert_eq!(next_frame(&mut stream).await.layout, 3);
    }

    /// Frames come no closer together than the caller asked, and the one
    /// sent after the wait is the newest: the ones between are dropped.
    #[tokio::test]
    async fn the_stage_stream_keeps_to_the_callers_rate_and_sends_the_newest() {
        let interval = Duration::from_millis(250);
        let opened = tokio::time::Instant::now();
        let (_player, state_tx, mut stream) =
            watch_stage(stage_of(&[("par", &[("red", 0)])]), 4).await;
        next_layout(&mut stream).await;
        next_frame(&mut stream).await;

        for value in 1..=3 {
            state_tx
                .send(stage_of(&[("par", &[("red", value)])]))
                .unwrap();
        }
        assert_eq!(next_frame(&mut stream).await.channels, [3]);
        assert!(
            opened.elapsed() >= interval,
            "sent before the interval was over"
        );

        // And again: the stream is not left a frame behind.
        let sent = tokio::time::Instant::now();
        for value in 4..=6 {
            state_tx
                .send(stage_of(&[("par", &[("red", value)])]))
                .unwrap();
        }
        assert_eq!(next_frame(&mut stream).await.channels, [6]);
        assert!(sent.elapsed() >= interval - Duration::from_millis(5));
    }

    /// No sampler wired is not an error, as no DMX engine is not for
    /// GetActiveEffects: an empty stage, which fills in if lighting comes up.
    #[tokio::test]
    async fn the_stage_stream_of_a_player_with_no_lighting_is_an_empty_stage() {
        let player = crate::testutil::player::memory_player(&["Song A"]).build();
        let mut stream = open_stage(&player, 0).await;
        let layout = next_layout(&mut stream).await;
        assert_eq!(
            (layout.id, layout.fixtures.len(), layout.venue),
            (1, 0, None)
        );
        let frame = next_frame(&mut stream).await;
        assert_eq!(
            frame,
            crate::proto::player::v1::StageFrame {
                layout: 1,
                ..Default::default()
            }
        );

        // Lighting comes up: a state sender, and the engine's install bumps
        // the layout.
        let (state_tx, _state_rx) =
            tokio::sync::watch::channel(stage_of(&[("par", &[("red", 4)])]));
        player.set_state_tx(state_tx);
        player.bump_stage_layout();
        assert_eq!(next_layout(&mut stream).await.fixtures[0].name, "par");
        assert_eq!(next_frame(&mut stream).await.channels, [4]);
    }

    /// Over the wire: the stream is a read, open while the player is locked,
    /// and a client that goes away takes its watch with it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_stage_stream_is_open_while_locked_and_ends_with_its_client(
    ) -> Result<(), Box<dyn Error>> {
        use crate::proto::player::v1::{stage_update::Update, WatchStageRequest};
        let (player, mut client, _device) = setup_grpc().await?;
        let (state_tx, state_rx) = tokio::sync::watch::channel(stage_of(&[("par", &[("red", 1)])]));
        player.set_state_tx(state_tx);
        drop(state_rx);
        let state_tx = player.state_tx().expect("the sender just set");
        player.set_locked(true);

        let mut stream = client
            .watch_stage(WatchStageRequest { max_rate: 0 })
            .await?
            .into_inner();
        let Some(Update::Layout(layout)) = stream.message().await?.and_then(|u| u.update) else {
            panic!("expected a layout first");
        };
        assert_eq!(layout.fixtures[0].name, "par");
        let Some(Update::Frame(frame)) = stream.message().await?.and_then(|u| u.update) else {
            panic!("expected a frame second");
        };
        assert_eq!(frame.channels, [1]);
        assert_eq!(state_tx.receiver_count(), 1, "the stream's own");

        drop(stream);
        eventually(
            || state_tx.receiver_count() == 0,
            "the stream outlived its client",
        );
        Ok(())
    }
}
