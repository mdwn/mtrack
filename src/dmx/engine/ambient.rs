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
//! show, on the wall clock. A song with lights of its own fades out of it
//! ([`Engine::leave_ambient`]); a song without leaves it playing.
//!
//! Ambient plays in an engine of its own, an overlay over the main one
//! crossed with its frame at the overlay's level. So it can hold over a song
//! whose show runs on underneath ([`AmbientMode::On`]), and fade back to
//! that show at the point it has reached.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tracing::{error, info};

use super::Engine;
use crate::{
    config::ambient::{PlaylistAmbient, ResolvedAmbient},
    lighting::{
        effects::{DmxCommand, EffectError},
        parser::LightShow,
        timeline::{LightingTimeline, TimelineUpdate},
        EffectEngine,
    },
};

/// What the operator asks of ambient light. Set at run time and not kept:
/// every start is [`AmbientMode::Auto`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AmbientMode {
    /// Between songs and under a song without lights, after its delay.
    #[default]
    Auto,
    /// Now, over whatever plays -- a song's own show included -- until the
    /// mode goes back to `Auto`.
    On,
    /// None: ambient playing fades out to dark. Songs light as ever.
    Off,
}

impl AmbientMode {
    /// The mode's name: `auto`, `on` or `off`.
    pub fn as_str(self) -> &'static str {
        match self {
            AmbientMode::Auto => "auto",
            AmbientMode::On => "on",
            AmbientMode::Off => "off",
        }
    }

    /// The mode named `name` (`auto`, `on` or `off`, any case).
    pub fn parse(name: &str) -> Option<AmbientMode> {
        match name.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(AmbientMode::Auto),
            "on" => Some(AmbientMode::On),
            "off" => Some(AmbientMode::Off),
            _ => None,
        }
    }
}

/// Ambient light ready to play: its settings and its parsed shows.
#[derive(Clone)]
pub(super) struct LoadedAmbient {
    settings: ResolvedAmbient,
    path: PathBuf,
    /// The file as read, to tell an edited show from the same one.
    content: String,
    shows: Vec<LightShow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    /// Not playing, nor waiting to (though it may still be fading out).
    #[default]
    Off,
    /// Waiting to fade in at `at`.
    Pending { at: Instant },
    /// Playing, or fading in.
    Playing,
}

/// What the engine's ambient light is doing.
#[derive(Default)]
pub(super) struct AmbientState {
    /// The ambient light the player's settings and the playlist's give.
    ambient: Option<LoadedAmbient>,
    phase: Phase,
    mode: AmbientMode,
    /// A song with lights of its own is playing (or paused) under ambient.
    song_lit: bool,
    /// Between songs, what ambient covers -- the last song's frame -- is
    /// gone, so ambient fading out leaves the stage dark.
    under_dark: bool,
}

/// What ambient light is doing, for status.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientStatus {
    /// The show file.
    pub show: PathBuf,
    /// `pending` (waiting out its delay) or `playing`.
    pub state: &'static str,
}

/// Ambient light's mode and what it is doing, for status: present whatever
/// the mode, so a surface can show it.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientReport {
    pub mode: AmbientMode,
    /// The show file, when there is one to play.
    pub show: Option<PathBuf>,
    /// `playing`, `pending` (waiting out its delay), `off`, or
    /// `unavailable` (on, with no show to play: none configured, or it does
    /// not load).
    pub state: &'static str,
}

/// An ambient level fading from `from` to `to` over `over`, from `started`.
#[derive(Clone, Copy, Debug)]
struct Level {
    from: f64,
    to: f64,
    started: Instant,
    over: Duration,
}

impl Level {
    /// The level at `now`, 0.0 to 1.0.
    fn at(&self, now: Instant) -> f64 {
        if self.over.is_zero() {
            return self.to;
        }
        let progress = (now.saturating_duration_since(self.started).as_secs_f64()
            / self.over.as_secs_f64())
        .min(1.0);
        self.from + (self.to - self.from) * progress
    }

    /// Fades on from where it is now to `to`, at the pace of a whole fade
    /// lasting `full`: a fade turned back part way takes as long to return
    /// as it had run.
    fn toward(&mut self, now: Instant, to: f64, full: Duration) {
        let from = self.at(now);
        *self = Level {
            from,
            to,
            started: now,
            over: full.mul_f64((to - from).abs()),
        };
    }
}

/// Ambient light playing over the main engine: its own engine and timeline
/// on the wall clock, crossed with what is under it at its level.
pub(super) struct Overlay {
    ambient: LoadedAmbient,
    engine: EffectEngine,
    timeline: LightingTimeline,
    /// The start of this pass of the show.
    started: Instant,
    /// How long one pass lasts (zero for a show that does not loop).
    length: Duration,
    level: Level,
    /// The ambient show this one replaced, still under it while it fades in.
    inner: Option<Box<Overlay>>,
}

/// What one overlay's timeline fired in a tick: a restart's state (the show
/// starting again) and the cues now due.
struct OverlayUpdate {
    restart: Option<TimelineUpdate>,
    update: TimelineUpdate,
}

impl Overlay {
    fn new(ambient: LoadedAmbient, mut engine: EffectEngine, now: Instant) -> Overlay {
        let mut timeline = LightingTimeline::new(ambient.shows.clone());
        let length = timeline.show_end();
        timeline.start();
        engine.set_tempo_map(timeline.tempo_map().cloned());
        let level = Level {
            from: 0.0,
            to: 1.0,
            started: now,
            over: ambient.settings.fade_in,
        };
        Overlay {
            ambient,
            engine,
            timeline,
            started: now,
            length,
            level,
            inner: None,
        }
    }

