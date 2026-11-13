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

//! `Watch`: one stream of what changes, for a client that draws the whole
//! dashboard. It carries what the web UI's websocket carries and what a poll
//! of its status page does, as proto messages.
//!
//! The feeds are the player's [`WatchHub`](crate::webui::hub::WatchHub): the
//! same typed messages the websocket writes as JSON are mapped here to
//! [`WatchEvent`]s, field for field. The hardware status is not a feed of the
//! hub: each stream that asks for it reads it once a second and sends it when
//! it differs from the one it last sent.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{stream::BoxStream, StreamExt};
use prost_types::Duration as ProtoDuration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tonic::Status;

use crate::config::{FadeCurve, FadeStatus, Section};
use crate::player::{
    AudioOutputHealth as OutputHealth, ControllerStatus as ControllerSnapshot,
    HardwareStatusSnapshot, Player,
};
use crate::proto::player::v1 as proto;
use crate::proto::player::v1::{watch_event::Event, WatchEvent, WatchRequest, WatchTopic};
use crate::webui::wire::messages as wire;

pub(super) type WatchStream = BoxStream<'static, Result<WatchEvent, Status>>;

/// How often a stream compares the hardware status with the last it sent.
const HARDWARE_INTERVAL: Duration = Duration::from_secs(1);

/// How many events a stream holds for a client that reads slowly, before
/// the task that feeds it waits.
const BUFFER: usize = 16;

/// The topics a request asks for: those it lists, or all when it lists
/// none. Unspecified and unknown values are ignored, so a list of only
/// those asks for all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Topics {
    playback: bool,
    logs: bool,
    waveform: bool,
    config_changed: bool,
    reload: bool,
    hardware: bool,
}

impl Topics {
    fn of(request: &WatchRequest) -> Self {
        let mut topics = Topics::none();
        for topic in request
            .topics
            .iter()
            .filter_map(|value| WatchTopic::try_from(*value).ok())
        {
            match topic {
                WatchTopic::Unspecified => {}
                WatchTopic::Playback => topics.playback = true,
                WatchTopic::Logs => topics.logs = true,
                WatchTopic::Waveform => topics.waveform = true,
                WatchTopic::ConfigChanged => topics.config_changed = true,
                WatchTopic::Reload => topics.reload = true,
                WatchTopic::Hardware => topics.hardware = true,
            }
        }
        if topics == Topics::none() {
            Topics::all()
        } else {
            topics
        }
    }

    fn none() -> Self {
        Topics {
            playback: false,
            logs: false,
            waveform: false,
            config_changed: false,
            reload: false,
            hardware: false,
        }
    }

    fn all() -> Self {
        Topics {
            playback: true,
            logs: true,
            waveform: true,
            config_changed: true,
            reload: true,
            hardware: true,
        }
    }

    /// Whether any of the hub's feeds is wanted.
    fn feeds(&self) -> bool {
        self.playback || self.logs || self.waveform || self.config_changed || self.reload
    }

    /// Whether the hub's message `msg` is on a wanted topic.
    fn wants(&self, msg: &wire::SocketMessage) -> bool {
        match msg {
            wire::SocketMessage::Playback(_) => self.playback,
            wire::SocketMessage::Logs { .. } => self.logs,
            wire::SocketMessage::Waveform(_) => self.waveform,
            wire::SocketMessage::ConfigChanged => self.config_changed,
            wire::SocketMessage::Reload(_) => self.reload,
        }
    }
}

/// Opens the stream: the current playback, the current song's cached waveform
/// and the hardware status at once (as far as they are asked for), then the
/// hub's news as it comes and the hardware status when it changes.
///
/// A reader that falls behind the hub skips what it missed and carries on.
/// The task that feeds the stream ends when the stream is dropped.
pub(super) fn watch(player: Arc<Player>, request: WatchRequest) -> WatchStream {
    let topics = Topics::of(&request);
    let (out, events) = mpsc::channel(BUFFER);
    tokio::spawn(feed(player, topics, out));
    futures_util::stream::unfold(events, |mut events| async {
        let event = events.recv().await?;
        Some((Ok(event), events))
    })
    .boxed()
}

async fn feed(player: Arc<Player>, topics: Topics, out: mpsc::Sender<WatchEvent>) {
    // Subscribed before the snapshots are read, so what happens between
    // them and the first read of the feed is not lost (and at worst said
    // twice).
    let mut feed = topics.feeds().then(|| player.watch_hub().subscribe());

    if topics.playback {
        let playback = crate::webui::state::current_playback(&player).await;
        if !send(&out, Event::Playback(playback_to_proto(&playback))).await {
            return;
        }
    }
    if topics.waveform {
        if let Some((song_name, tracks)) = player.watch_hub().cached_waveform(&player) {
            let waveform = proto::Waveform {
                song_name,
                tracks: tracks
                    .into_iter()
                    .map(|(name, peaks)| proto::WaveformTrack { name, peaks })
                    .collect(),
            };
            if !send(&out, Event::Waveform(waveform)).await {
                return;
            }
        }
    }

    // The build does not change while the player runs.
    let build = build_info();
    let mut last_hardware = None;
    if topics.hardware {
        let status = hardware_status(&player, &build);
        last_hardware = Some(status.clone());
        if !send(&out, Event::Hardware(status)).await {
            return;
        }
    }

    let mut tick = tokio::time::interval_at(
        tokio::time::Instant::now() + HARDWARE_INTERVAL,
        HARDWARE_INTERVAL,
    );
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = out.closed() => return,
            message = async {
                match feed.as_mut() {
                    Some(feed) => feed.recv().await,
                    None => std::future::pending().await,
                }
            } => match message {
                Ok(message) => {
                    if topics.wants(&message) && !send(&out, message_to_proto(&message)).await {
                        return;
                    }
                }
                // Skips what it missed, as the websocket does.
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            _ = tick.tick(), if topics.hardware => {
                let status = hardware_status(&player, &build);
                if last_hardware.as_ref() != Some(&status) {
                    last_hardware = Some(status.clone());
                    if !send(&out, Event::Hardware(status)).await {
                        return;
                    }
                }
            }
        }
    }
}

/// Sends `event`; false when the stream has been dropped.
async fn send(out: &mpsc::Sender<WatchEvent>, event: Event) -> bool {
    out.send(WatchEvent { event: Some(event) }).await.is_ok()
}

/// The player's build, and the web UI it serves.
fn build_info() -> proto::BuildInfo {
    proto::BuildInfo {
        version: crate::build_info::VERSION.to_string(),
        git_hash: crate::build_info::GIT_HASH.to_string(),
        build_time: crate::build_info::BUILD_TIME.to_string(),
        ui_build: crate::webui::server::ui_build(),
    }
}

