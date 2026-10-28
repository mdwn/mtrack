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

//! Ambient light: the show the rig plays while no song is.
//!
//! The player says when it goes idle ([`Engine::idle_ambient`]); once the
//! delay is up the effects loop fades from whatever is lit into the ambient
//! show, on the wall clock, and keeps its time moving. A song with lights of
//! its own fades out of it ([`Engine::leave_ambient`]); a song without
//! leaves it playing.

use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use tracing::{error, info};

use super::Engine;
use crate::{
    config::ambient::{PlaylistAmbient, ResolvedAmbient},
    lighting::{parser::LightShow, timeline::LightingTimeline},
};

/// Ambient light ready to play: its settings and its parsed shows.
#[derive(Clone)]
pub(super) struct LoadedAmbient {
    settings: ResolvedAmbient,
    path: PathBuf,
    /// The file as read, to tell an edited show from the same one.
    content: String,
    shows: Vec<LightShow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Not playing, nor waiting to.
    Off,
    /// Waiting to fade in at `at`.
    Pending { at: Instant },
    /// Playing, its time counted from `started`, the start of this pass of
    /// a show `length` long (zero for one that does not loop).
    Playing { started: Instant, length: Duration },
}

/// What the engine's ambient light is doing.
pub(super) struct AmbientState {
    ambient: Option<LoadedAmbient>,
    phase: Phase,
}

impl Default for AmbientState {
    fn default() -> Self {
        AmbientState {
            ambient: None,
            phase: Phase::Off,
        }
    }
}

/// What ambient light is doing, for status.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientStatus {
    /// The show file.
    pub show: PathBuf,
    /// `pending` (waiting out its delay) or `playing`.
    pub state: &'static str,
}

impl Engine {
    /// The player has gone idle: play the ambient light the player's
    /// settings and `playlist`'s give, once its delay is up -- or at once,
    /// with `hold` false (at startup, or on a playlist switch). Ambient
    /// already playing plays on if nothing about it changed, and crosses
    /// into the new show if it did. With no ambient light, any playing fades
    /// out.
    pub fn idle_ambient(&self, playlist: Option<&PlaylistAmbient>, hold: bool) {
        let loaded = self.load_ambient(playlist);
        let mut state = self.ambient.lock();
        let now = Instant::now();
        match (state.phase, loaded) {
            (Phase::Playing { .. }, Some(loaded))
                if state.ambient.as_ref().is_some_and(|a| a.same_as(&loaded)) => {}
            (Phase::Playing { .. }, Some(loaded)) => {
                info!(show = %loaded.path.display(), "Ambient light changes show.");
                state.phase = Phase::Pending { at: now };
                state.ambient = Some(loaded);
            }
            (Phase::Playing { .. }, None) => {
                let fade = state
                    .ambient
                    .as_ref()
                    .map_or(Duration::ZERO, |a| a.settings.fade_out);
                info!("Ambient light fades out: none for this playlist.");
                self.begin_break_fade(fade);
                self.effect_engine.lock().set_tempo_map(None);
                state.phase = Phase::Off;
                state.ambient = None;
            }
            (_, Some(loaded)) => {
                let delay = if hold {
                    loaded.settings.delay
                } else {
                    Duration::ZERO
                };
                // Already waiting: the earlier of the two.
                let at = match state.phase {
                    Phase::Pending { at } => at.min(now + delay),
                    _ => now + delay,
                };
                state.phase = Phase::Pending { at };
                state.ambient = Some(loaded);
            }
            (_, None) => {
                state.phase = Phase::Off;
                state.ambient = None;
            }
        }
    }

    /// A song with lights of its own is starting: ambient light playing
    /// fades out under its show, over its `fade_out`, and any waiting is
    /// called off. Returns whether ambient was playing.
    pub(super) fn leave_ambient(&self) -> bool {
        let mut state = self.ambient.lock();
        let was_playing = matches!(state.phase, Phase::Playing { .. });
        if was_playing {
            let fade = state
                .ambient
                .as_ref()
                .map_or(Duration::ZERO, |a| a.settings.fade_out);
            self.begin_break_fade(fade);
        }
        state.phase = Phase::Off;
        was_playing
    }

    /// Whether ambient light is playing.
    pub(super) fn ambient_playing(&self) -> bool {
        matches!(self.ambient.lock().phase, Phase::Playing { .. })
    }