    /// Moves this overlay's time to `now` -- starting the show again at its
    /// end, as a looping song does -- and an inner one's, collecting what
    /// they fire: this one's first.
    fn advance(&mut self, now: Instant, out: &mut Vec<OverlayUpdate>) {
        let mut elapsed = now.saturating_duration_since(self.started);
        let mut restart = None;
        if !self.length.is_zero() && elapsed >= self.length {
            let passes = (elapsed.as_nanos() / self.length.as_nanos()) as u32;
            self.started += self.length * passes;
            elapsed = now.saturating_duration_since(self.started);
            // Its first cues fire before this frame renders.
            restart = Some(if elapsed.is_zero() {
                self.timeline.start();
                TimelineUpdate::default()
            } else {
                self.timeline.start_at(elapsed)
            });
        }
        let update = self.timeline.update(elapsed);
        out.push(OverlayUpdate { restart, update });
        if let Some(inner) = self.inner.as_mut() {
            inner.advance(now, out);
        }
    }

    /// Applies what [`Self::advance`] collected, its groups resolved, in
    /// the same order.
    fn apply(&mut self, updates: &mut impl Iterator<Item = OverlayUpdate>) {
        if let Some(OverlayUpdate { restart, update }) = updates.next() {
            if let Some(restart) = restart {
                self.engine.stop_all_effects();
                apply_update(&mut self.engine, restart);
            }
            apply_update(&mut self.engine, update);
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.apply(updates);
        }
    }

    /// This overlay's frame crossed over `under` (the commands below it and
    /// the engine showing them) at its level, with an inner overlay crossed
    /// in between; what the rig shows is left as this engine's shown state.
    /// The inner overlay goes once this one is all the way in.
    fn render_over(
        &mut self,
        dt: Duration,
        now: Instant,
        under: &[DmxCommand],
        under_engine: &EffectEngine,
    ) -> Result<Vec<DmxCommand>, EffectError> {
        let Overlay {
            engine,
            inner,
            level,
            started,
            ..
        } = self;
        let (base, base_engine): (Cow<[DmxCommand]>, &EffectEngine) = match inner.as_deref_mut() {
            Some(inner) => {
                let commands = inner.render_over(dt, now, under, under_engine)?;
                (Cow::Owned(commands), &inner.engine)
            }
            None => (Cow::Borrowed(under), under_engine),
        };
        let t = level.at(now);
        let own = engine
            .update(dt, Some(now.saturating_duration_since(*started)))?
            .to_vec();
        let blended = engine.crossfade_commands(&base, &own, t);
        let shown = engine.crossfade_states(base_engine, t);
        engine.set_shown(Some(shown));
        if t >= 1.0 {
            self.inner = None;
        }
        Ok(blended)
    }

    /// Faded all the way out: its last frame is rendered, and it can go.
    fn faded_out(&self, now: Instant) -> bool {
        self.level.to <= 0.0 && self.level.at(now) <= 0.0
    }

    /// Takes the main engine's rig, as this one's inner overlay does.
    fn adopt_rig(&mut self, main: &EffectEngine) {
        self.engine.adopt_rig(main);
        if let Some(inner) = self.inner.as_mut() {
            inner.adopt_rig(main);
        }
    }
}

/// Applies a timeline update, its groups resolved, to `engine`, as
/// [`Engine::apply_timeline_update`] does to the main one.
fn apply_update(engine: &mut EffectEngine, update: TimelineUpdate) {
    for cmd in &update.layer_commands {
        engine.apply_layer_command(cmd);
    }
    for sequence_name in &update.stop_sequences {
        engine.stop_sequence(sequence_name);
    }
    let mut with_elapsed: Vec<_> = update.effects_with_elapsed.into_values().collect();
    with_elapsed.sort_by_key(|(effect, _)| effect.cue_time.unwrap_or(Duration::ZERO));
    for (effect, elapsed) in with_elapsed {
        if let Err(e) = engine.start_effect_with_elapsed(effect, elapsed) {
            error!("Failed to start ambient effect with elapsed time: {}", e);
        }
    }
    let mut effects = update.effects;
    effects.sort_by_key(|e| if e.id.starts_with("seq_") { 0 } else { 1 });
    for effect in effects {
        if let Err(e) = engine.start_effect(effect) {
            error!("Failed to start ambient effect: {}", e);
        }
    }
}

