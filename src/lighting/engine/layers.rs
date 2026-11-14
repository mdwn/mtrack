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

//! The layers' masters, and each layer's clock: how the time of the effects
//! on it runs.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::super::effects::{EffectInstance, EffectLayer};
use super::{scale_by_speed, unscale_by_speed};

/// The engine's "now", on both its time bases: the engine instant, which
/// steps a frame a tick, and the song time, when there is one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Now {
    pub instant: Instant,
    pub song: Option<Duration>,
}

/// When a layer froze, on both time bases.
#[derive(Clone, Copy, Debug)]
struct Frozen {
    /// What an effect timed on engine time holds at.
    at: Instant,
    /// What a cued effect holds at, and measures its time frozen from;
    /// `None` when the freeze came with no song time.
    song: Option<Duration>,
}

/// How the time of a layer's effects runs: its freeze and its speed master.
///
/// The one model of both: the engine renders and expires by it, a seek's
/// history replays through the engine and so through it, and a show's ends
/// are reckoned by it (see `LightingTimeline::effect_ends`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerClock {
    frozen: Option<Frozen>,
    /// The speed master, 1 by default. 0 freezes the layer rather than
    /// scaling its time.
    speed: f64,
    /// The song time the clock last changed at: its effects have run at one
    /// rate since.
    since: Duration,
}

impl Default for LayerClock {
    fn default() -> Self {
        Self {
            frozen: None,
            speed: 1.0,
            since: Duration::ZERO,
        }
    }
}

/// How long an effect started at `start` was held by a freeze from `frozen`
/// to `now`: from the freeze, or from its own start for one started during
/// it, which held its first frame from then (pushed by the whole freeze, it
/// waited out the part before its start again after the unfreeze).
fn held_for<T: Ord + Copy>(
    start: T,
    frozen: T,
    now: T,
    span: impl Fn(T, T) -> Duration,
) -> Duration {
    span(now, start.max(frozen))
}

impl LayerClock {
    /// The layer's speed master.
    pub(crate) fn speed(&self) -> f64 {
        self.speed
    }

    /// Whether the layer is frozen, by a freeze or a speed of 0.
    pub(crate) fn is_frozen(&self) -> bool {
        self.frozen.is_some()
    }

    /// How far into itself `effect` is at `now` before the speed master: on
    /// score time for a cued effect -- song time since its cue, less the
    /// song time it has spent frozen, held at the song time the layer froze
    /// at while frozen -- and otherwise on engine time since it started,
    /// held at the instant the layer froze at. An effect through a freeze
    /// with no song time to measure it by stays on engine time.
    pub(crate) fn base_elapsed(&self, effect: &EffectInstance, now: Now) -> Duration {
        match (effect.start_time, effect.cue_time, now.song) {
            (Some(_), Some(cue_time), Some(song))
                if !effect.engine_timed && self.frozen.is_none_or(|f| f.song.is_some()) =>
            {
                self.frozen
                    .and_then(|f| f.song)
                    .unwrap_or(song)
                    .saturating_sub(cue_time)
                    .saturating_sub(effect.score_frozen)
            }
            _ => effect
                .start_time
                .map(|start| {
                    self.frozen
                        .map_or(now.instant, |f| f.at)
                        .duration_since(start)
                })
                .unwrap_or(Duration::ZERO),
        }
    }

    /// `effect`'s own elapsed at `now`, what it renders and expires on: where
    /// it was at the layer's last speed change, on from there at the layer's
    /// speed.
    pub(crate) fn elapsed(&self, effect: &EffectInstance, now: Now) -> Duration {
        let base = self.base_elapsed(effect, now);
        effect.elapsed_from.saturating_add(scale_by_speed(
            base.saturating_sub(effect.speed_from),
            self.speed,
        ))
    }

    fn changed(&mut self, now: Now) {
        if let Some(song) = now.song {
            self.since = song;
        }
    }

    /// Freezes the layer at `now`, unless it is frozen already.
    pub(crate) fn freeze(&mut self, now: Now) {
        self.changed(now);
        self.frozen.get_or_insert(Frozen {
            at: now.instant,
            song: now.song,
        });
    }

    /// Lets the layer run again from `now`: each of its effects resumes where
    /// the freeze held it.
    pub(crate) fn unfreeze(
        &mut self,
        layer: EffectLayer,
        effects: &mut HashMap<String, EffectInstance>,
        now: Now,
    ) {
        self.changed(now);
        let Some(frozen) = self.frozen.take() else {
            return;
        };
        for effect in effects.values_mut().filter(|e| e.layer == layer) {
            if let Some(start) = effect.start_time {
                effect.start_time = Some(
                    start + held_for(start, frozen.at, now.instant, |a, b| a.duration_since(b)),
                );
            }
            match (frozen.song, now.song, effect.cue_time) {
                (Some(at), Some(song), Some(cue_time)) => {
                    effect.score_frozen += held_for(cue_time, at, song, |a, b| a.saturating_sub(b));
                }
                // No song time to measure the freeze by: on engine time
                // from here, which the start time above has shifted.
                (_, _, Some(_)) => effect.engine_timed = true,
                _ => {}
            }
        }
    }