    /// What ambient light is doing, if anything.
    pub fn ambient_status(&self) -> Option<AmbientStatus> {
        let state = self.ambient.lock();
        let state_name = match state.phase {
            Phase::Off => return None,
            Phase::Pending { .. } => "pending",
            Phase::Playing { .. } => "playing",
        };
        state.ambient.as_ref().map(|a| AmbientStatus {
            show: a.path.clone(),
            state: state_name,
        })
    }

    /// One effects-loop tick of ambient light, before the song time is
    /// read: start it once its delay is up, and move its time on.
    pub(super) fn tick_ambient(&self) {
        let mut state = self.ambient.lock();
        let now = Instant::now();
        match state.phase {
            Phase::Pending { at } if now >= at => {
                let Some(ambient) = state.ambient.clone() else {
                    state.phase = Phase::Off;
                    return;
                };
                let length = self.start_ambient(&ambient);
                state.phase = Phase::Playing {
                    started: now,
                    length,
                };
            }
            Phase::Playing { started, length } => {
                // Ambient has no audio to wait for: its time is the delayed
                // time base's from the moment it starts.
                let mut elapsed = now.duration_since(started);
                if !length.is_zero() && elapsed >= length {
                    // The show is over: it starts again, as a looping
                    // song's does, its first cues firing before this
                    // frame renders.
                    let passes = (elapsed.as_nanos() / length.as_nanos()) as u32;
                    let started = started + length * passes;
                    elapsed = now.duration_since(started);
                    state.phase = Phase::Playing { started, length };
                    self.update_song_time(elapsed + self.playback_delay);
                    self.start_lighting_timeline_at(elapsed + self.playback_delay);
                } else {
                    self.update_song_time(elapsed + self.playback_delay);
                }
            }
            _ => {}
        }
    }

    /// Fades from whatever is lit into `ambient`'s show, over its `fade_in`.
    /// Returns how long one pass of the show lasts: when its last effect
    /// ends, after which it starts again.
    fn start_ambient(&self, ambient: &LoadedAmbient) -> Duration {
        info!(show = %ambient.path.display(), "Ambient light fades in.");
        self.begin_break_fade(ambient.settings.fade_in);
        // Whatever the last song left -- its file watcher, its MIDI DMX, its
        // song-time writers -- is done with.
        self.watcher_handle.lock().take();
        self.midi_dmx_playbacks.lock().clear();
        self.begin_playback_generation();
        *self.timeline_cancel_handle.lock() = None;
        self.timeline_finished.store(true, Ordering::Relaxed);

        let timeline = LightingTimeline::new(ambient.shows.clone());
        let length = timeline.show_end();
        self.effect_engine
            .lock()
            .set_tempo_map(timeline.tempo_map().cloned());
        *self.current_song_timeline.lock() = Some(timeline);
        self.update_song_time(self.playback_delay);
        self.start_lighting_timeline_at(self.playback_delay);
        length
    }

    /// Resolves and reads the ambient light for `playlist`, `None` when there
    /// is none or it does not load (logged).
    fn load_ambient(&self, playlist: Option<&PlaylistAmbient>) -> Option<LoadedAmbient> {
        let player = self.lighting_config.as_ref().and_then(|l| l.ambient());
        let settings = match crate::config::ambient::resolve(player, playlist) {
            Ok(settings) => settings?,
            Err(e) => {
                error!(err = %e, "Ambient light is misconfigured; none will play.");
                return None;
            }
        };
        let path = resolve_show_path(self.base_path.as_deref(), &settings.show);
        match self.read_ambient_shows(&path) {
            Ok((content, shows)) => Some(LoadedAmbient {
                settings,
                path,
                content,
                shows,
            }),
            Err(e) => {
                error!(show = %path.display(), err = %e, "Ambient light failed to load; none will play.");
                None
            }
        }
    }

    fn read_ambient_shows(&self, path: &Path) -> Result<(String, Vec<LightShow>), String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let shows = crate::lighting::parser::parse_light_file(&content, Some(path), None)
            .map_err(|e| e.to_string())?;
        if shows.is_empty() {
            return Err("the file has no show".to_string());
        }
        if let Some(ref lighting_config) = self.lighting_config {
            crate::lighting::validation::validate_light_shows(&shows, Some(lighting_config))
                .map_err(|e| e.to_string())?;
        }
        let mut shows: Vec<_> = shows.into_iter().collect();
        shows.sort_by(|a, b| a.0.cmp(&b.0));
        Ok((content, shows.into_iter().map(|(_, show)| show).collect()))
    }
}