impl Engine {
    /// The player has gone idle: play the ambient light the player's
    /// settings and `playlist`'s give, once its delay is up -- or at once,
    /// with `hold` false (at startup, or on a playlist switch). Ambient
    /// already playing plays on if nothing about it changed, and crosses
    /// into the new show if it did. With no ambient light, any playing fades
    /// out. Mode `On` skips the delay; mode `Off` plays nothing.
    pub fn idle_ambient(&self, playlist: Option<&PlaylistAmbient>, hold: bool) {
        let loaded = self.load_ambient(playlist);
        let mut state = self.ambient.lock();
        state.song_lit = false;
        let now = Instant::now();
        match (state.mode, state.phase, loaded) {
            (AmbientMode::Off, _, loaded) => {
                // Off is dark between songs: what the last song left fades
                // out rather than holding (#570). A loop break's fade going
                // out over it -- a graceful ending's, or a vamp's under the
                // song stopped -- goes on at its own length (#571), since it
                // crosses out over everything after it.
                if !state.under_dark {
                    let fade = loaded
                        .as_ref()
                        .map_or(crate::config::ambient::DEFAULT_AMBIENT_FADE_OUT, |a| {
                            a.settings.fade_out
                        });
                    self.darken_under(fade, true);
                    state.under_dark = true;
                }
                state.phase = Phase::Off;
                state.ambient = loaded;
            }
            (AmbientMode::On, _, Some(loaded)) | (_, Phase::Playing, Some(loaded)) => {
                // The same show plays on; another crosses in.
                self.raise_overlay(&loaded, now);
                state.phase = Phase::Playing;
                state.ambient = Some(loaded);
            }
            (_, Phase::Playing, None) | (AmbientMode::On, _, None) => {
                info!("Ambient light fades out: none for this playlist.");
                self.lower_overlay(&mut state, now);
                state.phase = Phase::Off;
                state.ambient = None;
            }
            (AmbientMode::Auto, _, Some(loaded)) => {
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
            (AmbientMode::Auto, _, None) => {
                state.phase = Phase::Off;
                state.ambient = None;
            }
        }
    }

    /// A stop, pause or seek cuts a loop break's fade short: the looping
    /// song's lights go the rest of the way out over ambient's `fade_out`,
    /// if that is sooner, rather than holding what they lit that nothing
    /// after them does (#572).
    pub fn cut_break_fade_lights(&self) {
        let fade = self
            .ambient
            .lock()
            .ambient
            .as_ref()
            .map_or(crate::config::ambient::DEFAULT_AMBIENT_FADE_OUT, |a| {
                a.settings.fade_out
            });
        self.hurry_break_fade(fade);
    }

    /// A song with lights of its own is starting: ambient light playing
    /// fades out under its show, over its `fade_out`, and any waiting is
    /// called off -- unless the mode is `On`, which holds it over the song.
    /// Returns whether ambient was playing.
    pub(super) fn leave_ambient(&self) -> bool {
        let mut state = self.ambient.lock();
        state.song_lit = true;
        state.under_dark = false;
        let was_playing = state.phase == Phase::Playing;
        if state.mode == AmbientMode::On {
            return was_playing;
        }
        if was_playing {
            self.lower_overlay(&mut state, Instant::now());
        }
        state.phase = Phase::Off;
        was_playing
    }

    /// Sets ambient light's mode. `On` fades ambient in now, over its
    /// `fade_in`, and holds it over any song; `Off` fades it out, over its
    /// `fade_out`, to dark between songs and to the song's show during one;
    /// `Auto` goes back to playing it between songs, crossing back to a
    /// song playing under it.
    pub fn set_ambient_mode(&self, mode: AmbientMode) {
        let mut state = self.ambient.lock();
        info!(mode = mode.as_str(), "Ambient light mode set.");
        state.mode = mode;
        let now = Instant::now();
        match mode {
            AmbientMode::On => {
                if let Some(ambient) = state.ambient.clone() {
                    self.raise_overlay(&ambient, now);
                    state.phase = Phase::Playing;
                }
            }
            AmbientMode::Off => {
                self.lower_overlay(&mut state, now);
                state.phase = Phase::Off;
            }
            AmbientMode::Auto if state.song_lit => {
                self.lower_overlay(&mut state, now);
                state.phase = Phase::Off;
            }
            AmbientMode::Auto => {
                // Between songs: ambient, as when the player goes idle.
                if state.phase == Phase::Off && state.ambient.is_some() {
                    state.phase = Phase::Pending { at: now };
                }
            }
        }
    }

    /// Ambient light's mode.
    pub fn ambient_mode(&self) -> AmbientMode {
        self.ambient.lock().mode
    }

    /// Whether ambient light is playing.
    pub(super) fn ambient_playing(&self) -> bool {
        self.ambient.lock().phase == Phase::Playing
    }

    /// What ambient light is doing, if anything.
    pub fn ambient_status(&self) -> Option<AmbientStatus> {
        let state = self.ambient.lock();
        let state_name = match state.phase {
            Phase::Off => return None,
            Phase::Pending { .. } => "pending",
            Phase::Playing => "playing",
        };
        state.ambient.as_ref().map(|a| AmbientStatus {
            show: a.path.clone(),
            state: state_name,
        })
    }

    /// Ambient light's mode and what it is doing.
    pub fn ambient_report(&self) -> AmbientReport {
        let state = self.ambient.lock();
        let state_name = match state.phase {
            // On, and still off: there is no show to play.
            Phase::Off if state.mode == AmbientMode::On => "unavailable",
            Phase::Off => "off",
            Phase::Pending { .. } => "pending",
            Phase::Playing => "playing",
        };
        AmbientReport {
            mode: state.mode,
            show: state.ambient.as_ref().map(|a| a.path.clone()),
            state: state_name,
        }
    }

    /// One effects-loop tick of ambient light, before the frame renders:
    /// start it once its delay is up, move its time on, and once it is all
    /// that shows between songs, clear what it covers.
    pub(super) fn tick_ambient(&self) {
        let mut state = self.ambient.lock();
        let now = Instant::now();
        if let Phase::Pending { at } = state.phase {
            if now >= at {
                match state.ambient.clone() {
                    Some(ambient) => {
                        self.raise_overlay(&ambient, now);
                        state.phase = Phase::Playing;
                    }
                    None => state.phase = Phase::Off,
                }
            }
        }
        self.advance_overlay(now);
        if state.phase == Phase::Playing && !state.song_lit && !state.under_dark {
            let full = self
                .ambient_overlay
                .lock()
                .as_ref()
                .is_some_and(|o| o.level.at(now) >= 1.0);
            if full {
                // A loop break's fade going out goes on at its own length:
                // ambient covers only what it lights, and what it does not
                // is still fading in view.
                self.darken_under(Duration::ZERO, false);
                state.under_dark = true;
            }
        }
    }

    /// Renders ambient light over the main engine's frame `under`, when it
    /// is playing or fading, and leaves what the rig shows -- the blend --
    /// on the main engine for the web UI and `get_fixture_state`. `None`
    /// when there is no ambient to cross in.
    pub(super) fn render_ambient(
        &self,
        effect_engine: &mut EffectEngine,
        dt: Duration,
        under: &[DmxCommand],
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        let mut overlay = self.ambient_overlay.lock();
        let Some(o) = overlay.as_mut() else {
            return Ok(None);
        };
        let now = Instant::now();
        let blended = o.render_over(dt, now, under, effect_engine)?;
        effect_engine.set_shown(Some((
            o.engine.shown_fixture_states().clone(),
            o.engine.shown_poses().clone(),
        )));
        if o.faded_out(now) {
            *overlay = None;
        }
        Ok(Some(blended))
    }

    /// The main engine's rig changed (a venue registration): ambient's
    /// engine follows it.
    pub(super) fn ambient_follows_rig(&self, main: &EffectEngine) {
        if let Some(o) = self.ambient_overlay.lock().as_mut() {
            o.adopt_rig(main);
        }
    }

    /// Fades ambient in to `ambient`: the overlay playing it rises from
    /// where it is, or a new one crosses in over whatever plays, over its
    /// `fade_in`.
    fn raise_overlay(&self, ambient: &LoadedAmbient, now: Instant) {
        if let Some(o) = self
            .ambient_overlay
            .lock()
            .as_mut()
            .filter(|o| o.ambient.same_as(ambient))
        {
            o.level.toward(now, 1.0, ambient.settings.fade_in);
            return;
        }
        info!(show = %ambient.path.display(), "Ambient light fades in.");
        // Locked in the effects loop's order: the engine, then the overlay.
        let engine = self.effect_engine.lock().sibling();
        let mut fresh = Overlay::new(ambient.clone(), engine, now);
        let mut overlay = self.ambient_overlay.lock();
        fresh.inner = overlay.take().map(Box::new);
        *overlay = Some(fresh);
    }

    /// Fades ambient out from where it is, over its `fade_out`. Between
    /// songs, a last song's frame still under it fades to dark with it.
    fn lower_overlay(&self, state: &mut AmbientState, now: Instant) {
        let fade = {
            let mut overlay = self.ambient_overlay.lock();
            let Some(o) = overlay.as_mut() else {
                return;
            };
            let fade = o.ambient.settings.fade_out;
            o.level.toward(now, 0.0, fade);
            fade
        };
        if !state.song_lit && !state.under_dark {
            self.darken_under(fade, true);
            state.under_dark = true;
        }
    }

    /// Fades the main engine to dark over `fade`: whatever the last song
    /// left -- its show, file watcher, MIDI DMX and tempo -- is done with.
    fn darken_under(&self, fade: Duration, to_dark: bool) {
        // Going dark, what the rig holds lit that the last frame did not
        // write -- left by effects that ended, which nothing writes again --
        // fades out with the rest (#573). Under ambient at full it stays as
        // it was, what ambient does not light still in view.
        let floor = if to_dark {
            self.stale_lit_channels()
        } else {
            Vec::new()
        };
        self.begin_break_fade_over(fade, floor);
        self.watcher_handle.lock().take();
        self.midi_dmx_playbacks.lock().clear();
        self.effect_engine.lock().set_tempo_map(None);
    }

    /// The channels the rig holds lit that the last frame did not write.
    fn stale_lit_channels(&self) -> Vec<crate::lighting::effects::DmxCommand> {
        let written = self.last_written.lock().clone();
        self.universes
            .iter()
            .flat_map(|(&universe, u)| {
                u.lit_channels().into_iter().map(move |(channel, value)| {
                    crate::lighting::effects::DmxCommand {
                        universe,
                        channel,
                        value,
                    }
                })
            })
            .filter(|c| !written.contains(&(c.universe, c.channel)))
            .collect()
    }

    /// Moves the overlay's time on and starts what its timeline fires, its
    /// groups resolved with no overlay lock held, as for the outgoing pass.
    fn advance_overlay(&self, now: Instant) {
        let mut updates = Vec::new();
        {
            let mut overlay = self.ambient_overlay.lock();
            let Some(o) = overlay.as_mut() else {
                return;
            };
            o.advance(now, &mut updates);
        }
        let mut resolved = updates
            .into_iter()
            .map(|mut u| {
                if let Some(restart) = u.restart.as_mut() {
                    self.resolve_update_groups(restart);
                }
                self.resolve_update_groups(&mut u.update);
                u
            })
            .collect::<Vec<_>>()
            .into_iter();
        if let Some(o) = self.ambient_overlay.lock().as_mut() {
            o.apply(&mut resolved);
        }
    }

    fn resolve_update_groups(&self, update: &mut TimelineUpdate) {
        update.effects = std::mem::take(&mut update.effects)
            .into_iter()
            .map(|effect| self.resolve_effect_groups(effect))
            .collect();
        for (effect, _) in update.effects_with_elapsed.values_mut() {
            *effect = self.resolve_effect_groups(effect.clone());
        }
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

    /// The wash as the rig shows it to the web UI: (red, green, blue), 0-1.
    fn shown(engine: &Engine) -> (f64, f64, f64) {
        let effect_engine = engine.effect_engine.lock();
        let state = effect_engine.shown_fixture_states().get("wash").cloned();
        let value = |channel: &str| {
            state
                .as_ref()
                .and_then(|s| s.channels.get(channel))
                .map_or(0.0, |c| c.value)
        };
        (value("red"), value("green"), value("blue"))
    }

    /// Red for a second, then green.
    const RED_THEN_GREEN: &str =
        "show \"song\" {\n    @0.000\n    wash: static, color: \"red\", duration: 1s\n    \
                                  @1.000\n    wash: static, color: \"green\", duration: 60s\n}\n";

    #[test]
    fn on_holds_over_a_song_and_auto_crosses_back_where_it_is() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("5s", "200ms", "200ms"))?;
        // Idle at startup, its delay still to run when the song starts.
        engine.idle_ambient(None, true);
        start_song(&engine, RED_THEN_GREEN);
        assert_eq!(song_frame(&engine, 0.1), (255.0, 0.0, 0.0));

        // On: ambient fades up over the song, with no delay.
        engine.set_ambient_mode(AmbientMode::On);
        let first = song_frame(&engine, 0.1);
        assert!(first.0 > 200.0, "fades from the song: {first:?}");
        sleep(Duration::from_millis(250));
        assert_eq!(song_frame(&engine, 0.35), (0.0, 0.0, 255.0));
        let view = shown(&engine);
        assert!(
            view.2 > 0.99 && view.0 < 0.01,
            "the view shows ambient: {view:?}"
        );
        assert_eq!(engine.ambient_report().state, "playing");

        // The song's show runs on underneath: back to Auto, it crosses to
        // where the song is now, green.
        assert_eq!(song_frame(&engine, 1.5), (0.0, 0.0, 255.0));
        engine.set_ambient_mode(AmbientMode::Auto);
        assert_eq!(engine.ambient_report().state, "off");
        let first = song_frame(&engine, 1.5);
        assert!(first.2 > 200.0, "fades from ambient: {first:?}");
        sleep(Duration::from_millis(250));
        assert_eq!(song_frame(&engine, 1.75), (0.0, 255.0, 0.0));
        let view = shown(&engine);
        assert!(
            view.1 > 0.99 && view.2 < 0.01,
            "the view shows the song: {view:?}"
        );
        Ok(())
    }

