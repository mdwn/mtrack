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

mod layers;
mod processing;
mod validation;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicU64, Ordering};

use super::effects::*;
use super::tempo::TempoMap;
use tracing::debug;

use crate::dmx::midi_dmx_store::MidiDmxStore;

/// Registry of known fixtures and their DMX mappings.
pub(crate) struct FixtureRegistry {
    fixtures: HashMap<String, FixtureInfo>,
    /// Reverse map from (universe_id, dmx_channel) to (fixture_name, channel_name).
    /// Only built during tests for validation; not needed in production.
    #[cfg(test)]
    dmx_to_fixture_map: HashMap<(u16, u16), (String, String)>,
}

impl FixtureRegistry {
    fn new() -> Self {
        Self {
            fixtures: HashMap::new(),
            #[cfg(test)]
            dmx_to_fixture_map: HashMap::new(),
        }
    }

    fn register(&mut self, fixture: FixtureInfo) {
        #[cfg(test)]
        {
            for (channel_name, &offset) in &fixture.channels {
                let dmx_channel = fixture.address + offset - 1;
                self.dmx_to_fixture_map.insert(
                    (fixture.universe, dmx_channel),
                    (fixture.name.clone(), channel_name.clone()),
                );
            }
        }
        self.fixtures.insert(fixture.name.clone(), fixture);
    }

    fn get(&self, name: &str) -> Option<&FixtureInfo> {
        self.fixtures.get(name)
    }

    fn contains(&self, name: &str) -> bool {
        self.fixtures.contains_key(name)
    }

    fn as_map(&self) -> &HashMap<String, FixtureInfo> {
        &self.fixtures
    }

    #[cfg(test)]
    fn lookup_dmx_channel(&self, universe: u16, dmx_channel: u16) -> Option<&(String, String)> {
        self.dmx_to_fixture_map.get(&(universe, dmx_channel))
    }
}

/// Layer control state: intensity masters, speed masters, and frozen layers.
struct LayerState {
    /// Layer intensity masters (0.0 to 1.0) — multiplies effect output per layer.
    intensity_masters: HashMap<EffectLayer, f64>,
    /// Layer speed masters (0.0+) — multiplies effect speed per layer (1.0 = normal).
    speed_masters: HashMap<EffectLayer, f64>,
    /// Frozen layers — maps layer to the Instant when it was frozen.
    frozen: HashMap<EffectLayer, Instant>,
}

impl LayerState {
    fn new() -> Self {
        Self {
            intensity_masters: HashMap::new(),
            speed_masters: HashMap::new(),
            frozen: HashMap::new(),
        }
    }

    fn intensity_master(&self, layer: &EffectLayer) -> f64 {
        *self.intensity_masters.get(layer).unwrap_or(&1.0)
    }

    fn speed_master(&self, layer: &EffectLayer) -> f64 {
        *self.speed_masters.get(layer).unwrap_or(&1.0)
    }
}

/// Caches the last computed DmxCommands so unchanged frames skip recomputation.
struct EffectCache {
    commands: Vec<DmxCommand>,
    /// Last-seen MIDI DMX store generation.
    last_store_generation: u32,
    /// True when engine state changed outside update() (e.g. clear_layer).
    dirty: bool,
}

impl EffectCache {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
            last_store_generation: 0,
            dirty: false,
        }
    }

    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn get_cached(&self) -> &[DmxCommand] {
        &self.commands
    }

    fn update(&mut self, commands: Vec<DmxCommand>, generation: u32) {
        self.commands = commands;
        self.last_store_generation = generation;
        self.dirty = false;
    }
}

/// The main effects engine that manages and processes lighting effects
pub struct EffectEngine {
    active_effects: HashMap<String, EffectInstance>,
    fixtures: FixtureRegistry,
    current_time: Instant,
    /// Elapsed simulated time since engine start
    engine_elapsed: Duration,
    /// Optional tempo map for tempo-aware effects (measure/beat-based timing)
    tempo_map: Option<TempoMap>,
    layer_state: LayerState,
    /// Effects being released — tracks (release_fade_time, release_start_time) per effect
    releasing_effects: HashMap<String, (Duration, Instant)>,
    /// Last computed merged fixture states (for preview/debugging)
    last_merged_states: HashMap<String, FixtureState>,
    /// Last known song time (score-time) for tempo-aware speed lookups.
    last_song_time: Option<Duration>,
    /// Reference to the MIDI DMX store for reading interpolated values each frame.
    midi_dmx_store: Option<Arc<parking_lot::RwLock<MidiDmxStore>>>,
    cache: EffectCache,
    /// Sub-phase indicator for update() progress.
    update_subphase: Arc<AtomicU64>,
}