    /// Sets the speed master from `now`: each effect on the layer carries on
    /// from where it is, not from its cue at the new speed. 0 freezes the
    /// layer, and a speed from 0 lets it run again.
    pub(crate) fn set_speed(
        &mut self,
        layer: EffectLayer,
        effects: &mut HashMap<String, EffectInstance>,
        speed: f64,
        now: Now,
    ) {
        for effect in effects.values_mut().filter(|e| e.layer == layer) {
            let (base, own) = (self.base_elapsed(effect, now), self.elapsed(effect, now));
            effect.speed_from = base;
            effect.elapsed_from = own;
        }
        let speed = speed.max(0.0);
        if speed == 0.0 {
            self.freeze(now);
        } else if self.speed == 0.0 {
            self.unfreeze(layer, effects, now);
        }
        self.changed(now);
        self.speed = if (speed - 1.0).abs() < f64::EPSILON {
            1.0
        } else {
            speed
        };
    }

    /// The song time `effect`'s own elapsed reaches its end at, the clock
    /// as it has been since it last changed: back from `now` for one already
    /// past it (not before its cue, nor the change it ran under), on from
    /// `now` for one still running. `None` for one that never ends: held by
    /// the freeze, or so slow no time there is reaches it.
    pub(crate) fn end_of(&self, effect: &EffectInstance, now: Now) -> Option<Duration> {
        let song = now.song.unwrap_or_default();
        let elapsed = self.elapsed(effect, now);
        let total = effect.total_duration();
        let at = self.frozen.and_then(|f| f.song).unwrap_or(song);
        if effect.has_reached_terminal_state(elapsed) {
            let past = unscale_by_speed(elapsed - total, self.speed);
            let floor = self.since.max(effect.cue_time.unwrap_or_default());
            Some(at.saturating_sub(past).max(floor))
        } else if self.is_frozen() {
            None
        } else {
            at.checked_add(unscale_by_speed(total - elapsed, self.speed))
        }
    }
}

/// Layer control state: intensity masters, and each layer's clock.
#[derive(Clone, Default)]
pub(crate) struct LayerState {
    /// Layer intensity masters (0.0 to 1.0) — multiplies effect output per layer.
    intensity_masters: HashMap<EffectLayer, f64>,
    clocks: HashMap<EffectLayer, LayerClock>,
}

impl LayerState {
    /// Reset every layer back to defaults — no masters, nothing frozen.
    pub(crate) fn reset(&mut self) {
        self.intensity_masters.clear();
        self.clocks.clear();
    }

    /// Reset a single layer back to defaults.
    pub(crate) fn reset_layer(&mut self, layer: EffectLayer) {
        self.intensity_masters.remove(&layer);
        self.clocks.remove(&layer);
    }

    pub(crate) fn intensity_master(&self, layer: EffectLayer) -> f64 {
        *self.intensity_masters.get(&layer).unwrap_or(&1.0)
    }

    /// Sets a layer's intensity master, clamped to 0.0 to 1.0.
    pub(crate) fn set_intensity_master(&mut self, layer: EffectLayer, intensity: f64) {
        let clamped = intensity.clamp(0.0, 1.0);
        if (clamped - 1.0).abs() < f64::EPSILON {
            // 1.0 is the default
            self.intensity_masters.remove(&layer);
        } else {
            self.intensity_masters.insert(layer, clamped);
        }
    }

    /// The layer's clock, at rest for a layer nothing has set.
    pub(crate) fn clock(&self, layer: EffectLayer) -> LayerClock {
        self.clocks.get(&layer).copied().unwrap_or_default()
    }

    /// Moves the instants the layers froze at by `rebase`: state replayed
    /// in one engine, put in another (see `EffectEngine::install`).
    pub(crate) fn rebase(&mut self, rebase: impl Fn(Instant) -> Instant) {
        for clock in self.clocks.values_mut() {
            if let Some(frozen) = clock.frozen.as_mut() {
                frozen.at = rebase(frozen.at);
            }
        }
    }

    /// Lays `later`'s masters and clocks over these: what was set there,
    /// later than these were, stands.
    pub(crate) fn overlay(&mut self, later: &LayerState) {
        self.intensity_masters
            .extend(later.intensity_masters.iter().map(|(k, v)| (*k, *v)));
        self.clocks
            .extend(later.clocks.iter().map(|(k, v)| (*k, *v)));
    }