/// The build, hardware and controllers, as a proto message.
fn hardware_status(player: &Player, build: &proto::BuildInfo) -> proto::HardwareStatus {
    proto::HardwareStatus {
        build: Some(build.clone()),
        hardware: Some(hardware_to_proto(&player.hardware_status())),
        controllers: player
            .controller_statuses()
            .iter()
            .map(controller_to_proto)
            .collect(),
        locked: player.is_locked(),
    }
}

fn message_to_proto(message: &wire::SocketMessage) -> Event {
    match message {
        wire::SocketMessage::Playback(playback) => Event::Playback(playback_to_proto(playback)),
        wire::SocketMessage::Logs { lines } => Event::Logs(proto::LogLines {
            lines: lines
                .iter()
                .map(|line| proto::LogLine {
                    level: line.level.clone(),
                    target: line.target.clone(),
                    message: line.message.clone(),
                })
                .collect(),
        }),
        wire::SocketMessage::Waveform(waveform) => Event::Waveform(proto::Waveform {
            song_name: waveform.song_name.clone(),
            tracks: waveform
                .tracks
                .iter()
                .map(|track| proto::WaveformTrack {
                    name: track.name.clone(),
                    // Widened from f32 on the way in; narrowed back it is
                    // the player's own value.
                    peaks: track.peaks.iter().map(|peak| *peak as f32).collect(),
                })
                .collect(),
        }),
        wire::SocketMessage::ConfigChanged => Event::ConfigChanged(proto::ConfigChanged {}),
        wire::SocketMessage::Reload(reload) => Event::Reload(proto::ShowReload {
            error: match reload {
                wire::ShowReload::Ok => None,
                wire::ShowReload::Error { error } => Some(error.clone()),
            },
        }),
    }
}

/// A length of whole milliseconds, as the wire carries it.
fn millis(ms: u64) -> ProtoDuration {
    ProtoDuration {
        seconds: (ms / 1000) as i64,
        nanos: ((ms % 1000) * 1_000_000) as i32,
    }
}

fn section_to_proto(section: &Section) -> proto::Section {
    proto::Section {
        name: section.name.clone(),
        start_measure: section.start_measure as u32,
        end_measure: section.end_measure as u32,
        start_beat: section.start_beat,
        end_beat: section.end_beat,
        color: section.color.clone(),
    }
}

fn fade_to_proto(fade: &FadeStatus) -> proto::Fade {
    proto::Fade {
        length: fade.length.clone(),
        curve: Some(
            match fade.curve {
                FadeCurve::EqualPower => "equal_power",
                FadeCurve::Linear => "linear",
            }
            .to_string(),
        ),
        exclude: fade.exclude.as_ref().map(|names| proto::TrackNames {
            names: names.clone(),
        }),
        lights: fade.lights.clone(),
    }
}

fn next_to_proto(next: &wire::NextUp) -> proto::NextUp {
    proto::NextUp {
        song: next.song.clone(),
        breaks_loop: next.breaks_loop,
        ends_loop: next.ends_loop,
        fade_out: next.fade_out.as_ref().map(|fade| proto::NextFade {
            fade: fade.as_ref().map(fade_to_proto),
        }),
        fade_in: next.fade_in.as_ref().map(|fade| proto::NextFade {
            fade: fade.as_ref().map(fade_to_proto),
        }),
        auto_play: next
            .auto_play_ms
            .map(|gap| proto::NextAutoPlay { gap_ms: gap }),
    }
}

fn loop_state_to_proto(state: &wire::LoopState) -> proto::LoopState {
    let (name, section) = match state {
        wire::LoopState::Idle => ("idle", None),
        wire::LoopState::SectionOffered { section_name } => ("section_offered", Some(section_name)),
        wire::LoopState::LoopArmed { section_name } => ("loop_armed", Some(section_name)),
        wire::LoopState::Looping { section_name } => ("looping", Some(section_name)),
        wire::LoopState::BreakRequested { section_name } => ("break_requested", Some(section_name)),
    };
    proto::LoopState {
        state: name.to_string(),
        section_name: section.cloned(),
    }
}

/// The playback status as the websocket's `playback` message says it, field
/// for field.
pub(super) fn playback_to_proto(playback: &wire::Playback) -> proto::Playback {
    proto::Playback {
        is_playing: playback.is_playing,
        elapsed: Some(millis(playback.elapsed_ms)),
        song_name: playback.song_name.clone(),
        song_duration: Some(millis(playback.song_duration_ms)),
        playlist_name: playback.playlist_name.clone(),
        playlist_position: playback.playlist_position as u32,
        playlist_off_list: playback.playlist_off_list,
        playlist_at_first: playback.playlist_at_first,
        playlist_at_last: playback.playlist_at_last,
        playlist_songs: playback.playlist_songs.clone(),
        tracks: playback
            .tracks
            .iter()
            .map(|track| proto::PlaybackTrack {
                name: track.name.clone(),
                output_channels: track
                    .output_channels
                    .iter()
                    .map(|c| u32::from(*c))
                    .collect(),
                gain_db: track.gain_db,
                muted: track.muted,
            })
            .collect(),
        available_playlists: playback.available_playlists.clone(),
        persisted_playlist_name: playback.persisted_playlist_name.clone(),
        locked: playback.locked,
        hardware_reload_started: playback.hardware_reload_started,
        hardware_reload_refused: playback
            .hardware_reload_refused
            .iter()
            .map(|refused| proto::RefusedReload {
                generation: refused.generation,
                error: refused.error.clone(),
            })
            .collect(),
        process_id: playback.process_id.clone(),
        beat_grid: playback.beat_grid.as_ref().map(|grid| proto::PlaybackGrid {
            grid: Some(proto::BeatGrid {
                beats: grid.grid.beats.clone(),
                measure_starts: grid.grid.measure_starts.iter().map(|&i| i as u32).collect(),
            }),
            accent_levels: grid.accent_levels.clone(),
        }),
        available_sections: playback
            .available_sections
            .iter()
            .map(section_to_proto)
            .collect(),
        active_section: playback
            .active_section
            .as_ref()
            .map(|section| proto::ActiveSection {
                name: section.name.clone(),
                start: Some(millis(section.start_ms)),
                end: Some(millis(section.end_ms)),
            }),
        looping: playback.looping,
        reactive_loop_state: Some(loop_state_to_proto(&playback.reactive_loop_state)),
        pending_start: playback.pending_start_ms.map(millis),
        pilot_hints: playback
            .pilot_hints
            .iter()
            .map(|hint| proto::PilotHint {
                label: hint.label.clone(),
                at: Some(millis(hint.at_ms)),
                start: Some(millis(hint.start_ms)),
                end: Some(millis(hint.end_ms)),
                has_audio: hint.has_audio,
            })
            .collect(),
        tempo: playback.tempo.as_ref().map(|tempo| proto::PlaybackTempo {
            bpm: tempo.bpm,
            numerator: tempo.time_signature.0,
            denominator: tempo.time_signature.1,
        }),
        auto_play: playback
            .auto_play
            .as_ref()
            .map(|auto_play| proto::AutoPlayStatus {
                song: auto_play.song.clone(),
                starts_in: Some(millis(auto_play.starts_in_ms)),
            }),
        ending: playback
            .ending
            .as_ref()
            .map(|ending| proto::PlaybackEnding {
                song: ending.song.clone(),
                remaining: Some(millis(ending.remaining_ms)),
            }),
        ambient: playback.ambient.as_ref().map(super::ambient_state_to_proto),
        next: playback.next.as_ref().map(next_to_proto),
    }
}

