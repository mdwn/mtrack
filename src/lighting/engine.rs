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

mod layers;
mod processing;
mod validation;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::effects::*;
use super::tempo::TempoMap;
use tracing::info;

/// The main effects engine that manages and processes lighting effects
pub struct EffectEngine {
    active_effects: HashMap<String, EffectInstance>,
    fixture_registry: HashMap<String, FixtureInfo>,
    current_time: Instant,
    /// Elapsed simulated time since engine start
    engine_elapsed: Duration,
    /// Persistent fixture states - maintains the current state of each fixture
    fixture_states: HashMap<String, FixtureState>,
    /// Channel locks - prevents lower-layer effects from affecting locked channels
    channel_locks: HashMap<String, std::collections::HashSet<String>>,
    /// Optional tempo map for tempo-aware effects (measure/beat-based timing)
    tempo_map: Option<TempoMap>,
    /// Layer intensity masters (0.0 to 1.0) - multiplies effect output per layer
    layer_intensity_masters: HashMap<EffectLayer, f64>,
    /// Layer speed masters (0.0+) - multiplies effect speed per layer (1.0 = normal)
    layer_speed_masters: HashMap<EffectLayer, f64>,
    /// Frozen layers - maps layer to the Instant when it was frozen
    /// Effects on frozen layers use this time instead of current_time for elapsed calculation
    frozen_layers: HashMap<EffectLayer, Instant>,
    /// Effects being released - tracks (release_fade_time, release_start_time) per effect
    releasing_effects: HashMap<String, (Duration, Instant)>,
}