impl LoadedAmbient {
    /// The same settings and the same shows: nothing to cross into.
    fn same_as(&self, other: &LoadedAmbient) -> bool {
        self.settings == other.settings && self.path == other.path && self.content == other.content
    }
}

/// A show path as written, relative to the project unless absolute.
fn resolve_show_path(base: Option<&Path>, show: &str) -> PathBuf {
    let path = Path::new(show);
    match base {
        Some(base) if path.is_relative() => base.join(path),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, sync::Arc, thread::sleep};

    use super::*;
    use crate::config;
    use crate::dmx::ola_client::OlaClientFactory;

    /// An engine whose lighting configuration plays `show` (written to a
    /// file in the returned directory) as its ambient light, with the wash
    /// patched at channels 1-3 of universe 5.
    fn ambient_engine(
        show: &str,
        settings: (&str, &str, &str),
    ) -> Result<(tempfile::TempDir, Arc<Engine>), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("ambient.light"), show)?;
        let (delay, fade_in, fade_out) = settings;
        // The wash is named as a group, so the show validates.
        let groups = std::collections::HashMap::from([(
            "wash".to_string(),
            config::lighting::LogicalGroup::new("wash".to_string(), vec![]),
        )]);
        let lighting = config::Lighting::new(None, Some(groups), None).with_ambient(Some(
            config::ambient::Ambient::new(
                Some(dir.path().join("ambient.light").display().to_string()),
                Some(delay.to_string()),
                Some(fade_in.to_string()),
                Some(fade_out.to_string()),
            ),
        ));
        let engine = Engine::new(
            &config::Dmx::new(
                None,
                None,
                Some(9010),
                vec![config::Universe::new(5, "universe1".to_string())],
                None,
            ),
            Some(&lighting),
            // No project: no lighting system to resolve the wash group
            // against, so it passes through to the fixture.
            None,
            OlaClientFactory::create_mock_client(),
        )?;
        let mut channels = std::collections::HashMap::new();
        for (name, offset) in [("red", 1), ("green", 2), ("blue", 3)] {
            channels.insert(name.to_string(), offset);
        }
        engine
            .effect_engine
            .lock()
            .register_fixture(crate::lighting::effects::FixtureInfo::new(
                "wash".to_string(),
                5,
                1,
                "Generic".to_string(),
                channels,
                None,
            ));
        Ok((dir, Arc::new(engine)))
    }

    fn timeline(source: &str) -> LightingTimeline {
        let shows = crate::lighting::parser::parse_light_shows(source).unwrap();
        LightingTimeline::new(shows.into_values().collect())
    }

    /// Plays `source` as a song's show from the top, as `Engine::play` does.
    fn start_song(engine: &Engine, source: &str) {
        engine.leave_ambient();
        *engine.current_song_timeline.lock() = Some(timeline(source));
        engine.begin_playback_generation();
        engine.update_song_time(Duration::ZERO);
        engine.start_lighting_timeline_at(Duration::ZERO);
    }

    /// One effects-loop tick; the wash's (red, green, blue).
    fn tick(engine: &Engine) -> (f64, f64, f64) {
        engine.effects_loop_tick();
        let universe = engine.get_universe(5).unwrap();
        (
            universe.get_target_value(0),
            universe.get_target_value(1),
            universe.get_target_value(2),
        )
    }

    /// A song frame at song time `secs`.
    fn song_frame(engine: &Engine, secs: f64) -> (f64, f64, f64) {
        engine.update_song_time(Duration::from_secs_f64(secs));
        tick(engine)
    }

    const BLUE: &str =
        "show \"ambient\" {\n    @0.000\n    wash: static, color: \"blue\", duration: 60s\n}\n";
    const RED: &str =
        "show \"song\" {\n    @0.000\n    wash: static, color: \"red\", duration: 60s\n}\n";
    const GREEN: &str =
        "show \"song\" {\n    @0.000\n    wash: static, color: \"green\", duration: 60s\n}\n";

    #[test]
    fn ambient_fades_in_between_songs_and_out_under_the_next() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "1s", "400ms"))?;
        start_song(&engine, RED);
        assert_eq!(song_frame(&engine, 1.0), (255.0, 0.0, 0.0));

        // The song ends: ambient fades in from its last frame.
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        let first = tick(&engine);
        assert!(first.0 > 200.0, "starts from the song's frame: {first:?}");
        sleep(Duration::from_millis(300));
        let mid = tick(&engine);
        assert!(
            mid.0 > 20.0 && mid.2 > 20.0,
            "part way, some of each: {mid:?}"
        );
        sleep(Duration::from_millis(800));
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        assert_eq!(engine.ambient_status().unwrap().state, "playing");

        // The next song: ambient fades out under its show.
        start_song(&engine, GREEN);
        assert!(engine.ambient_status().is_none());
        let first = song_frame(&engine, 0.0);
        assert!(first.2 > 200.0, "starts from ambient: {first:?}");
        sleep(Duration::from_millis(450));
        assert_eq!(song_frame(&engine, 0.45), (0.0, 255.0, 0.0));
        Ok(())
    }

    #[test]
    fn the_delay_holds_the_last_frame() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("1s", "0s", "0s"))?;
        start_song(&engine, RED);
        song_frame(&engine, 1.0);
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        assert_eq!(engine.ambient_status().unwrap().state, "pending");
        assert_eq!(tick(&engine), (255.0, 0.0, 0.0));
        sleep(Duration::from_millis(1100));
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        Ok(())
    }

    #[test]
    fn startup_and_a_playlist_switch_skip_the_delay() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("10s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        Ok(())
    }

    #[test]
    fn ambient_moves_on_and_starts_again_at_its_end() -> Result<(), Box<dyn Error>> {
        let show = "show \"ambient\" {\n    @0.000\n    wash: static, color: \"blue\", duration: 500ms\n    \
                    @0.500\n    wash: static, color: \"green\", duration: 500ms\n}\n";
        let (_dir, engine) = ambient_engine(show, ("0s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        sleep(Duration::from_millis(700));
        assert_eq!(tick(&engine), (0.0, 255.0, 0.0), "its second cue");
        sleep(Duration::from_millis(500));
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0), "round again");
        Ok(())
    }

    #[test]
    fn no_frame_goes_dark_when_the_show_starts_again() -> Result<(), Box<dyn Error>> {
        let show = "show \"ambient\" {\n    @0.000\n    wash: static, color: \"blue\", duration: 200ms\n}\n";
        let (_dir, engine) = ambient_engine(show, ("0s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        let until = Instant::now() + Duration::from_millis(700);
        while Instant::now() < until {
            assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
            sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    #[test]
    fn a_playlist_can_change_the_show_or_turn_it_off() -> Result<(), Box<dyn Error>> {
        let (dir, engine) = ambient_engine(BLUE, ("0s", "0s", "300ms"))?;
        std::fs::write(dir.path().join("green.light"), GREEN)?;
        engine.idle_ambient(None, false);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));

        // The same again: it plays on rather than restart.
        engine.idle_ambient(None, false);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));

        // A playlist with its own show crosses into it.
        let own = config::ambient::PlaylistAmbient::On(config::ambient::Ambient::new(
            Some(dir.path().join("green.light").display().to_string()),
            None,
            None,
            None,
        ));
        engine.idle_ambient(Some(&own), false);
        assert_eq!(tick(&engine), (0.0, 255.0, 0.0));

        // One with none fades it out.
        engine.idle_ambient(Some(&config::ambient::PlaylistAmbient::Off), false);
        assert!(engine.ambient_status().is_none());
        let first = tick(&engine);
        assert!(first.1 > 200.0, "fades from green: {first:?}");
        sleep(Duration::from_millis(350));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));
        Ok(())
    }

    #[test]
    fn a_show_that_does_not_load_plays_nothing() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine("show \"ambient\" {", ("0s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        assert!(engine.ambient_status().is_none());
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));
        Ok(())
    }

    #[test]
    fn a_song_without_lights_leaves_ambient_playing() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));

        let song_config = config::Song::new(
            "No Light",
            None,
            None,
            None,
            None,
            None,
            vec![],
            std::collections::HashMap::new(),
            Vec::new(),
        );
        let song = Arc::new(crate::songs::Song::new(
            std::path::Path::new("/tmp"),
            &song_config,
        )?);
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = crate::clock::PlaybackClock::wall();
        clock.start();
        Engine::play(
            engine.clone(),
            song,
            crate::playsync::PlaybackSync {
                cancel_handle: crate::playsync::CancelHandle::new(),
                ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                clock,
                start_time: Duration::ZERO,
                loop_control: crate::playsync::LoopControl::new(),
            },
        )?;
        assert_eq!(engine.ambient_status().unwrap().state, "playing");
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        Ok(())
    }
}