fn subsystem_to_proto(subsystem: &crate::player::SubsystemStatus) -> proto::SubsystemStatus {
    proto::SubsystemStatus {
        status: subsystem.status.clone(),
        name: subsystem.name.clone(),
        error: subsystem.error.clone(),
    }
}

fn audio_output_to_proto(health: &OutputHealth) -> proto::AudioOutputHealth {
    proto::AudioOutputHealth {
        status: health.status.as_str().to_string(),
        callback_alive: health.callback_alive,
        writing_signal: health.writing_signal,
        since_last_callback: health.since_last_callback_ms.map(millis),
        since_last_signal: health.since_last_signal_ms.map(millis),
        callbacks: health.callbacks,
        recoveries: health.recoveries,
        underruns: health.underruns,
        last_error: health.last_error.clone(),
    }
}

fn hardware_to_proto(hardware: &HardwareStatusSnapshot) -> proto::HardwareSnapshot {
    proto::HardwareSnapshot {
        init_done: hardware.init_done,
        hostname: hardware.hostname.clone(),
        profile: hardware.profile.clone(),
        profile_file: hardware.profile_file.clone(),
        audio: Some(subsystem_to_proto(&hardware.audio)),
        audio_output: hardware.audio_output.as_ref().map(audio_output_to_proto),
        midi: Some(subsystem_to_proto(&hardware.midi)),
        dmx: Some(subsystem_to_proto(&hardware.dmx)),
        trigger: Some(subsystem_to_proto(&hardware.trigger)),
        lighting_venue: Some(proto::LightingVenueStatus {
            name: hardware.lighting_venue.name.clone(),
            status: hardware.lighting_venue.status.clone(),
            error: hardware.lighting_venue.error.clone(),
        }),
        test_output: hardware
            .test_output
            .as_ref()
            .map(|test| proto::TestOutputStatus {
                universe: u32::from(test.universe),
                address: u32::from(test.address),
                footprint: u32::from(test.footprint),
                fixture_type: test.fixture_type.clone(),
                mode: test.mode.clone(),
                expires_in_secs: test.expires_in_secs,
            }),
    }
}