impl EffectEngine {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            fixture_registry: HashMap::new(),
            current_time: Instant::now(),
            engine_elapsed: Duration::ZERO,
            fixture_states: HashMap::new(),
            channel_locks: HashMap::new(),
            tempo_map: None,
            layer_intensity_masters: HashMap::new(),
            layer_speed_masters: HashMap::new(),
            frozen_layers: HashMap::new(),
            releasing_effects: HashMap::new(),
        }
    }

    /// Set the tempo map for tempo-aware effects
    pub fn set_tempo_map(&mut self, tempo_map: Option<TempoMap>) {
        self.tempo_map = tempo_map;
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

        self.fixture_registry.insert(fixture.name.clone(), fixture);
    }

    /// Start an effect
    pub fn start_effect(&mut self, mut effect: EffectInstance) -> Result<(), EffectError> {
        // Validate effect
        validation::validate_effect(&self.fixture_registry, &effect)?;

        // Log effect parameters once when the effect is started
        // This captures the configuration that will guide execution.
        let (effect_kind, effect_params) = match &effect.effect_type {
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
                transition: _,
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
        };

        info!(
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

        // Stop any conflicting effects
        layers::stop_conflicting_effects(&mut self.active_effects, &effect, &self.fixture_registry);

        // Set the start time to the current engine time
        effect.start_time = Some(self.current_time);
        self.active_effects.insert(effect.id.clone(), effect);
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
        validation::validate_effect(&self.fixture_registry, &effect)?;

        // Log effect parameters once when the effect is started
        let (effect_kind, effect_params) = match &effect.effect_type {
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
                transition: _,
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
        };

        info!(
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

        // Stop any conflicting effects
        layers::stop_conflicting_effects(&mut self.active_effects, &effect, &self.fixture_registry);

        // Set the start time to be in the past by the elapsed amount
        // This makes the effect appear at the correct point in its lifecycle
        effect.start_time = Some(
            self.current_time
                .checked_sub(elapsed_time)
                .unwrap_or(self.current_time),
        );
        self.active_effects.insert(effect.id.clone(), effect);
        Ok(())
    }

    /// Update the engine and return DMX commands
    pub fn update(&mut self, dt: Duration) -> Result<Vec<DmxCommand>, EffectError> {
        self.current_time += dt;
        self.engine_elapsed += dt;

        // Start with only states from permanent effects as the base
        let mut current_fixture_states = HashMap::new();

        // Always include persisted permanent states as the base
        for (fixture_name, state) in &self.fixture_states {
            current_fixture_states.insert(fixture_name.clone(), state.clone());
        }

        // Track which channels come from permanent effects to preserve them later
        let mut permanent_channels: HashMap<String, std::collections::HashSet<String>> =
            current_fixture_states
                .iter()
                .map(|(name, state)| (name.clone(), state.channels.keys().cloned().collect()))
                .collect();

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

        // Sort effect IDs within each layer by (priority, start_time, id)
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
                    .then_with(|| a.cmp(b))
            });
        }

        // Track effects that have just completed to preserve their final state
        let mut completed_effects = Vec::new();

        // Process each layer in order
        for (layer, effect_ids) in effects_by_layer {
            // Get layer masters
            let layer_intensity = self.get_layer_intensity_master(layer);
            let layer_speed = self.get_layer_speed_master(layer);
            let frozen_at = self.frozen_layers.get(&layer).cloned();

            for effect_id in effect_ids {
                // Clone the effect to avoid borrowing conflicts
                let effect = self.active_effects.get(&effect_id).unwrap().clone();

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
                let elapsed = if layer_speed == 0.0 || layer_speed == 1.0 {
                    base_elapsed
                } else {
                    Duration::from_secs_f64(base_elapsed.as_secs_f64() * layer_speed)
                };

                // Check if effect has reached terminal state (value-based where applicable)
                let is_expired = if effect.start_time.is_some() {
                    effect.has_reached_terminal_state(elapsed)
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
                    &self.fixture_registry,
                    &effect,
                    elapsed,
                    self.engine_elapsed,
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
                        if self.fixture_registry.contains_key(&fixture_name) {
                            // Check if any channels are locked for this fixture
                            let locked_channels = self.channel_locks.get(&fixture_name);

                            // Filter out locked channels from the effect state
                            let mut filtered_state = effect_state.clone();
                            if let Some(locked) = locked_channels {
                                // Remove locked channels from the effect state, but always allow
                                // brightness/pulse multipliers to pass through
                                filtered_state.channels.retain(|channel_name, _| {
                                    channel_name.starts_with("_dimmer_mult")
                                        || channel_name.starts_with("_pulse_mult")
                                        || channel_name == "dimmer"
                                        || !locked.contains(channel_name)
                                });
                            }

                            // Only blend if there are unlocked channels
                            if !filtered_state.channels.is_empty() {
                                current_fixture_states
                                    .entry(fixture_name.clone())
                                    .or_insert_with(FixtureState::new)
                                    .blend_with(&filtered_state);

                                // Do not mark permanent channels during active frames; persist only on completion
                            }
                        }
                    }
                }
            }
        }

        // Handle completed effects by preserving their final state
        for effect_id in completed_effects {
            // Clean up releasing effects tracking
            self.releasing_effects.remove(&effect_id);

            if let Some(effect) = self.active_effects.remove(&effect_id) {
                // Calculate the final state of the completed effect
                if let Some(final_states) = self.process_effect_final_state(&effect)? {
                    // Only preserve the final state for permanent effects
                    if effect.is_permanent() {
                        // Preserve the final state in persistent storage
                        for (fixture_name, final_state) in final_states {
                            if self.fixture_registry.contains_key(&fixture_name) {
                                let fixture_name_clone = fixture_name.clone();
                                current_fixture_states
                                    .entry(fixture_name_clone.clone())
                                    .or_insert_with(FixtureState::new)
                                    .blend_with(&final_state);

                                // Lock channels if this is a foreground Replace effect
                                if effect.layer == EffectLayer::Foreground
                                    && effect.blend_mode == BlendMode::Replace
                                {
                                    let locked_channels =
                                        self.channel_locks.entry(fixture_name_clone).or_default();

                                    // Lock all channels that this effect affected
                                    for channel_name in final_state.channels.keys() {
                                        locked_channels.insert(channel_name.clone());
                                    }
                                }

                                // Ensure channels from this permanent effect are saved
                                let entry =
                                    permanent_channels.entry(fixture_name.clone()).or_default();
                                // Include original final channels
                                for ch in final_state.channels.keys() {
                                    entry.insert(ch.clone());
                                }
                                // Also include any per-layer multiplier channels materialized by blend_with
                                if let Some(cur) = current_fixture_states.get(&fixture_name) {
                                    for ch in cur.channels.keys() {
                                        if ch.starts_with("_dimmer_mult")
                                            || ch.starts_with("_pulse_mult")
                                        {
                                            entry.insert(ch.clone());
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        // Temporary effects complete and end — remove per-layer multipliers for this effect's layer
                        for (fixture_name, _final_state) in final_states {
                            if self.fixture_registry.contains_key(&fixture_name) {
                                // Identify the layer suffix for this effect
                                let layer_suffix = match effect.layer {
                                    EffectLayer::Background => "_bg",
                                    EffectLayer::Midground => "_mid",
                                    EffectLayer::Foreground => "_fg",
                                };

                                // Remove per-layer multipliers for this layer from current_fixture_states
                                if let Some(current_state) =
                                    current_fixture_states.get_mut(&fixture_name)
                                {
                                    // Remove dimmer multiplier for this layer (defaults to 1.0 at emission)
                                    let dimmer_key = format!("_dimmer_mult{}", layer_suffix);
                                    current_state.channels.remove(&dimmer_key);

                                    // Remove pulse multiplier for this layer (defaults to 1.0 at emission)
                                    let pulse_key = format!("_pulse_mult{}", layer_suffix);
                                    current_state.channels.remove(&pulse_key);
                                }

                                // Also remove from persisted fixture_states
                                if let Some(persisted_state) =
                                    self.fixture_states.get_mut(&fixture_name)
                                {
                                    let dimmer_key = format!("_dimmer_mult{}", layer_suffix);
                                    persisted_state.channels.remove(&dimmer_key);
                                    let pulse_key = format!("_pulse_mult{}", layer_suffix);
                                    persisted_state.channels.remove(&pulse_key);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Update persistent fixture states - only save channels from permanent effects
        self.fixture_states.clear();
        for (fixture_name, state) in &current_fixture_states {
            if let Some(perm_channels) = permanent_channels.get(fixture_name) {
                // Only save channels that were from permanent effects
                let mut preserved_state = FixtureState::new();
                for channel_name in perm_channels {
                    if let Some(channel_state) = state.channels.get(channel_name) {
                        preserved_state
                            .channels
                            .insert(channel_name.clone(), *channel_state);
                    }
                }
                if !preserved_state.channels.is_empty() {
                    self.fixture_states
                        .insert(fixture_name.clone(), preserved_state);
                }
            }
        }

        // Merge current frame states with persisted permanent states for emission,
        // so permanent dimming (e.g., RGB multipliers) persists even when no effect is active.
        let mut merged_states: HashMap<String, FixtureState> = HashMap::new();
        for name in self.fixture_registry.keys() {
            match (
                current_fixture_states.get(name),
                self.fixture_states.get(name),
            ) {
                (Some(current), Some(persisted)) => {
                    // Start from persisted, then overlay current so current wins
                    let mut merged = persisted.clone();
                    merged.blend_with(current);
                    merged_states.insert(name.clone(), merged);
                }
                (Some(current), None) => {
                    merged_states.insert(name.clone(), current.clone());
                }
                (None, Some(persisted)) => {
                    merged_states.insert(name.clone(), persisted.clone());
                }
                (None, None) => {}
            }
        }

        // Convert fixture states to DMX commands
        let mut commands = Vec::new();
        for (fixture_name, fixture_state) in merged_states {
            if let Some(fixture_info) = self.fixture_registry.get(&fixture_name) {
                commands.extend(fixture_state.to_dmx_commands(fixture_info));
            }
        }

        Ok(commands)
    }

    /// Process the final state of a completed effect
    fn process_effect_final_state(
        &mut self,
        effect: &EffectInstance,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        if !effect.enabled {
            return Ok(None);
        }

        // Calculate the final state by processing the effect at its end time
        if effect.start_time.is_some() {
            if let Some(total_duration) = effect.total_duration() {
                let final_elapsed = total_duration;
                processing::process_effect(
                    &self.fixture_registry,
                    effect,
                    final_elapsed,
                    self.engine_elapsed,
                    self.tempo_map.as_ref(),
                )
            } else {
                // Indefinite effect - use current state
                processing::process_effect(
                    &self.fixture_registry,
                    effect,
                    Duration::ZERO,
                    self.engine_elapsed,
                    self.tempo_map.as_ref(),
                )
            }
        } else {
            // Effect has no timing, use current state
            processing::process_effect(
                &self.fixture_registry,
                effect,
                Duration::ZERO,
                self.engine_elapsed,
                self.tempo_map.as_ref(),
            )
        }
    }

    /// Stop all active effects
    pub fn stop_all_effects(&mut self) {
        self.active_effects.clear();
        self.releasing_effects.clear();
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
    }

    // ===== Layer Control Methods (grandMA-inspired) =====

    /// Clear a layer - immediately stops all effects on the specified layer
    /// This is equivalent to a "kill" or panic button for a layer
    pub fn clear_layer(&mut self, layer: EffectLayer) {
        layers::clear_layer(
            &mut self.active_effects,
            &mut self.releasing_effects,
            &mut self.frozen_layers,
            layer,
        );
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
            &mut self.frozen_layers,
            layer,
            fade_time,
            self.current_time,
        );
    }

    /// Freeze a layer - pauses all effects on the layer at their current state
    /// Effects maintain their current output values but don't advance in time
    pub fn freeze_layer(&mut self, layer: EffectLayer) {
        layers::freeze_layer(
            &mut self.frozen_layers,
            &mut self.active_effects,
            layer,
            self.current_time,
        );
    }

    /// Unfreeze a layer - resumes effects on the layer from where they left off
    pub fn unfreeze_layer(&mut self, layer: EffectLayer) {
        layers::unfreeze_layer(
            &mut self.frozen_layers,
            &mut self.active_effects,
            layer,
            self.current_time,
        );
    }

    /// Check if a layer is frozen
    #[cfg(test)]
    pub fn is_layer_frozen(&self, layer: EffectLayer) -> bool {
        self.frozen_layers.contains_key(&layer)
    }

    // ===== Layer Master Methods =====

    /// Set the intensity master for a layer (0.0 to 1.0)
    /// This multiplies with all effect outputs on the layer
    pub fn set_layer_intensity_master(&mut self, layer: EffectLayer, intensity: f64) {
        layers::set_layer_intensity_master(&mut self.layer_intensity_masters, layer, intensity);
    }

    /// Get the intensity master for a layer (defaults to 1.0)
    pub fn get_layer_intensity_master(&self, layer: EffectLayer) -> f64 {
        *self.layer_intensity_masters.get(&layer).unwrap_or(&1.0)
    }

    /// Set the speed master for a layer (0.0+ where 1.0 = normal speed)
    /// This multiplies with effect speeds on the layer
    /// 0.5 = half speed, 2.0 = double speed, 0.0 = frozen at current state
    pub fn set_layer_speed_master(&mut self, layer: EffectLayer, speed: f64) {
        layers::set_layer_speed_master(
            &mut self.layer_speed_masters,
            &mut self.frozen_layers,
            &mut self.active_effects,
            layer,
            speed,
            self.current_time,
        );
    }

    /// Get the speed master for a layer (defaults to 1.0)
    pub fn get_layer_speed_master(&self, layer: EffectLayer) -> f64 {
        *self.layer_speed_masters.get(&layer).unwrap_or(&1.0)
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

    /// Check if two blend modes are compatible (can layer together) - public for tests
    #[cfg(test)]
    pub fn blend_modes_are_compatible_public(&self, existing: BlendMode, new: BlendMode) -> bool {
        layers::blend_modes_are_compatible(existing, new)
    }
}