    pub(crate) fn clock_mut(&mut self, layer: EffectLayer) -> &mut LayerClock {
        self.clocks.entry(layer).or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::super::super::effects::EffectType;
    use super::*;

    fn make_effect(id: &str, layer: EffectLayer) -> EffectInstance {
        let mut inst = EffectInstance::new(
            id.to_string(),
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            vec!["f1".to_string()],
            None,
            None,
            None,
        );
        inst.layer = layer;
        inst
    }

    fn at(instant: Instant) -> Now {
        Now {
            instant,
            song: None,
        }
    }

    #[test]
    fn intensity_master_set_value() {
        let mut state = LayerState::default();
        state.set_intensity_master(EffectLayer::BACKGROUND, 0.5);
        assert_eq!(state.intensity_master(EffectLayer::BACKGROUND), 0.5);
    }

    #[test]
    fn intensity_master_one_removes() {
        let mut state = LayerState::default();
        state.set_intensity_master(EffectLayer::BACKGROUND, 0.5);
        state.set_intensity_master(EffectLayer::BACKGROUND, 1.0);
        assert!(state.intensity_masters.is_empty());
    }

    #[test]
    fn intensity_master_clamps() {
        let mut state = LayerState::default();
        state.set_intensity_master(EffectLayer::BACKGROUND, 1.5);
        assert!(state.intensity_masters.is_empty());
        state.set_intensity_master(EffectLayer::BACKGROUND, -0.5);
        assert_eq!(state.intensity_master(EffectLayer::BACKGROUND), 0.0);
    }

    #[test]
    fn freeze_does_not_overwrite() {
        let mut clock = LayerClock::default();
        let t1 = Instant::now();
        clock.freeze(at(t1));
        clock.freeze(at(t1 + Duration::from_secs(1)));
        assert_eq!(clock.frozen.map(|f| f.at), Some(t1));
    }

    #[test]
    fn unfreeze_adjusts_start_times() {
        let mut clock = LayerClock::default();
        let base = Instant::now();
        let mut effect = make_effect("a", EffectLayer::BACKGROUND);
        effect.start_time = Some(base);
        let mut active: HashMap<String, EffectInstance> = [("a".to_string(), effect)].into();
        clock.freeze(at(base));
        clock.unfreeze(
            EffectLayer::BACKGROUND,
            &mut active,
            at(base + Duration::from_secs(2)),
        );
        assert!(!clock.is_frozen());
        assert_eq!(
            active["a"].start_time,
            Some(base + Duration::from_secs(2)),
            "pushed forward by its 2 s frozen"
        );
    }

    #[test]
    fn unfreeze_starts_an_effect_cued_during_the_freeze_from_its_beginning() {
        let mut clock = LayerClock::default();
        let base = Instant::now();
        let mut effect = make_effect("a", EffectLayer::BACKGROUND);
        // Frozen at 0; cued 5 s in; unfrozen at 10 s.
        effect.start_time = Some(base + Duration::from_secs(5));
        let mut active: HashMap<String, EffectInstance> = [("a".to_string(), effect)].into();
        clock.freeze(at(base));
        clock.unfreeze(
            EffectLayer::BACKGROUND,
            &mut active,
            at(base + Duration::from_secs(10)),
        );
        assert_eq!(
            active["a"].start_time,
            Some(base + Duration::from_secs(10)),
            "it waits out the 5 s before its cue again"
        );
    }

    #[test]
    fn a_speed_of_zero_freezes_and_a_speed_from_zero_unfreezes() {
        let mut clock = LayerClock::default();
        let mut active = HashMap::new();
        let now = at(Instant::now());
        clock.set_speed(EffectLayer::BACKGROUND, &mut active, 0.0, now);
        assert!(clock.is_frozen());
        clock.set_speed(EffectLayer::BACKGROUND, &mut active, 0.5, now);
        assert!(!clock.is_frozen());
        assert_eq!(clock.speed(), 0.5);
    }

    #[test]
    fn an_effects_end_is_reckoned_at_its_layers_speed() {
        let instant = Instant::now();
        let song = |s: u64| Now {
            instant,
            song: Some(Duration::from_secs(s)),
        };
        let mut clock = LayerClock::default();
        let mut effect = make_effect("a", EffectLayer::BACKGROUND);
        effect.start_time = Some(instant);
        effect.cue_time = Some(Duration::ZERO);
        let mut active: HashMap<String, EffectInstance> = [("a".to_string(), effect)].into();
        // Half speed from 1 s: 1 s in, 4 s to go at half speed, so 9 s.
        clock.set_speed(EffectLayer::BACKGROUND, &mut active, 0.5, song(1));
        assert_eq!(
            clock.end_of(&active["a"], song(2)),
            Some(Duration::from_secs(9))
        );
        assert_eq!(
            clock.end_of(&active["a"], song(12)),
            Some(Duration::from_secs(9)),
            "back from a time past it"
        );
        clock.freeze(song(3));
        assert_eq!(clock.end_of(&active["a"], song(4)), None);
    }
}