fn controller_to_proto(controller: &ControllerSnapshot) -> proto::ControllerStatus {
    proto::ControllerStatus {
        kind: controller.kind.clone(),
        status: controller.status.clone(),
        detail: controller.detail.clone(),
        error: controller.error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::grpc::PlayerServer;
    use crate::proto::player::v1::player_service_server::PlayerService;
    use crate::webui::wire::messages::SocketMessage;
    use std::collections::HashMap;

    /// A project on disk: song S (10 seconds, one track, 120 bpm in 4/4,
    /// sections "a" and "b") followed by song T, on a mock audio device
    /// that maps S's track to outputs 1 and 2. Players start locked.
    async fn project() -> (tempfile::TempDir, Arc<Player>) {
        let dir = tempfile::tempdir().expect("dir");
        let songs_dir = dir.path().join("songs");
        for name in ["S", "T"] {
            let song_dir = songs_dir.join(name);
            std::fs::create_dir_all(&song_dir).expect("song dir");
            crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 10]], 8000)
                .expect("wav");
        }
        std::fs::write(
            songs_dir.join("S/song.yaml"),
            "name: S\ntracks:\n  - name: t\n    file: t.wav\ntempo:\n  bpm: 120\n  \
             time_signature: 4/4\nsections:\n  - name: a\n    start_measure: 1\n    \
             end_measure: 2\n  - name: b\n    start_measure: 2\n    end_measure: 3\n",
        )
        .expect("song S");
        std::fs::write(
            songs_dir.join("T/song.yaml"),
            "name: T\ntracks:\n  - name: t\n    file: t.wav\n",
        )
        .expect("song T");
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs:\n- S\n- T\n").expect("playlist");
        let songs = crate::songs::get_all_songs(&songs_dir).expect("songs");
        let playlist = crate::playlist::Playlist::new(
            "playlist",
            &crate::config::Playlist::deserialize(&playlist_path).expect("playlist"),
            songs.clone(),
        )
        .expect("playlist");
        let player = Player::new(
            HashMap::from([
                (
                    "all_songs".to_string(),
                    crate::playlist::from_songs(songs).expect("all songs"),
                ),
                ("playlist".to_string(), playlist),
            ]),
            "playlist".to_string(),
            &crate::config::Player::new(
                vec![],
                Some(crate::config::Audio::new("mock-device")),
                None,
                None,
                HashMap::from([("t".to_string(), vec![1, 2])]),
                songs_dir.to_str().expect("utf-8"),
            ),
            None,
        )
        .expect("player");
        player.await_hardware_ready().await;
        (dir, player)
    }

    /// Opens `Watch` on `player` for `topics` (raw values, so an unknown
    /// one can be asked for), through the service.
    async fn open(player: &Arc<Player>, topics: &[i32]) -> WatchStream {
        let server = PlayerServer::new(player.clone());
        PlayerService::watch(
            &server,
            tonic::Request::new(WatchRequest {
                topics: topics.to_vec(),
            }),
        )
        .await
        .expect("the stream opens")
        .into_inner()
    }

    async fn next_event(stream: &mut WatchStream) -> Event {
        tokio::time::timeout(crate::testutil::PATIENCE, stream.next())
            .await
            .expect("the stream said nothing")
            .expect("the stream ended")
            .expect("the stream failed")
            .event
            .expect("an event")
    }

    /// Nothing for a while: the stream is quiet.
    async fn is_quiet(stream: &mut WatchStream, wait: Duration) -> bool {
        tokio::time::timeout(wait, stream.next()).await.is_err()
    }

    async fn next_playback(stream: &mut WatchStream) -> proto::Playback {
        match next_event(stream).await {
            Event::Playback(playback) => playback,
            other => panic!("expected playback, got {other:?}"),
        }
    }

    async fn next_hardware(stream: &mut WatchStream) -> proto::HardwareStatus {
        match next_event(stream).await {
            Event::Hardware(hardware) => hardware,
            other => panic!("expected hardware, got {other:?}"),
        }
    }

    const ALL: &[i32] = &[];

    fn topic(topic: WatchTopic) -> i32 {
        topic as i32
    }

    /// A stream that asks for everything opens with the playback and the
    /// hardware at once, and says nothing more while nothing happens.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_stream_opens_with_the_playback_and_the_hardware() {
        let (_dir, player) = project().await;
        let mut stream = open(&player, ALL).await;

        let playback = next_playback(&mut stream).await;
        assert_eq!(playback.song_name, "S");
        assert!(!playback.is_playing);
        assert_eq!(playback.song_duration, Some(millis(10_000)));
        assert_eq!(playback.playlist_name, "playlist");
        assert_eq!(playback.playlist_songs, ["S", "T"]);
        assert!(playback.playlist_at_first);
        assert!(!playback.playlist_at_last);
        assert!(playback.locked, "players start locked");
        assert_eq!(playback.process_id, crate::player::process_id().to_string());
        assert_eq!(
            playback.tracks,
            [proto::PlaybackTrack {
                name: "t".to_string(),
                output_channels: vec![1, 2],
                gain_db: 0.0,
                muted: false,
            }]
        );
        assert_eq!(
            playback.available_sections,
            [
                proto::Section {
                    name: "a".to_string(),
                    start_measure: 1,
                    end_measure: 2,
                    ..Default::default()
                },
                proto::Section {
                    name: "b".to_string(),
                    start_measure: 2,
                    end_measure: 3,
                    ..Default::default()
                },
            ]
        );
        assert_eq!(
            playback.tempo,
            Some(proto::PlaybackTempo {
                bpm: 120.0,
                numerator: 4,
                denominator: 4
            })
        );
        let next = playback.next.expect("a song follows");
        assert_eq!(next.song.as_deref(), Some("T"));
        // A song that is not playing goes on by Play, with no fades and no
        // auto play: each said as decided to be none.
        assert_eq!(next.breaks_loop, Some(false));
        assert_eq!(next.fade_out, Some(proto::NextFade { fade: None }));
        assert_eq!(next.fade_in, Some(proto::NextFade { fade: None }));
        assert_eq!(next.auto_play, Some(proto::NextAutoPlay { gap_ms: None }));
        assert_eq!(next.ends_loop, None, "not decided by this case");

        let hardware = next_hardware(&mut stream).await;
        assert!(hardware.locked);

        assert!(
            is_quiet(&mut stream, Duration::from_millis(400)).await,
            "nothing changed, so nothing is sent"
        );
    }

    /// A song playing is reported as the websocket reports it: playing, on
    /// its clock, with the section its loop is set on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_playing_song_is_in_the_playback_with_its_section() {
        let (_dir, player) = project().await;
        let _hub = player.watch_hub().start(player.clone());
        let mut stream = open(&player, &[topic(WatchTopic::Playback)]).await;
        next_playback(&mut stream).await;

        player.play().await.expect("play");
        player.loop_section("a").await.expect("loop section a");
        let playing = loop {
            let playback = next_playback(&mut stream).await;
            if playback.is_playing && playback.active_section.is_some() {
                break playback;
            }
        };
        assert_eq!(playing.song_name, "S");
        let active = playing.active_section.expect("a section");
        assert_eq!(active.name, "a");
        assert_eq!(active.start, Some(millis(0)));
        assert_eq!(active.end, Some(millis(2_000)));
        assert!(playing.elapsed.is_some());
        assert_eq!(
            playing.next.and_then(|next| next.song).as_deref(),
            Some("T")
        );
        let _ = player.stop().await;
    }

    /// The topics a request names are all a stream sends; the playback is
    /// not sent to one that did not ask.
    #[tokio::test(flavor = "multi_thread")]
    async fn only_the_asked_for_topics_arrive() {
        let (_dir, player) = project().await;
        let sender = player.watch_hub().sender();
        let mut stream = open(
            &player,
            &[topic(WatchTopic::ConfigChanged), topic(WatchTopic::Reload)],
        )
        .await;
        assert!(
            is_quiet(&mut stream, Duration::from_millis(200)).await,
            "no playback or hardware snapshot was asked for"
        );

        // The stream's task subscribes as it starts; wait for it.
        crate::testutil::eventually(|| sender.receiver_count() == 1, "never subscribed");
        sender
            .send(SocketMessage::Logs {
                lines: vec![wire::LogLine {
                    level: "INFO".to_string(),
                    target: "t".to_string(),
                    message: "not asked for".to_string(),
                }],
            })
            .expect("a reader");
        sender
            .send(SocketMessage::Waveform(wire::Waveform::new(
                "S".to_string(),
                [("t".to_string(), vec![0.5])],
            )))
            .expect("a reader");
        sender.send(SocketMessage::ConfigChanged).expect("a reader");
        assert_eq!(
            next_event(&mut stream).await,
            Event::ConfigChanged(proto::ConfigChanged {})
        );
        sender
            .send(SocketMessage::Reload(wire::ShowReload::Ok))
            .expect("a reader");
        assert_eq!(
            next_event(&mut stream).await,
            Event::Reload(proto::ShowReload { error: None })
        );
    }

    /// An empty request, and one with only values that are no topic, ask
    /// for every topic.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_or_unknown_request_asks_for_everything() {
        let (_dir, player) = project().await;
        for topics in [
            ALL,
            &[topic(WatchTopic::Unspecified)][..],
            &[99, topic(WatchTopic::Unspecified)][..],
        ] {
            let mut stream = open(&player, topics).await;
            next_playback(&mut stream).await;
            next_hardware(&mut stream).await;
            // Every feed is read, not only the snapshots.
            assert_eq!(player.watch_hub().sender().receiver_count(), 1);
            drop(stream);
            crate::testutil::eventually(
                || player.watch_hub().sender().receiver_count() == 0,
                "the stream outlived its client",
            );
        }
        // An unknown value beside a real topic is ignored, not all.
        let mut stream = open(&player, &[99, topic(WatchTopic::Hardware)]).await;
        next_hardware(&mut stream).await;
        assert!(is_quiet(&mut stream, Duration::from_millis(200)).await);
    }

    /// A stream that asks for no feed of the hub does not hold the pollers
    /// awake, and one that asks for any does until it is dropped.
    #[tokio::test(flavor = "multi_thread")]
    async fn only_a_stream_that_wants_a_feed_counts_as_a_reader() {
        let (_dir, player) = project().await;
        let sender = player.watch_hub().sender();
        let mut hardware = open(&player, &[topic(WatchTopic::Hardware)]).await;
        next_hardware(&mut hardware).await;
        assert_eq!(sender.receiver_count(), 0);

        let mut playback = open(&player, &[topic(WatchTopic::Playback)]).await;
        next_playback(&mut playback).await;
        assert_eq!(sender.receiver_count(), 1);
        drop(playback);
        crate::testutil::eventually(|| sender.receiver_count() == 0, "never let go");
    }

    /// The cached waveform of the song the playlist is on is sent as the
    /// stream opens, as the websocket sends it; none cached, none sent.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_cached_waveform_comes_when_the_stream_opens() {
        let (_dir, player) = project().await;
        let mut stream = open(&player, &[topic(WatchTopic::Waveform)]).await;
        assert!(
            is_quiet(&mut stream, Duration::from_millis(200)).await,
            "nothing is cached yet"
        );

        player.watch_hub().waveform_cache().lock().insert(
            "S".to_string(),
            vec![("t".to_string(), vec![0.5, 0.8, 0.25])],
        );
        let mut stream = open(&player, &[topic(WatchTopic::Waveform)]).await;
        assert_eq!(
            next_event(&mut stream).await,
            Event::Waveform(proto::Waveform {
                song_name: "S".to_string(),
                tracks: vec![proto::WaveformTrack {
                    name: "t".to_string(),
                    peaks: vec![0.5, 0.8, 0.25],
                }],
            })
        );
    }

    /// A song the playlist moves to brings its waveform, computed from its
    /// files, and the hub keeps it for the next stream.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_song_brings_its_waveform() {
        let (_dir, player) = project().await;
        let _hub = player.watch_hub().start(player.clone());
        let mut stream = open(&player, &[topic(WatchTopic::Waveform)]).await;
        let first = loop {
            if let Event::Waveform(waveform) = next_event(&mut stream).await {
                if waveform.song_name == "S" {
                    break waveform;
                }
            }
        };
        assert_eq!(first.tracks[0].name, "t");
        assert!(!first.tracks[0].peaks.is_empty());

        player.next().await.expect("next");
        loop {
            if let Event::Waveform(waveform) = next_event(&mut stream).await {
                if waveform.song_name == "T" {
                    assert_eq!(waveform.tracks[0].name, "t");
                    break;
                }
            }
        }
        assert!(player.watch_hub().waveform_cache().lock().contains_key("T"));
    }

    /// The log lines the poller reads arrive as lines, split as the
    /// websocket's are.
    #[tokio::test(flavor = "multi_thread")]
    async fn log_lines_arrive_split_into_level_target_and_message() {
        let (_dir, player) = project().await;
        let _ = std::panic::catch_unwind(|| {
            crate::tui::logging::init_tui_logging(100);
        });
        let buffer = crate::tui::logging::get_log_buffer().expect("a log buffer");
        let _hub = player.watch_hub().start(player.clone());
        let mut stream = open(&player, &[topic(WatchTopic::Logs)]).await;
        // Subscribed before the line is written: the poller sends what is
        // new since its last read.
        crate::testutil::eventually(
            || player.watch_hub().sender().receiver_count() == 1,
            "never subscribed",
        );
        buffer
            .lock()
            .push_back("WARN watch::test: a line for Watch".to_string());
        loop {
            let Event::Logs(logs) = next_event(&mut stream).await else {
                panic!("only logs were asked for");
            };
            if let Some(line) = logs.lines.iter().find(|l| l.message == "a line for Watch") {
                assert_eq!(line.level, "WARN");
                assert_eq!(line.target, "watch::test");
                break;
            }
        }
    }

    /// A change to the config store is told on the stream.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_config_mutation_is_told_as_config_changed() {
        let (dir, player) = project().await;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: songs\n").expect("config");
        let store = Arc::new(crate::config::ConfigStore::new(
            crate::config::Player::deserialize(&path).expect("config"),
            path,
        ));
        player.set_config_store(store.clone());
        let _hub = player.watch_hub().start(player.clone());
        let mut stream = open(&player, &[topic(WatchTopic::ConfigChanged)]).await;
        crate::testutil::eventually(
            || player.watch_hub().sender().receiver_count() == 1,
            "never subscribed",
        );
        // The config watcher subscribes to the store as it starts.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let snapshot = store.read().await.expect("read");
        store
            .update_midi(
                Some(crate::config::Midi::new("test-midi", None)),
                &snapshot.checksum,
            )
            .await
            .expect("update");
        assert_eq!(
            next_event(&mut stream).await,
            Event::ConfigChanged(proto::ConfigChanged {})
        );
    }

    /// A reload of a light show is told with its error, when it has one.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_show_reload_is_told_with_its_error() {
        let (_dir, player) = project().await;
        let sender = player.watch_hub().sender();
        let mut stream = open(&player, &[topic(WatchTopic::Reload)]).await;
        crate::testutil::eventually(|| sender.receiver_count() == 1, "never subscribed");

        sender
            .send(SocketMessage::Reload(wire::ShowReload::Error {
                error: "no such effect".to_string(),
            }))
            .expect("a reader");
        sender
            .send(SocketMessage::Reload(wire::ShowReload::Ok))
            .expect("a reader");
        assert_eq!(
            next_event(&mut stream).await,
            Event::Reload(proto::ShowReload {
                error: Some("no such effect".to_string())
            })
        );
        assert_eq!(
            next_event(&mut stream).await,
            Event::Reload(proto::ShowReload { error: None })
        );
    }

    /// The hardware status says what the status page does: the build, the
    /// subsystems, the controllers and the lock.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_hardware_carries_the_build_the_subsystems_and_the_controllers() {
        let (_dir, player) = project().await;
        player.install_controller(crate::controller::Controller::new(
            vec![crate::config::Controller::Grpc(
                crate::config::GrpcController::new(0),
            )],
            player.clone(),
        ));
        let mut stream = open(&player, &[topic(WatchTopic::Hardware)]).await;
        let hardware = next_hardware(&mut stream).await;

        let build = hardware.build.expect("a build");
        assert_eq!(build.version, crate::build_info::VERSION);
        assert_eq!(build.git_hash, crate::build_info::GIT_HASH);
        assert_eq!(build.build_time, crate::build_info::BUILD_TIME);
        assert_eq!(build.ui_build, crate::webui::server::ui_build());
        let snapshot = hardware.hardware.expect("hardware");
        assert!(snapshot.init_done);
        assert_eq!(snapshot.audio.expect("audio").status, "connected");
        assert_eq!(snapshot.midi.expect("midi").status, "not_connected");
        assert_eq!(snapshot.dmx.expect("dmx").status, "not_connected");
        assert_eq!(
            snapshot.lighting_venue.expect("a venue status").status,
            "none"
        );
        assert_eq!(hardware.controllers.len(), 1);
        assert_eq!(hardware.controllers[0].kind, "grpc");
        assert_eq!(
            hardware.controllers,
            player
                .controller_statuses()
                .iter()
                .map(controller_to_proto)
                .collect::<Vec<_>>()
        );
        assert!(hardware.locked);
    }

    /// A change to the hardware status is sent, within a second or so; with
    /// no change nothing is.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_hardware_is_sent_again_when_it_changes_and_not_when_it_does_not() {
        let (_dir, player) = project().await;
        let mut stream = open(&player, &[topic(WatchTopic::Hardware)]).await;
        assert!(next_hardware(&mut stream).await.locked);
        assert!(
            is_quiet(
                &mut stream,
                HARDWARE_INTERVAL * 2 + Duration::from_millis(300)
            )
            .await,
            "an unchanged status was sent again"
        );

        player.set_locked(false);
        let changed = next_hardware(&mut stream).await;
        assert!(!changed.locked);
        assert!(
            is_quiet(
                &mut stream,
                HARDWARE_INTERVAL * 2 + Duration::from_millis(300)
            )
            .await,
            "an unchanged status was sent again"
        );
    }

    /// A reader that falls far behind the hub skips what it missed and
    /// goes on: the stream does not end, and the newest is not lost.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_reader_skips_what_it_missed_and_carries_on() {
        let (_dir, player) = project().await;
        let sender = player.watch_hub().sender();
        let mut stream = open(&player, &[topic(WatchTopic::Logs)]).await;
        crate::testutil::eventually(|| sender.receiver_count() == 1, "never subscribed");

        // Far more than the stream's buffer and the hub's capacity, with
        // nobody reading.
        let flood = 2_000;
        for i in 0..flood {
            let _ = sender.send(SocketMessage::Logs {
                lines: vec![wire::LogLine {
                    level: "INFO".to_string(),
                    target: "flood".to_string(),
                    message: i.to_string(),
                }],
            });
        }
        let last = (flood - 1).to_string();
        let mut received = 0;
        loop {
            let Event::Logs(logs) = next_event(&mut stream).await else {
                panic!("only logs were asked for");
            };
            received += 1;
            if logs.lines[0].message == last {
                break;
            }
        }
        assert!(received < flood, "{received} of {flood} were kept");
    }

    #[test]
    fn a_playback_is_mapped_field_for_field() {
        let playback = wire::Playback {
            is_playing: true,
            elapsed_ms: 12_345,
            song_name: "S".to_string(),
            song_duration_ms: 200_001,
            playlist_name: "set".to_string(),
            playlist_position: 3,
            playlist_off_list: true,
            playlist_at_first: false,
            playlist_at_last: true,
            playlist_songs: vec!["S".to_string(), "T".to_string()],
            tracks: vec![wire::TrackInfo {
                name: "t".to_string(),
                output_channels: vec![1, 2],
                gain_db: -6.5,
                muted: true,
            }],
            available_playlists: vec!["set".to_string(), "other".to_string()],
            persisted_playlist_name: "saved".to_string(),
            locked: true,
            hardware_reload_started: u64::MAX,
            hardware_reload_refused: vec![wire::RefusedReload {
                generation: 7,
                error: "busy".to_string(),
            }],
            process_id: "18446744073709551615".to_string(),
            beat_grid: Some(wire::PlaybackGrid {
                grid: crate::audio::beat_grid::BeatGrid {
                    beats: vec![0.5, 1.0, 1.5],
                    measure_starts: vec![0, 2],
                },
                accent_levels: Some(vec![3, 1, 0]),
            }),
            available_sections: vec![crate::config::Section {
                name: "verse".to_string(),
                start_measure: 1,
                end_measure: 5,
                start_beat: Some(2.5),
                end_beat: Some(1.0),
                color: Some("#ff0000".to_string()),
            }],
            active_section: Some(wire::ActiveSection {
                name: "verse".to_string(),
                start_ms: 1_500,
                end_ms: 9_250,
            }),
            looping: true,
            reactive_loop_state: wire::LoopState::LoopArmed {
                section_name: "verse".to_string(),
            },
            pending_start_ms: Some(4_000),
            pilot_hints: vec![wire::ResolvedPilotHint {
                label: "cue".to_string(),
                at_ms: 100,
                start_ms: 200,
                end_ms: 300,
                has_audio: true,
            }],
            tempo: Some(wire::PlaybackTempo {
                bpm: 97.5,
                time_signature: (7, 8),
            }),
            auto_play: Some(wire::AutoPlay {
                song: "T".to_string(),
                starts_in_ms: 1_999,
            }),
            ending: Some(wire::Ending {
                song: "S".to_string(),
                remaining_ms: 2_001,
            }),
            ambient: Some(wire::AmbientState::Read(wire::AmbientReading {
                mode: crate::dmx::engine::AmbientMode::On,
                source: Some(crate::playlist::AmbientSource::Playlist),
                show: Some("glow.light".to_string()),
                state: "playing".to_string(),
            })),
            next: Some(wire::NextUp {
                song: Some("T".to_string()),
                ..Default::default()
            }),
        };
        let mapped = playback_to_proto(&playback);

        assert!(mapped.is_playing);
        assert_eq!(mapped.elapsed, Some(millis(12_345)));
        assert_eq!(mapped.song_duration, Some(millis(200_001)));
        assert_eq!(mapped.playlist_name, "set");
        assert_eq!(mapped.playlist_position, 3);
        assert!(mapped.playlist_off_list);
        assert!(!mapped.playlist_at_first);
        assert!(mapped.playlist_at_last);
        assert_eq!(mapped.available_playlists, ["set", "other"]);
        assert_eq!(mapped.persisted_playlist_name, "saved");
        assert!(mapped.locked);
        assert_eq!(mapped.tracks[0].gain_db, -6.5);
        assert!(mapped.tracks[0].muted);
        assert_eq!(mapped.hardware_reload_started, u64::MAX);
        assert_eq!(
            mapped.hardware_reload_refused,
            [proto::RefusedReload {
                generation: 7,
                error: "busy".to_string()
            }]
        );
        assert_eq!(mapped.process_id, "18446744073709551615");
        assert_eq!(
            mapped.beat_grid,
            Some(proto::PlaybackGrid {
                grid: Some(proto::BeatGrid {
                    beats: vec![0.5, 1.0, 1.5],
                    measure_starts: vec![0, 2],
                }),
                accent_levels: Some(vec![3, 1, 0]),
            })
        );
        assert_eq!(
            mapped.available_sections,
            [proto::Section {
                name: "verse".to_string(),
                start_measure: 1,
                end_measure: 5,
                start_beat: Some(2.5),
                end_beat: Some(1.0),
                color: Some("#ff0000".to_string()),
            }]
        );
        assert_eq!(
            mapped.active_section,
            Some(proto::ActiveSection {
                name: "verse".to_string(),
                start: Some(millis(1_500)),
                end: Some(millis(9_250)),
            })
        );
        assert!(mapped.looping);
        assert_eq!(
            mapped.reactive_loop_state,
            Some(proto::LoopState {
                state: "loop_armed".to_string(),
                section_name: Some("verse".to_string()),
            })
        );
        assert_eq!(mapped.pending_start, Some(millis(4_000)));
        assert_eq!(
            mapped.pilot_hints,
            [proto::PilotHint {
                label: "cue".to_string(),
                at: Some(millis(100)),
                start: Some(millis(200)),
                end: Some(millis(300)),
                has_audio: true,
            }]
        );
        assert_eq!(
            mapped.tempo,
            Some(proto::PlaybackTempo {
                bpm: 97.5,
                numerator: 7,
                denominator: 8
            })
        );
        assert_eq!(
            mapped.auto_play,
            Some(proto::AutoPlayStatus {
                song: "T".to_string(),
                starts_in: Some(millis(1_999)),
            })
        );
        assert_eq!(
            mapped.ending,
            Some(proto::PlaybackEnding {
                song: "S".to_string(),
                remaining: Some(millis(2_001)),
            })
        );
        assert_eq!(
            mapped.ambient,
            Some(proto::AmbientStatus {
                mode: "on".to_string(),
                state: "playing".to_string(),
                show: Some("glow.light".to_string()),
                source: Some("playlist".to_string()),
                available: None,
                reason: None,
            })
        );
        assert_eq!(
            mapped.next,
            Some(proto::NextUp {
                song: Some("T".to_string()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn what_is_absent_stays_absent() {
        let mut playback = wire::Playback {
            is_playing: false,
            elapsed_ms: 0,
            song_name: String::new(),
            song_duration_ms: 0,
            playlist_name: String::new(),
            playlist_position: 0,
            playlist_off_list: false,
            playlist_at_first: true,
            playlist_at_last: true,
            playlist_songs: vec![],
            tracks: vec![],
            available_playlists: vec![],
            persisted_playlist_name: String::new(),
            locked: false,
            hardware_reload_started: 0,
            hardware_reload_refused: vec![],
            process_id: String::new(),
            beat_grid: None,
            available_sections: vec![],
            active_section: None,
            looping: false,
            reactive_loop_state: wire::LoopState::Idle,
            pending_start_ms: None,
            pilot_hints: vec![],
            tempo: None,
            auto_play: None,
            ending: None,
            ambient: None,
            next: None,
        };
        let mapped = playback_to_proto(&playback);
        assert_eq!(mapped.beat_grid, None);
        assert_eq!(mapped.active_section, None);
        assert_eq!(mapped.pending_start, None);
        assert_eq!(mapped.tempo, None);
        assert_eq!(mapped.auto_play, None);
        assert_eq!(mapped.ending, None);
        assert_eq!(mapped.ambient, None, "no DMX engine");
        assert_eq!(mapped.next, None, "no current song");
        // Zero is a time, not an absence.
        assert_eq!(mapped.elapsed, Some(millis(0)));

        // A beat grid with no metronome has no accent levels, which is not
        // an empty list of them.
        playback.beat_grid = Some(wire::PlaybackGrid {
            grid: crate::audio::beat_grid::BeatGrid {
                beats: vec![],
                measure_starts: vec![],
            },
            accent_levels: None,
        });
        assert_eq!(
            playback_to_proto(&playback)
                .beat_grid
                .unwrap()
                .accent_levels,
            None
        );
        playback.beat_grid.as_mut().unwrap().accent_levels = Some(vec![]);
        assert_eq!(
            playback_to_proto(&playback)
                .beat_grid
                .unwrap()
                .accent_levels,
            Some(vec![])
        );
    }

    /// An ambient read that did not answer is not the absence of ambient.
    #[test]
    fn an_ambient_read_that_did_not_answer_says_so() {
        let mut playback = a_playback();
        playback.ambient = Some(wire::AmbientState::Unread(wire::AmbientUnread {
            available: false,
            reason: "the DMX engine did not answer".to_string(),
        }));
        assert_eq!(
            playback_to_proto(&playback).ambient,
            Some(proto::AmbientStatus {
                mode: String::new(),
                state: String::new(),
                show: None,
                source: None,
                available: Some(false),
                reason: Some("the DMX engine did not answer".to_string()),
            })
        );
        // Read, with no show and no source.
        playback.ambient = Some(wire::AmbientState::Read(wire::AmbientReading {
            mode: crate::dmx::engine::AmbientMode::Auto,
            source: None,
            show: None,
            state: "off".to_string(),
        }));
        let ambient = playback_to_proto(&playback).ambient.unwrap();
        assert_eq!(ambient.mode, "auto");
        assert_eq!((ambient.show, ambient.source), (None, None));
        assert_eq!(ambient.available, None);
    }

    fn a_playback() -> wire::Playback {
        wire::Playback {
            is_playing: false,
            elapsed_ms: 0,
            song_name: String::new(),
            song_duration_ms: 0,
            playlist_name: String::new(),
            playlist_position: 0,
            playlist_off_list: false,
            playlist_at_first: true,
            playlist_at_last: true,
            playlist_songs: vec![],
            tracks: vec![],
            available_playlists: vec![],
            persisted_playlist_name: String::new(),
            locked: false,
            hardware_reload_started: 0,
            hardware_reload_refused: vec![],
            process_id: String::new(),
            beat_grid: None,
            available_sections: vec![],
            active_section: None,
            looping: false,
            reactive_loop_state: wire::LoopState::Idle,
            pending_start_ms: None,
            pilot_hints: vec![],
            tempo: None,
            auto_play: None,
            ending: None,
            ambient: None,
            next: None,
        }
    }

    /// The websocket's three states of a field a case may decide: absent,
    /// present and null, present with a value.
    #[test]
    fn next_keeps_absent_apart_from_present_and_null() {
        let fade = FadeStatus {
            length: "3s".to_string(),
            curve: FadeCurve::Linear,
            exclude: Some(vec![]),
            lights: Some("1s".to_string()),
        };
        // Absent: the case does not decide them.
        let undecided = next_to_proto(&wire::NextUp {
            song: Some("T".to_string()),
            ..Default::default()
        });
        assert_eq!(
            (undecided.fade_out, undecided.fade_in, undecided.auto_play),
            (None, None, None)
        );

        // Present and null: decided to be none.
        let none = next_to_proto(&wire::NextUp {
            song: Some("T".to_string()),
            breaks_loop: Some(false),
            fade_out: Some(None),
            fade_in: Some(None),
            auto_play_ms: Some(None),
            ..Default::default()
        });
        assert_eq!(none.breaks_loop, Some(false));
        assert_eq!(none.fade_out, Some(proto::NextFade { fade: None }));
        assert_eq!(none.fade_in, Some(proto::NextFade { fade: None }));
        assert_eq!(none.auto_play, Some(proto::NextAutoPlay { gap_ms: None }));

        // Present with a value, at the end of the playlist.
        let some = next_to_proto(&wire::NextUp {
            song: None,
            ends_loop: Some(true),
            fade_out: Some(Some(fade)),
            fade_in: Some(None),
            auto_play_ms: Some(Some(2_500.5)),
            ..Default::default()
        });
        assert_eq!(some.song, None);
        assert_eq!(some.ends_loop, Some(true));
        assert_eq!(
            some.fade_out,
            Some(proto::NextFade {
                fade: Some(proto::Fade {
                    length: "3s".to_string(),
                    curve: Some("linear".to_string()),
                    exclude: Some(proto::TrackNames { names: vec![] }),
                    lights: Some("1s".to_string()),
                })
            })
        );
        assert_eq!(
            some.auto_play,
            Some(proto::NextAutoPlay {
                gap_ms: Some(2_500.5)
            })
        );
        // The default exclusions and lights are absent, not empty.
        let default_fade = fade_to_proto(&FadeStatus {
            length: "2s".to_string(),
            curve: FadeCurve::EqualPower,
            exclude: None,
            lights: None,
        });
        assert_eq!(default_fade.curve.as_deref(), Some("equal_power"));
        assert_eq!((default_fade.exclude, default_fade.lights), (None, None));
    }

    #[test]
    fn every_loop_state_is_named_with_its_section() {
        for (state, name, section) in [
            (wire::LoopState::Idle, "idle", None),
            (
                wire::LoopState::SectionOffered {
                    section_name: "a".to_string(),
                },
                "section_offered",
                Some("a"),
            ),
            (
                wire::LoopState::Looping {
                    section_name: "b".to_string(),
                },
                "looping",
                Some("b"),
            ),
            (
                wire::LoopState::BreakRequested {
                    section_name: "c".to_string(),
                },
                "break_requested",
                Some("c"),
            ),
        ] {
            let mapped = loop_state_to_proto(&state);
            assert_eq!(mapped.state, name);
            assert_eq!(mapped.section_name.as_deref(), section);
        }
    }

    #[test]
    fn the_hardware_is_mapped_field_for_field() {
        use crate::audio::output_status::OutputStatus;
        use crate::player::{LightingVenueStatus, SubsystemStatus, TestOutputStatus};
        let snapshot = HardwareStatusSnapshot {
            init_done: true,
            hostname: Some("stage-pi".to_string()),
            profile: Some("stage".to_string()),
            profile_file: Some("stage.yml".to_string()),
            audio: SubsystemStatus {
                status: "connected".to_string(),
                name: Some("MAT".to_string()),
                error: Some("maps tracks past the device".to_string()),
            },
            audio_output: Some(OutputHealth {
                status: OutputStatus::Recovering,
                callback_alive: false,
                writing_signal: true,
                since_last_callback_ms: Some(1_500),
                since_last_signal_ms: None,
                callbacks: 99,
                recoveries: 2,
                underruns: 3,
                last_error: Some("xrun".to_string()),
            }),
            midi: SubsystemStatus {
                status: "failed".to_string(),
                name: None,
                error: Some("no device".to_string()),
            },
            dmx: SubsystemStatus {
                status: "initializing".to_string(),
                name: None,
                error: None,
            },
            trigger: SubsystemStatus {
                status: "not_connected".to_string(),
                name: None,
                error: None,
            },
            lighting_venue: LightingVenueStatus {
                name: Some("barn".to_string()),
                status: "failed".to_string(),
                error: Some("par 3: no mode".to_string()),
            },
            test_output: Some(TestOutputStatus {
                universe: 2,
                address: 17,
                footprint: 9,
                fixture_type: "par".to_string(),
                mode: None,
                expires_in_secs: 4.25,
            }),
        };
        let mapped = hardware_to_proto(&snapshot);
        assert!(mapped.init_done);
        assert_eq!(mapped.hostname.as_deref(), Some("stage-pi"));
        assert_eq!(mapped.profile.as_deref(), Some("stage"));
        assert_eq!(mapped.profile_file.as_deref(), Some("stage.yml"));
        assert_eq!(
            mapped.audio,
            Some(proto::SubsystemStatus {
                status: "connected".to_string(),
                name: Some("MAT".to_string()),
                error: Some("maps tracks past the device".to_string()),
            })
        );
        assert_eq!(
            mapped.audio_output,
            Some(proto::AudioOutputHealth {
                status: "recovering".to_string(),
                callback_alive: false,
                writing_signal: true,
                since_last_callback: Some(millis(1_500)),
                since_last_signal: None,
                callbacks: 99,
                recoveries: 2,
                underruns: 3,
                last_error: Some("xrun".to_string()),
            })
        );
        assert_eq!(mapped.midi.unwrap().error.as_deref(), Some("no device"));
        assert_eq!(mapped.dmx.unwrap().status, "initializing");
        assert_eq!(mapped.trigger.unwrap().status, "not_connected");
        assert_eq!(
            mapped.lighting_venue,
            Some(proto::LightingVenueStatus {
                name: Some("barn".to_string()),
                status: "failed".to_string(),
                error: Some("par 3: no mode".to_string()),
            })
        );
        assert_eq!(
            mapped.test_output,
            Some(proto::TestOutputStatus {
                universe: 2,
                address: 17,
                footprint: 9,
                fixture_type: "par".to_string(),
                mode: None,
                expires_in_secs: 4.25,
            })
        );
        assert_eq!(
            controller_to_proto(&ControllerSnapshot {
                kind: "osc".to_string(),
                status: "error".to_string(),
                detail: Some("port 9000".to_string()),
                error: Some("in use".to_string()),
            }),
            proto::ControllerStatus {
                kind: "osc".to_string(),
                status: "error".to_string(),
                detail: Some("port 9000".to_string()),
                error: Some("in use".to_string()),
            }
        );
    }

    /// The waveform's peaks are the player's f32s, whatever the websocket
    /// widened them to.
    #[test]
    fn a_waveforms_peaks_come_back_as_the_players_floats() {
        let peaks = [0.1_f32, 0.8, 1.0 / 3.0, f32::MIN_POSITIVE];
        let message = SocketMessage::Waveform(wire::Waveform::new(
            "S".to_string(),
            [("t".to_string(), peaks.to_vec())],
        ));
        let Event::Waveform(waveform) = message_to_proto(&message) else {
            panic!("a waveform");
        };
        assert_eq!(waveform.tracks[0].peaks, peaks);
    }

    #[test]
    fn a_gap_in_milliseconds_is_the_duration_it_names() {
        assert_eq!(
            millis(2_001),
            ProtoDuration {
                seconds: 2,
                nanos: 1_000_000
            }
        );
        assert_eq!(
            millis(u64::from(u32::MAX) * 1000 + 7),
            ProtoDuration {
                seconds: i64::from(u32::MAX),
                nanos: 7_000_000
            }
        );
    }
}