impl Default for EffectEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectEngine {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            fixtures: FixtureRegistry::new(),
            current_time: Instant::now(),
            engine_elapsed: Duration::ZERO,
            tempo_map: None,
            layer_state: LayerState::new(),
            releasing_effects: HashMap::new(),
            last_merged_states: HashMap::new(),
            last_song_time: None,
            midi_dmx_store: None,
            cache: EffectCache::new(),
            update_subphase: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns the shared update sub-phase Arc for external monitoring.
    pub fn update_subphase(&self) -> Arc<AtomicU64> {
        self.update_subphase.clone()
    }

    /// Returns the current MIDI DMX store generation, or 0 if no store is set.
    fn store_generation(&self) -> u32 {
        self.midi_dmx_store
            .as_ref()
            .map(|s| s.read().generation())
            .unwrap_or(0)
    }

    /// Set the tempo map for tempo-aware effects
    pub fn set_tempo_map(&mut self, tempo_map: Option<TempoMap>) {
        self.tempo_map = tempo_map;
    }

    /// Returns whether a tempo map is currently set
    #[cfg(test)]
    pub fn has_tempo_map(&self) -> bool {
        self.tempo_map.is_some()
    }

    /// Format effect type for logging
    fn format_effect_for_logging(effect: &EffectInstance) -> (&'static str, String) {
        match &effect.effect_type {
            EffectType::Static {
                parameters,
                duration,
            } => (
                "Static",
                format!("params={:?}, duration={:?}", parameters, duration),
            ),
            EffectType::ColorCycle {
                colors,
                speed,
                direction,
                transition,
                ..
            } => (
                "ColorCycle",
                format!(
                    "colors={:?}, speed={:?}, direction={:?}, transition={:?}",
                    colors, speed, direction, transition
                ),
            ),
            EffectType::Strobe {
                frequency,
                duration,
            } => (
                "Strobe",
                format!("frequency={:?}, duration={:?}", frequency, duration),
            ),
            EffectType::Dimmer {
                start_level,
                end_level,
                duration,
                curve,
            } => (
                "Dimmer",
                format!(
                    "start_level={:?}, end_level={:?}, duration={:?}, curve={:?}",
                    start_level, end_level, duration, curve
                ),
            ),
            EffectType::Chase {
                pattern,
                speed,
                direction,
                ..
            } => (
                "Chase",
                format!(
                    "pattern={:?}, speed={:?}, direction={:?}",
                    pattern, speed, direction
                ),
            ),
            EffectType::Rainbow {
                speed,
                saturation,
                brightness,
                ..
            } => (
                "Rainbow",
                format!(
                    "speed={:?}, saturation={:?}, brightness={:?}",
                    speed, saturation, brightness
                ),
            ),
            EffectType::Pulse {
                base_level,
                pulse_amplitude,
                frequency,
                duration,
            } => (
                "Pulse",
                format!(
                    "base_level={:?}, pulse_amplitude={:?}, frequency={:?}, duration={:?}",
                    base_level, pulse_amplitude, frequency, duration
                ),
            ),
        }
    }

    /// Register a fixture with the engine
    pub fn register_fixture(&mut self, fixture: FixtureInfo) {
        // Validate fixture capabilities based on special cases
        if let Err(e) = validation::validate_fixture_capabilities(&fixture) {
            eprintln!(
                "Warning: Fixture '{}' has capability issues: {}",
                fixture.name, e
            );
        }

        self.fixtures.register(fixture);
    }

    /// Look up which fixture and channel a DMX address belongs to (test only).
    #[cfg(test)]
    pub fn lookup_dmx_channel(&self, universe: u16, dmx_channel: u16) -> Option<&(String, String)> {
        self.fixtures.lookup_dmx_channel(universe, dmx_channel)
    }