    #[test]
    fn on_wins_over_a_song_that_starts() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "0s"))?;
        engine.set_ambient_mode(AmbientMode::On);
        engine.idle_ambient(None, true);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        start_song(&engine, RED);
        assert_eq!(engine.ambient_report().state, "playing");
        assert_eq!(song_frame(&engine, 0.5), (0.0, 0.0, 255.0));

        // The song ends with ambient still on; the next idle keeps it.
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        Ok(())
    }

    #[test]
    fn off_fades_ambient_out_to_dark_and_keeps_it_off() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "300ms"))?;
        start_song(&engine, RED);
        song_frame(&engine, 0.5);
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));

        engine.set_ambient_mode(AmbientMode::Off);
        let report = engine.ambient_report();
        assert_eq!((report.mode, report.state), (AmbientMode::Off, "off"));
        let first = tick(&engine);
        assert!(first.2 > 200.0, "fades from ambient: {first:?}");
        sleep(Duration::from_millis(350));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0), "dark, not the last song");

        // Going idle again plays nothing.
        engine.idle_ambient(None, true);
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));

        // A song lights as ever.
        start_song(&engine, GREEN);
        assert_eq!(song_frame(&engine, 0.1), (0.0, 255.0, 0.0));

        // Back to Auto between songs: ambient comes up.
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        engine.set_ambient_mode(AmbientMode::Auto);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));
        Ok(())
    }

    /// Patches a spot at channels 4-6 of universe 5, beside the wash.
    fn add_spot(engine: &Engine) {
        let mut channels = std::collections::HashMap::new();
        for (name, offset) in [("red", 1), ("green", 2), ("blue", 3)] {
            channels.insert(name.to_string(), offset);
        }
        engine
            .effect_engine
            .lock()
            .register_fixture(crate::lighting::effects::FixtureInfo::new(
                "spot".to_string(),
                5,
                4,
                "Generic".to_string(),
                channels,
                None,
            ));
    }

    /// The spot's red, which only the vamp lights.
    fn spot_red(engine: &Engine) -> f64 {
        engine.get_universe(5).unwrap().get_target_value(3)
    }

    const VAMP: &str = "show \"song\" {\n    @0.000\n    wash: static, color: \"red\", duration: 60s\n    spot: static, color: \"red\", duration: 60s\n}\n";

    /// A vamp lighting wash and spot breaks out into GREEN, which lights
    /// only the wash, its fade still running at `secs` into GREEN.
    fn vamp_into_green(engine: &Engine) {
        add_spot(engine);
        start_song(engine, VAMP);
        song_frame(engine, 0.5);
        assert_eq!(spot_red(engine), 255.0);
        engine.begin_break_fade(Duration::from_secs(10));
        engine.stop_lighting_timeline();
        start_song(engine, GREEN);
        song_frame(engine, 0.5);
        sleep(Duration::from_millis(200));
        song_frame(engine, 0.7);
        assert!(spot_red(engine) > 200.0, "the vamp still fading");
    }

    #[test]
    fn a_stop_under_a_break_fade_leaves_nothing_lit() -> Result<(), Box<dyn Error>> {
        // A Stop while the vamp still faded under the next song dropped its
        // fade: the spot, which the next song does not light, held.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        engine.set_ambient_mode(AmbientMode::Off);
        // Ambient's settings loaded, as at the player's startup, and its
        // dark fade over.
        engine.idle_ambient(None, false);
        tick(&engine);
        sleep(Duration::from_millis(150));
        tick(&engine);
        vamp_into_green(&engine);
        engine.stop_lighting_timeline();
        engine.cut_break_fade_lights();
        engine.idle_ambient(None, true);
        tick(&engine);
        sleep(Duration::from_millis(300));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0), "the wash dark");
        assert_eq!(spot_red(&engine), 0.0, "the spot dark too");
        Ok(())
    }

    #[test]
    fn a_seek_under_a_break_fade_lets_the_vamp_go() -> Result<(), Box<dyn Error>> {
        // The same for a seek in the next song: it plays on, the vamp's
        // spot must not hold.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        vamp_into_green(&engine);
        engine.stop_lighting_timeline();
        engine.cut_break_fade_lights();
        start_song(&engine, GREEN);
        song_frame(&engine, 2.0);
        // Ambient's settings unloaded: the default fade_out, a second.
        sleep(Duration::from_millis(1100));
        assert_eq!(
            song_frame(&engine, 3.1),
            (0.0, 255.0, 0.0),
            "the song plays"
        );
        assert_eq!(spot_red(&engine), 0.0, "the vamp gone");
        Ok(())
    }

    #[test]
    fn a_song_ending_under_a_break_fade_lets_that_fade_run_its_length() -> Result<(), Box<dyn Error>>
    {
        // With the mode Off, the song going dark as it ended took the vamp's
        // fade under it dark at the same short pace.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        engine.set_ambient_mode(AmbientMode::Off);
        vamp_into_green(&engine);
        // GREEN ends by itself, the vamp still 10 s from out.
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        tick(&engine);
        sleep(Duration::from_millis(400));
        let (_, green, _) = tick(&engine);
        assert_eq!(green, 0.0, "the song gone dark");
        assert!(spot_red(&engine) > 200.0, "the vamp still at its own pace");
        Ok(())
    }

    #[test]
    fn a_loop_crossfade_under_a_break_fade_keeps_it() -> Result<(), Box<dyn Error>> {
        // The next song looping with a crossfade dropped the vamp's fade
        // under it, and what only the vamp lit held.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        vamp_into_green(&engine);
        let before = spot_red(&engine);
        engine.begin_loop_crossfade(
            Some(timeline(GREEN)),
            Duration::from_secs(1),
            Duration::from_millis(100),
        );
        song_frame(&engine, 0.05);
        song_frame(&engine, 0.3);
        sleep(Duration::from_millis(300));
        song_frame(&engine, 0.6);
        let after = spot_red(&engine);
        assert!(
            after < before && after > 150.0,
            "fading on: {before} -> {after}"
        );
        assert!(
            engine.get_universe(5).unwrap().get_target_value(1) > 0.0,
            "the song plays"
        );
        Ok(())
    }

    /// Patches a mover with a pan channel at channel 7 of universe 5.
    fn add_mover(engine: &Engine) {
        engine
            .effect_engine
            .lock()
            .register_fixture(crate::lighting::effects::FixtureInfo::new(
                "mover".to_string(),
                5,
                7,
                "Generic".to_string(),
                std::collections::HashMap::from([("pan".to_string(), 1)]),
                None,
            ));
    }

    fn pan(engine: &Engine) -> f64 {
        engine.get_universe(5).unwrap().get_target_value(6)
    }

    /// A show that pans the mover to `value`.
    fn pan_show(value: &str) -> String {
        format!(
            "show \"song\" {{\n    @0.000\n    mover: static, pan: {value}, duration: 60s\n}}\n"
        )
    }

    /// Renders frames for `secs`, asserting the mover's pan never steps
    /// further than its slowest fade could carry it in the time between
    /// two frames (`per_sec`), plus a little: a stalled machine waits
    /// longer between frames, not a jump.
    fn assert_pan_glides(engine: &Engine, secs: f64, per_sec: f64) {
        let started = std::time::Instant::now();
        let mut last = (pan(engine), std::time::Instant::now());
        let mut song = engine.get_song_time().as_secs_f64();
        while started.elapsed().as_secs_f64() < secs {
            sleep(Duration::from_millis(20));
            song += 0.02;
            song_frame(engine, song);
            let now = (pan(engine), std::time::Instant::now());
            let step = (now.0 - last.0).abs();
            let allowed = 6.0 + per_sec * now.1.duration_since(last.1).as_secs_f64();
            assert!(step <= allowed, "pan stepped {} to {}", last.0, now.0);
            last = now;
        }
    }

    #[test]
    fn a_shorter_fade_ending_first_does_not_jump_a_mover() -> Result<(), Box<dyn Error>> {
        // A mover's pan, held by a newer fade that ended before an older
        // one, jumped as that newer fade went.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        add_mover(&engine);
        // A pans to 80%, then breaks into B (20%) over 2 s; B breaks into
        // GREEN, which does not drive the mover, over 300 ms.
        start_song(&engine, &pan_show("80%"));
        song_frame(&engine, 0.5);
        assert_eq!(pan(&engine), 204.0);
        engine.begin_break_fade(Duration::from_secs(2));
        engine.stop_lighting_timeline();
        start_song(&engine, &pan_show("20%"));
        song_frame(&engine, 0.1);
        sleep(Duration::from_millis(200));
        song_frame(&engine, 0.3);
        engine.begin_break_fade(Duration::from_millis(300));
        engine.stop_lighting_timeline();
        start_song(&engine, GREEN);
        // The 300 ms fade moves it fastest: 153 over 0.3 s.
        assert_pan_glides(&engine, 0.8, 520.0);
        Ok(())
    }

    #[test]
    fn a_loop_pass_going_under_a_fade_does_not_jump_a_mover() -> Result<(), Box<dyn Error>> {
        // A looping song's pass going under an older fade -- its crossfade
        // over, or cut short by another break, the next pass, a stop or a
        // seek -- jumped the pan it held. Held, it fires no more cues: B's
        // at 5 s would pan the mover to 0.
        let b = "show \"song\" {\n    @0.000\n    mover: static, pan: 20%, duration: 60s\n    @5.000\n    mover: static, pan: 0%, duration: 60s\n}\n";
        for next in ["end", "break", "pass", "stop", "seek"] {
            let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
            add_mover(&engine);
            start_song(&engine, &pan_show("80%"));
            song_frame(&engine, 0.5);
            engine.begin_break_fade(Duration::from_secs(3));
            engine.stop_lighting_timeline();
            start_song(&engine, b);
            song_frame(&engine, 0.1);
            // A third of the way out, A and B are well apart.
            sleep(Duration::from_millis(1000));
            song_frame(&engine, 1.1);
            // B loops into a pass that does not drive the mover, over 500 ms;
            // only the 3 s fade moves the pan meanwhile (153 over 3 s).
            engine.begin_loop_crossfade(
                Some(timeline(GREEN)),
                Duration::from_secs(1),
                Duration::from_millis(500),
            );
            assert_pan_glides(&engine, 0.15, 100.0);
            match next {
                "end" => {}
                "break" => {
                    engine.begin_break_fade(Duration::from_millis(300));
                    engine.stop_lighting_timeline();
                    start_song(&engine, GREEN);
                }
                "pass" => engine.begin_loop_crossfade(
                    Some(timeline(GREEN)),
                    Duration::from_secs(1),
                    Duration::from_millis(500),
                ),
                "stop" => engine.stop_lighting_timeline(),
                _ => {
                    engine.update_song_time(Duration::from_secs(10));
                    engine.start_lighting_timeline_at(Duration::from_secs(10));
                }
            }
            assert_pan_glides(&engine, 0.8, 100.0);
            // B's 0% cue would leave it near 70; the fade alone, past 100.
            assert!(pan(&engine) > 85.0, "{next}: no cue of B's fired");
        }
        Ok(())
    }

    #[test]
    fn a_loop_pass_going_before_its_first_frame_does_not_jump_a_mover() -> Result<(), Box<dyn Error>>
    {
        // A pass set aside before it had rendered held an empty frame, and
        // the pan it drove fell through to the older fade's.
        for next in ["pass", "stop"] {
            let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
            add_mover(&engine);
            start_song(&engine, &pan_show("80%"));
            song_frame(&engine, 0.5);
            engine.begin_break_fade(Duration::from_secs(3));
            engine.stop_lighting_timeline();
            start_song(&engine, &pan_show("20%"));
            song_frame(&engine, 0.1);
            sleep(Duration::from_millis(1000));
            song_frame(&engine, 1.1);
            let before = pan(&engine);
            engine.begin_loop_crossfade(
                Some(timeline(GREEN)),
                Duration::from_secs(1),
                Duration::from_millis(500),
            );
            match next {
                "pass" => engine.begin_loop_crossfade(
                    Some(timeline(GREEN)),
                    Duration::from_secs(1),
                    Duration::from_millis(500),
                ),
                _ => engine.stop_lighting_timeline(),
            }
            song_frame(&engine, 0.02);
            let after = pan(&engine);
            assert!((after - before).abs() < 10.0, "{next}: {before} -> {after}");
        }
        Ok(())
    }

    #[test]
    fn an_inner_pass_over_inside_a_fade_is_held() -> Result<(), Box<dyn Error>> {
        // A loop break during a crossfade between passes keeps the older
        // pass crossing inside the fade (#560); its crossfade over, it
        // stops rendering and holds its last frame.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        start_song(&engine, RED);
        song_frame(&engine, 0.5);
        engine.begin_loop_crossfade(
            Some(timeline(GREEN)),
            Duration::from_secs(1),
            Duration::from_secs(1),
        );
        song_frame(&engine, 0.05);
        engine.begin_break_fade(Duration::from_secs(5));
        engine.stop_lighting_timeline();
        start_song(&engine, BLUE);
        let inner_held = |engine: &Engine| {
            engine.break_fades.lock()[0]
                .inner
                .as_ref()
                .map(|inner| inner.held)
        };
        song_frame(&engine, 0.01);
        assert_eq!(inner_held(&engine), Some(false), "still crossing");
        sleep(Duration::from_millis(1200));
        song_frame(&engine, 1.2);
        assert_eq!(inner_held(&engine), Some(true), "over, and held");
        Ok(())
    }

    #[test]
    fn a_merged_held_pass_shows_what_it_sends() -> Result<(), Box<dyn Error>> {
        // Two held passes merged into one showed the older one's pan while
        // the rig got the newer one's.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        add_mover(&engine);
        start_song(&engine, &pan_show("80%"));
        song_frame(&engine, 0.5);
        engine.begin_break_fade(Duration::from_secs(2));
        engine.stop_lighting_timeline();
        start_song(&engine, &pan_show("20%"));
        song_frame(&engine, 0.1);
        // B's pass (20%) goes into one at 50%, which goes into GREEN.
        for next in [pan_show("50%"), GREEN.to_string()] {
            engine.begin_loop_crossfade(
                Some(timeline(&next)),
                Duration::from_secs(1),
                Duration::from_millis(50),
            );
            song_frame(&engine, 0.01);
            song_frame(&engine, 0.2);
        }
        assert_eq!(engine.break_fades.lock().len(), 2);
        // Half way out, the held pan (50%) shows through A's (80%).
        sleep(Duration::from_millis(800));
        song_frame(&engine, 1.0);
        let sent = pan(&engine);
        assert!(sent < 190.0, "the held pass shows through: {sent}");
        let shown =
            engine.effect_engine.lock().shown_fixture_states()["mover"].channels["pan"].value;
        assert!(
            (shown * 255.0 - sent).abs() < 3.0,
            "shows {} but sends {sent}",
            shown * 255.0
        );
        Ok(())
    }

    #[test]
    fn a_fade_over_under_an_older_one_fires_no_more_cues() -> Result<(), Box<dyn Error>> {
        // A fade that was over, kept for an older one still going out, went
        // on playing its song's show: B's cue at 1 s moved the mover under
        // A's fade long after B had faded out.
        let b = "show \"song\" {\n    @0.000\n    mover: static, pan: 20%, duration: 60s\n    @1.000\n    mover: static, pan: 0%, duration: 60s\n}\n";
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        add_mover(&engine);
        start_song(&engine, &pan_show("80%"));
        song_frame(&engine, 0.5);
        engine.begin_break_fade(Duration::from_secs(6));
        engine.stop_lighting_timeline();
        start_song(&engine, b);
        song_frame(&engine, 0.1);
        engine.begin_break_fade(Duration::from_millis(300));
        engine.stop_lighting_timeline();
        start_song(&engine, GREEN);
        // Past B's fade and its cue: only A's 6 s fade moves the pan.
        assert_pan_glides(&engine, 1.5, 100.0);
        let fades = engine.break_fades.lock();
        assert_eq!(fades.len(), 2);
        assert!(fades[1].held, "B's fade, over, is held");
        Ok(())
    }

    #[test]
    fn a_song_looping_under_a_long_fade_adds_one_held_layer() -> Result<(), Box<dyn Error>> {
        // Each pass that went under the fade stacked another layer, every
        // one rendering a whole engine each frame until the fade was out.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        add_mover(&engine);
        start_song(&engine, &pan_show("80%"));
        song_frame(&engine, 0.5);
        engine.begin_break_fade(Duration::from_secs(30));
        engine.stop_lighting_timeline();
        start_song(&engine, &pan_show("20%"));
        song_frame(&engine, 0.1);
        for _ in 0..10 {
            engine.begin_loop_crossfade(
                Some(timeline(GREEN)),
                Duration::from_secs(1),
                Duration::from_millis(50),
            );
            song_frame(&engine, 0.01);
            song_frame(&engine, 0.2);
        }
        let fades = engine.break_fades.lock();
        assert_eq!(fades.len(), 2, "the fade, and one held pass");
        assert!(fades[1].held);
        Ok(())
    }

    #[test]
    fn a_show_over_before_its_song_still_goes_dark_while_off() -> Result<(), Box<dyn Error>> {
        // #573: a show whose effects had all ended left their last frame on
        // the rig -- nothing writes a channel no effect drives, and the
        // engine darkened had nothing left to fade.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        engine.set_ambient_mode(AmbientMode::Off);
        engine.idle_ambient(None, false);
        tick(&engine);
        sleep(Duration::from_millis(150));
        tick(&engine);
        start_song(
            &engine,
            "show \"song\" {\n    @0.000\n    wash: static, color: \"red\", duration: 200ms\n}\n",
        );
        assert_eq!(song_frame(&engine, 0.1), (255.0, 0.0, 0.0));
        // The effect is over; the rig holds its frame, as it does between
        // cues.
        assert_eq!(song_frame(&engine, 0.5), (255.0, 0.0, 0.0));
        // The song ends.
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        tick(&engine);
        sleep(Duration::from_millis(300));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0), "dark over fade_out");
        Ok(())
    }

    #[test]
    fn a_song_stopped_while_off_fades_to_dark() -> Result<(), Box<dyn Error>> {
        // #570: with the mode Off, the song's last frame held for good.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "300ms"))?;
        engine.set_ambient_mode(AmbientMode::Off);
        start_song(&engine, RED);
        assert_eq!(song_frame(&engine, 0.5), (255.0, 0.0, 0.0));
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        let first = tick(&engine);
        assert!(first.0 > 200.0, "fades from the song: {first:?}");
        sleep(Duration::from_millis(350));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));
        // Going idle again changes nothing.
        engine.idle_ambient(None, true);
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));
        Ok(())
    }

    #[test]
    fn an_ending_while_off_fades_at_its_own_length() -> Result<(), Box<dyn Error>> {
        // #571: ambient's short fade_out cut across a graceful ending's.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        engine.set_ambient_mode(AmbientMode::Off);
        start_song(&engine, RED);
        assert_eq!(song_frame(&engine, 0.5), (255.0, 0.0, 0.0));
        // The ending's lights cross to dark over a second, as Stop on a
        // looping song does, and the player goes idle at once.
        engine.begin_break_fade(Duration::from_secs(1));
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        sleep(Duration::from_millis(400));
        let mid = tick(&engine);
        assert!(mid.0 > 60.0, "still fading at the ending's pace: {mid:?}");
        sleep(Duration::from_millis(800));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0), "dark once it is over");
        Ok(())
    }

    #[test]
    fn an_ending_cut_while_off_still_goes_dark() -> Result<(), Box<dyn Error>> {
        // #572: a second Stop cut the ending's fade and froze its frame.
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "100ms"))?;
        engine.set_ambient_mode(AmbientMode::Off);
        start_song(&engine, RED);
        assert_eq!(song_frame(&engine, 0.5), (255.0, 0.0, 0.0));
        engine.begin_break_fade(Duration::from_secs(10));
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        sleep(Duration::from_millis(200));
        let mid = tick(&engine);
        assert!(mid.0 > 200.0, "still fading at the ending's pace: {mid:?}");
        // The cut: the ending's lights hurry to dark, and the player goes
        // idle again.
        engine.cut_break_fade_lights();
        engine.idle_ambient(None, true);
        let cut = tick(&engine);
        assert!(cut.0 > 200.0, "no jump at the cut: {cut:?}");
        sleep(Duration::from_millis(300));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0), "dark over fade_out");
        Ok(())
    }

    #[test]
    fn off_while_fading_in_takes_the_last_songs_frame_dark_too() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "2s", "200ms"))?;
        start_song(&engine, RED);
        song_frame(&engine, 0.5);
        engine.stop_lighting_timeline();
        engine.idle_ambient(None, true);
        tick(&engine);
        sleep(Duration::from_millis(200));
        let mid = tick(&engine);
        assert!(mid.0 > 100.0 && mid.2 > 0.0, "part way in: {mid:?}");

        engine.set_ambient_mode(AmbientMode::Off);
        sleep(Duration::from_millis(300));
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));
        Ok(())
    }

    #[test]
    fn on_with_no_show_to_play_says_so() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine("show \"ambient\" {", ("0s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        assert_eq!(engine.ambient_report().state, "off");
        engine.set_ambient_mode(AmbientMode::On);
        let report = engine.ambient_report();
        assert_eq!(
            (report.mode, report.state),
            (AmbientMode::On, "unavailable")
        );
        assert_eq!(report.show, None);
        assert_eq!(tick(&engine), (0.0, 0.0, 0.0));
        Ok(())
    }

    #[test]
    fn ambient_follows_a_venue_reregistration() -> Result<(), Box<dyn Error>> {
        let (_dir, engine) = ambient_engine(BLUE, ("0s", "0s", "0s"))?;
        engine.idle_ambient(None, false);
        assert_eq!(tick(&engine), (0.0, 0.0, 255.0));

        // The wash moves to channels 4-6.
        let mut channels = std::collections::HashMap::new();
        for (name, offset) in [("red", 1), ("green", 2), ("blue", 3)] {
            channels.insert(name.to_string(), offset);
        }
        {
            let mut main = engine.effect_engine.lock();
            main.replace_fixtures(vec![crate::lighting::effects::FixtureInfo::new(
                "wash".to_string(),
                5,
                4,
                "Generic".to_string(),
                channels,
                None,
            )]);
            engine.ambient_follows_rig(&main);
        }
        engine.effects_loop_tick();
        let universe = engine.get_universe(5).unwrap();
        assert_eq!(universe.get_target_value(5), 255.0);
        Ok(())
    }

    #[test]
    fn modes_parse_and_name_themselves() {
        for mode in [AmbientMode::Auto, AmbientMode::On, AmbientMode::Off] {
            assert_eq!(AmbientMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(AmbientMode::parse(" ON "), Some(AmbientMode::On));
        assert_eq!(AmbientMode::parse("dim"), None);
    }
}