    /// Set the MIDI DMX store reference for reading interpolated MIDI values.
    pub fn set_midi_dmx_store(&mut self, store: Arc<parking_lot::RwLock<MidiDmxStore>>) {
        self.midi_dmx_store = Some(store);
    }

    /// Start an effect
    pub fn start_effect(&mut self, mut effect: EffectInstance) -> Result<(), EffectError> {
        // Validate effect
        validation::validate_effect(self.fixtures.as_map(), &effect)?;

        // Log effect parameters
        let (effect_kind, effect_params) = Self::format_effect_for_logging(&effect);
        debug!(
            effect_id = %effect.id,
            effect_kind,
            effect_params = %effect_params,
            layer = ?effect.layer,
            blend_mode = ?effect.blend_mode,
            priority = effect.priority,
            up_time = ?effect.up_time,
            hold_time = ?effect.hold_time,
            down_time = ?effect.down_time,
            targets = ?effect.target_fixtures,
            "Starting lighting effect"
        );

        // Set the start time to the current engine time
        effect.start_time = Some(self.current_time);
        self.active_effects.insert(effect.id.clone(), effect);
        self.cache.invalidate();
        Ok(())
    }

    /// Start an effect with a pre-calculated elapsed time (for seeking)
    /// This sets the effect's start_time to be in the past so it appears at the correct point in its lifecycle
    pub fn start_effect_with_elapsed(
        &mut self,
        mut effect: EffectInstance,
        elapsed_time: Duration,
    ) -> Result<(), EffectError> {
        // Validate effect
        validation::validate_effect(self.fixtures.as_map(), &effect)?;

        // Log effect parameters
        let (effect_kind, effect_params) = Self::format_effect_for_logging(&effect);
        debug!(
            effect_id = %effect.id,
            effect_kind,
            effect_params = %effect_params,
            layer = ?effect.layer,
            blend_mode = ?effect.blend_mode,
            priority = effect.priority,
            up_time = ?effect.up_time,
            hold_time = ?effect.hold_time,
            down_time = ?effect.down_time,
            targets = ?effect.target_fixtures,
            elapsed_time = ?elapsed_time,
            "Starting lighting effect with elapsed time"
        );

        // Set the start time to be in the past by the elapsed amount
        effect.start_time = Some(
            self.current_time
                .checked_sub(elapsed_time)
                .unwrap_or(self.current_time),
        );
        self.active_effects.insert(effect.id.clone(), effect);
        self.cache.invalidate();
        Ok(())
    }

    /// Update the engine and return DMX commands
    /// song_time is the current song time (score-time) for tempo-aware effect completion checks
    pub fn update(
        &mut self,
        dt: Duration,
        song_time: Option<Duration>,
    ) -> Result<&[DmxCommand], EffectError> {
        self.update_subphase.store(10, Ordering::Relaxed);
        self.current_time += dt;
        self.engine_elapsed += dt;
        self.last_song_time = song_time;

        // Fast path for MIDI-DMX-only frames: when no DSL effects are running,
        // generate DmxCommands directly from the store. This skips all HashMap
        // cloning, fixture state rebuilding, and the full pipeline.
        if !self.cache.dirty && self.active_effects.is_empty() && self.releasing_effects.is_empty()
        {
            let store_gen = self
                .midi_dmx_store
                .as_ref()
                .map(|s| s.read().generation())
                .unwrap_or(0);

            // Unchanged since last frame — return cached commands.
            if store_gen == self.cache.last_store_generation {
                self.update_subphase.store(0, Ordering::Relaxed);
                return Ok(self.cache.get_cached());
            }

            // Store changed — rebuild commands directly from store (no intermediate
            // active effects pipeline).
            let mut commands = Vec::new();
            self.last_merged_states.clear();

            if let Some(ref store) = self.midi_dmx_store {
                let store = store.read();
                for (slot_idx, normalized_value) in store.iter_active() {
                    let (fixture_name, channel_name) = store.fixture_info(slot_idx);
                    if let Some(fixture_info) = self.fixtures.get(fixture_name) {
                        if let Some(&offset) = fixture_info.channels.get(channel_name) {
                            let dmx_channel = fixture_info.address + offset - 1;
                            let dmx_value = (normalized_value * 255.0) as u8;
                            commands.push(DmxCommand {
                                universe: fixture_info.universe,
                                channel: dmx_channel,
                                value: dmx_value,
                            });
                            // Update merged state for simulator visibility.
                            self.last_merged_states
                                .entry(fixture_name.clone())
                                .or_default()
                                .set_channel(
                                    channel_name.clone(),
                                    ChannelState::new(
                                        normalized_value,
                                        EffectLayer::Background,
                                        BlendMode::Replace,
                                    ),
                                );
                        }
                    }
                }
            }

            self.cache.update(commands, store_gen);
            self.update_subphase.store(0, Ordering::Relaxed);
            return Ok(self.cache.get_cached());
        }

        // Cache-only fast path: effects existed previously but are now done,
        // permanent state exists, and nothing has changed.
        if !self.cache.dirty && self.active_effects.is_empty() && self.releasing_effects.is_empty()
        {
            let store_gen = self
                .midi_dmx_store
                .as_ref()
                .map(|s| s.read().generation())
                .unwrap_or(0);
            if store_gen == self.cache.last_store_generation {
                self.update_subphase.store(0, Ordering::Relaxed);
                return Ok(self.cache.get_cached());
            }
        }

        self.update_subphase.store(20, Ordering::Relaxed);

        // Use song_time for tempo-aware speed lookups (BPM at current position).
        // Falls back to engine_elapsed when no song is playing.
        let absolute_time = song_time.unwrap_or(self.engine_elapsed);

        // Start with empty fixture states — no persistent state carried between frames
        let mut current_fixture_states = HashMap::new();

        self.update_subphase.store(30, Ordering::Relaxed);

        // Inject MIDI DMX values from the lockless store (lowest priority).
        if let Some(ref store) = self.midi_dmx_store {
            let store = store.read();
            for (slot_idx, normalized_value) in store.iter_active() {
                let (fixture_name, channel_name) = store.fixture_info(slot_idx);
                if self.fixtures.contains(fixture_name) {
                    let state = current_fixture_states
                        .entry(fixture_name.clone())
                        .or_insert_with(FixtureState::new);
                    if !state.channels.contains_key(channel_name) {
                        state.set_channel(
                            channel_name.clone(),
                            ChannelState::new(
                                normalized_value,
                                EffectLayer::Background,
                                BlendMode::Replace,
                            ),
                        );
                    }
                }
            }
        }

        self.update_subphase.store(40, Ordering::Relaxed);

        // Group effects by layer - collect effect IDs first to avoid borrowing conflicts
        // Within each layer, we will sort effects deterministically so that:
        // - Higher priority effects are processed after lower priority ones
        // - For equal priority, later-started effects are processed after earlier ones
        // This ensures consistent layering behavior between runs and avoids
        // HashMap iteration order affecting visual output.
        let mut effects_by_layer: std::collections::BTreeMap<EffectLayer, Vec<String>> =
            std::collections::BTreeMap::new();

        for (effect_id, effect) in &self.active_effects {
            if effect.enabled {
                effects_by_layer
                    .entry(effect.layer)
                    .or_default()
                    .push(effect_id.clone());
            }
        }

        // Sort effect IDs within each layer by (priority, start_time, cue_time, id)
        // Using cue_time ensures deterministic ordering when multiple effects start at the same time
        for (_layer, effect_ids) in effects_by_layer.iter_mut() {
            effect_ids.sort_by(|a, b| {
                let ea = self.active_effects.get(a).unwrap();
                let eb = self.active_effects.get(b).unwrap();

                ea.priority
                    .cmp(&eb.priority)
                    .then_with(|| {
                        // Effects without a start_time are treated as earliest
                        let sa = ea.start_time;
                        let sb = eb.start_time;
                        match (sa, sb) {
                            (Some(ta), Some(tb)) => ta.cmp(&tb),
                            (None, Some(_)) => std::cmp::Ordering::Less,
                            (Some(_), None) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    })
                    .then_with(|| {
                        // Use cue_time for deterministic ordering when effects have same priority and start_time
                        // This ensures effects at the same time are always processed in the same order
                        match (ea.cue_time, eb.cue_time) {
                            (Some(ca), Some(cb)) => ca.cmp(&cb),
                            (None, Some(_)) => std::cmp::Ordering::Less,
                            (Some(_), None) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    })
                    .then_with(|| a.cmp(b))
            });
        }

        self.update_subphase.store(50, Ordering::Relaxed);

        // Track effects that have just completed to preserve their final state
        let mut completed_effects = Vec::new();

        // Process each layer in order
        for (layer, effect_ids) in effects_by_layer {
            // Get layer masters
            let layer_intensity = self.get_layer_intensity_master(layer);
            let layer_speed = self.get_layer_speed_master(layer);
            let frozen_at = self.layer_state.frozen.get(&layer).cloned();

            for effect_id in effect_ids {
                // Get reference to effect to avoid unnecessary clone
                let effect = self.active_effects.get(&effect_id).unwrap();

                // Check if this effect is being released
                let release_info = self.releasing_effects.get(&effect_id).cloned();

                // Calculate base elapsed time
                // If layer is frozen, use the frozen time instead of current time
                let reference_time = frozen_at.unwrap_or(self.current_time);
                let base_elapsed = effect
                    .start_time
                    .map(|start| reference_time.duration_since(start))
                    .unwrap_or(Duration::ZERO);

                // Apply speed master to elapsed time
                // Speed 0.0 triggers freeze_layer, and frozen_at provides the frozen reference time.
                // We use base_elapsed directly for both 0.0 and 1.0 (no multiplication needed).
                let elapsed = if (layer_speed - 1.0).abs() < f64::EPSILON || layer_speed == 0.0 {
                    base_elapsed
                } else {
                    // Multiply duration by speed factor
                    Duration::from_secs_f64(base_elapsed.as_secs_f64() * layer_speed)
                };

                // Check if effect has reached terminal state (value-based where applicable)
                // For effects with cue_time, use score-time elapsed instead of real-time elapsed
                // because hold_time/up_time/down_time are calculated in score-time
                let is_expired = if effect.start_time.is_some() {
                    if let (Some(cue_time), Some(current_song_time)) = (effect.cue_time, song_time)
                    {
                        // Use score-time elapsed for tempo-aware completion
                        // This ensures effects complete at the correct musical time, not real-time
                        let score_elapsed = current_song_time.saturating_sub(cue_time);
                        effect.has_reached_terminal_state(score_elapsed)
                    } else {
                        // Fall back to real-time elapsed for effects without cue_time
                        effect.has_reached_terminal_state(elapsed)
                    }
                } else {
                    false
                };

                // Check if a releasing effect has completed its fade
                let release_completed = if let Some((fade_time, release_start)) = &release_info {
                    let release_elapsed = self.current_time.duration_since(*release_start);
                    release_elapsed >= *fade_time
                } else {
                    false
                };

                if is_expired || release_completed {
                    // Effect has completed. For temporary effects, do not blend final state.
                    // For permanent effects, preserve via the completion handler below.

                    // Queue for removal after this frame
                    completed_effects.push(effect_id.clone());
                    continue;
                }

                // Process the effect and get fixture states
                if let Some(mut effect_states) = processing::process_effect(
                    self.fixtures.as_map(),
                    effect,
                    elapsed,
                    absolute_time,
                    self.tempo_map.as_ref(),
                )? {
                    // Calculate release fade multiplier if this effect is being released
                    let release_multiplier = if let Some((fade_time, release_start)) = release_info
                    {
                        let release_elapsed = self.current_time.duration_since(release_start);
                        let progress = if fade_time.is_zero() {
                            1.0
                        } else {
                            (release_elapsed.as_secs_f64() / fade_time.as_secs_f64())
                                .clamp(0.0, 1.0)
                        };
                        1.0 - progress // Fade from 1.0 to 0.0
                    } else {
                        1.0
                    };

                    // Combined intensity multiplier (layer master * release fade)
                    let intensity_multiplier = layer_intensity * release_multiplier;

                    // Apply intensity multiplier to effect states if not 1.0
                    if (intensity_multiplier - 1.0).abs() > f64::EPSILON {
                        for fixture_state in effect_states.values_mut() {
                            for channel_state in fixture_state.channels.values_mut() {
                                channel_state.value *= intensity_multiplier;
                            }
                        }
                    }

                    // Blend the effect states into the current fixture states
                    for (fixture_name, effect_state) in effect_states {
                        if self.fixtures.contains(&fixture_name) {
                            current_fixture_states
                                .entry(fixture_name.clone())
                                .or_insert_with(FixtureState::new)
                                .blend_with(&effect_state);
                        }
                    }
                }
            }
        }

        self.update_subphase.store(60, Ordering::Relaxed);

        // Handle completed effects — simply remove them. No state persists after completion.
        for effect_id in completed_effects {
            self.releasing_effects.remove(&effect_id);

            if let Some(effect) = self.active_effects.remove(&effect_id) {
                // Clean up per-layer multipliers for completed effects
                let dimmer_key = multiplier_key("dimmer", effect.layer);
                let pulse_key = multiplier_key("pulse", effect.layer);

                for fixture_name in &effect.target_fixtures {
                    if let Some(current_state) = current_fixture_states.get_mut(fixture_name) {
                        current_state.channels.remove(&dimmer_key);
                        current_state.channels.remove(&pulse_key);
                    }
                }
            }
        }

        self.update_subphase.store(70, Ordering::Relaxed);

        // Use current frame states directly — no persistent state merge needed

        self.update_subphase.store(80, Ordering::Relaxed);

        // Store states for preview/debugging (before converting to DMX)
        self.last_merged_states = current_fixture_states.clone();

        // Convert fixture states to DMX commands.
        let mut commands = Vec::new();
        for (fixture_name, fixture_state) in current_fixture_states {
            if let Some(fixture_info) = self.fixtures.get(&fixture_name) {
                commands.extend(fixture_state.to_dmx_commands(fixture_info));
            }
        }

        // Cache commands and store generation for fast-path short-circuit on
        // subsequent frames where nothing changes.
        let gen = self.store_generation();
        self.cache.update(commands, gen);

        self.update_subphase.store(0, Ordering::Relaxed);
        Ok(self.cache.get_cached())
    }

    /// Stop all active effects
    pub fn stop_all_effects(&mut self) {
        self.active_effects.clear();
        self.releasing_effects.clear();
        self.last_merged_states.clear();
        // Clear MIDI DMX values so they don't bleed into the next song.
        // Uses a read lock because MidiDmxStore uses interior mutability
        // (atomics) — no write lock needed.
        if let Some(ref store) = self.midi_dmx_store {
            store.read().clear();
        }
        self.cache.invalidate();
    }

    /// Stop all effects from a specific sequence
    /// Effects from sequences have IDs starting with "seq_{sequence_name}_"
    pub fn stop_sequence(&mut self, sequence_name: &str) {
        let prefix = format!("seq_{}_", sequence_name);
        let mut to_remove = Vec::new();

        // Collect effect IDs that match this sequence
        for effect_id in self.active_effects.keys() {
            if effect_id.starts_with(&prefix) {
                to_remove.push(effect_id.clone());
            }
        }

        // Remove the effects
        for effect_id in to_remove {
            self.active_effects.remove(&effect_id);
            self.releasing_effects.remove(&effect_id);
        }
        self.cache.invalidate();
    }

    // ===== Layer Control Methods (grandMA-inspired) =====

    /// Clear a layer - immediately stops all effects on the specified layer
    /// This is equivalent to a "kill" or panic button for a layer
    pub fn clear_layer(&mut self, layer: EffectLayer) {
        layers::clear_layer(
            &mut self.active_effects,
            &mut self.releasing_effects,
            &mut self.layer_state.frozen,
            layer,
        );
        self.last_merged_states.clear();
        self.cache.invalidate();
    }

    /// Clear all layers - immediately stops all effects on all layers
    /// This is equivalent to a "kill all" or panic button for everything
    pub fn clear_all_layers(&mut self) {
        layers::clear_all_layers(
            &mut self.active_effects,
            &mut self.releasing_effects,
            &mut self.layer_state.frozen,
        );
        self.last_merged_states.clear();
        self.cache.invalidate();
    }

    /// Release a layer - gracefully fades out all effects on the specified layer
    /// Uses each effect's down_time, or a default of 1 second if not specified
    pub fn release_layer(&mut self, layer: EffectLayer) {
        self.release_layer_with_time(layer, None);
    }

    /// Release a layer with a custom fade time
    /// If fade_time is None, uses each effect's down_time (or 1 second default)
    pub fn release_layer_with_time(&mut self, layer: EffectLayer, fade_time: Option<Duration>) {
        layers::release_layer_with_time(
            &mut self.active_effects,
            &mut self.releasing_effects,
            &mut self.layer_state.frozen,
            layer,
            fade_time,
            self.current_time,
        );
        self.cache.invalidate();
    }

    /// Freeze a layer - pauses all effects on the layer at their current state
    /// Effects maintain their current output values but don't advance in time
    pub fn freeze_layer(&mut self, layer: EffectLayer) {
        layers::freeze_layer(
            &mut self.layer_state.frozen,
            &mut self.active_effects,
            layer,
            self.current_time,
        );
        self.cache.invalidate();
    }

    /// Unfreeze a layer - resumes effects on the layer from where they left off
    pub fn unfreeze_layer(&mut self, layer: EffectLayer) {
        layers::unfreeze_layer(
            &mut self.layer_state.frozen,
            &mut self.active_effects,
            layer,
            self.current_time,
        );
        self.cache.invalidate();
    }

    /// Check if a layer is frozen
    #[cfg(test)]
    pub fn is_layer_frozen(&self, layer: EffectLayer) -> bool {
        self.layer_state.frozen.contains_key(&layer)
    }

    // ===== Layer Master Methods =====

    /// Set the intensity master for a layer (0.0 to 1.0)
    /// This multiplies with all effect outputs on the layer
    pub fn set_layer_intensity_master(&mut self, layer: EffectLayer, intensity: f64) {
        layers::set_layer_intensity_master(
            &mut self.layer_state.intensity_masters,
            layer,
            intensity,
        );
        self.cache.invalidate();
    }

    /// Get the intensity master for a layer (defaults to 1.0)
    pub fn get_layer_intensity_master(&self, layer: EffectLayer) -> f64 {
        self.layer_state.intensity_master(&layer)
    }

    /// Set the speed master for a layer (0.0+ where 1.0 = normal speed)
    /// This multiplies with effect speeds on the layer
    /// 0.5 = half speed, 2.0 = double speed, 0.0 = frozen at current state
    pub fn set_layer_speed_master(&mut self, layer: EffectLayer, speed: f64) {
        layers::set_layer_speed_master(
            &mut self.layer_state.speed_masters,
            &mut self.layer_state.frozen,
            &mut self.active_effects,
            layer,
            speed,
            self.current_time,
        );
        self.cache.invalidate();
    }

    /// Get the speed master for a layer (defaults to 1.0)
    pub fn get_layer_speed_master(&self, layer: EffectLayer) -> f64 {
        self.layer_state.speed_master(&layer)
    }

    /// Dispatch a single layer command to the appropriate engine method.
    pub fn apply_layer_command(&mut self, cmd: &crate::lighting::parser::LayerCommand) {
        use crate::lighting::parser::LayerCommandType;
        match cmd.command_type {
            LayerCommandType::Clear => {
                if let Some(layer) = cmd.layer {
                    self.clear_layer(layer);
                } else {
                    self.clear_all_layers();
                }
            }
            LayerCommandType::Release => {
                if let Some(layer) = cmd.layer {
                    if let Some(fade_time) = cmd.fade_time {
                        self.release_layer_with_time(layer, Some(fade_time));
                    } else {
                        self.release_layer(layer);
                    }
                }
            }
            LayerCommandType::Freeze => {
                if let Some(layer) = cmd.layer {
                    self.freeze_layer(layer);
                }
            }
            LayerCommandType::Unfreeze => {
                if let Some(layer) = cmd.layer {
                    self.unfreeze_layer(layer);
                }
            }
            LayerCommandType::Master => {
                if let Some(layer) = cmd.layer {
                    if let Some(intensity) = cmd.intensity {
                        self.set_layer_intensity_master(layer, intensity);
                    }
                    if let Some(speed) = cmd.speed {
                        self.set_layer_speed_master(layer, speed);
                    }
                }
            }
        }
    }

    /// Get the number of active effects
    #[cfg(test)]
    pub fn active_effects_count(&self) -> usize {
        self.active_effects.len()
    }

    /// Check if a specific effect is active
    #[cfg(test)]
    pub fn has_effect(&self, effect_id: &str) -> bool {
        self.active_effects.contains_key(effect_id)
    }

    /// Get all active effects (for debugging/simulation)
    #[allow(dead_code)]
    pub fn get_active_effects(&self) -> &HashMap<String, EffectInstance> {
        &self.active_effects
    }

    #[allow(dead_code)]
    /// Get current fixture states (for debugging/simulation/preview)
    /// Returns the merged fixture states from the last update() call.
    /// This provides a snapshot of all fixture states without generating DMX commands.
    pub fn get_fixture_states(&self) -> HashMap<String, FixtureState> {
        self.last_merged_states.clone()
    }

    /// Get the fixture registry (for simulator to determine fixture capabilities).
    #[allow(dead_code)]
    pub fn get_fixture_registry(&self) -> &HashMap<String, FixtureInfo> {
        self.fixtures.as_map()
    }

    /// Get a formatted string listing all active effects
    pub fn format_active_effects(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();

        if self.active_effects.is_empty() {
            return "No active effects".to_string();
        }

        writeln!(output, "Active effects ({}):", self.active_effects.len()).unwrap();

        // Group effects by layer for better readability
        let mut effects_by_layer: std::collections::HashMap<EffectLayer, Vec<&EffectInstance>> =
            std::collections::HashMap::new();

        for effect in self.active_effects.values() {
            effects_by_layer
                .entry(effect.layer)
                .or_default()
                .push(effect);
        }

        // Sort layers for consistent output
        let mut layers: Vec<_> = effects_by_layer.keys().collect();
        layers.sort();

        for layer in layers {
            let effects = &effects_by_layer[layer];
            // Print layer name
            let layer_name = match layer {
                EffectLayer::Background => "Background",
                EffectLayer::Midground => "Midground",
                EffectLayer::Foreground => "Foreground",
            };
            writeln!(output, "  {}:", layer_name).unwrap();
            for effect in effects {
                let elapsed = if let Some(start_time) = effect.start_time {
                    self.current_time.duration_since(start_time)
                } else {
                    Duration::ZERO
                };

                let effect_type_str = match &effect.effect_type {
                    EffectType::Static { .. } => "Static",
                    EffectType::ColorCycle { .. } => "ColorCycle",
                    EffectType::Strobe { .. } => "Strobe",
                    EffectType::Dimmer { .. } => "Dimmer",
                    EffectType::Chase { .. } => "Chase",
                    EffectType::Rainbow { .. } => "Rainbow",
                    EffectType::Pulse { .. } => "Pulse",
                };

                let total = effect.total_duration();
                let duration_str = format!(" (duration: {:.2}s)", total.as_secs_f64());

                writeln!(
                    output,
                    "    - {} [{}] - {} fixture(s) - elapsed: {:.2}s{}",
                    effect.id,
                    effect_type_str,
                    effect.target_fixtures.len(),
                    elapsed.as_secs_f64(),
                    duration_str
                )
                .unwrap();
            }
        }

        // Also show releasing effects if any
        if !self.releasing_effects.is_empty() {
            writeln!(
                output,
                "\nReleasing effects ({}):",
                self.releasing_effects.len()
            )
            .unwrap();
            for (effect_id, (fade_time, release_start)) in &self.releasing_effects {
                let release_elapsed = self.current_time.duration_since(*release_start);
                writeln!(
                    output,
                    "    - {} - fading out (elapsed: {:.2}s / {:.2}s)",
                    effect_id,
                    release_elapsed.as_secs_f64(),
                    fade_time.as_secs_f64()
                )
                .unwrap();
            }
        }

        output
    }
}
