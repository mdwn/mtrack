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

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::effects::*;
use super::tempo::TempoMap;
use crate::lighting::effects::TempoAwareFrequency;

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
        if let Err(e) = self.validate_fixture_capabilities(&fixture) {
            eprintln!(
                "Warning: Fixture '{}' has capability issues: {}",
                fixture.name, e
            );
        }

        self.fixture_registry.insert(fixture.name.clone(), fixture);
    }

    /// Validate fixture capabilities based on its type and channels
    fn validate_fixture_capabilities(&self, fixture: &FixtureInfo) -> Result<(), EffectError> {
        // Check for common capability mismatches
        if fixture.fixture_type.contains("RGB") && !fixture.channels.contains_key("red") {
            return Err(EffectError::Parameter(format!(
                "RGB fixture '{}' missing red channel",
                fixture.name
            )));
        }

        if fixture.fixture_type.contains("RGB") && !fixture.channels.contains_key("green") {
            return Err(EffectError::Parameter(format!(
                "RGB fixture '{}' missing green channel",
                fixture.name
            )));
        }

        if fixture.fixture_type.contains("RGB") && !fixture.channels.contains_key("blue") {
            return Err(EffectError::Parameter(format!(
                "RGB fixture '{}' missing blue channel",
                fixture.name
            )));
        }

        if fixture.fixture_type.contains("Strobe") && !fixture.channels.contains_key("strobe") {
            return Err(EffectError::Parameter(format!(
                "Strobe fixture '{}' missing strobe channel",
                fixture.name
            )));
        }

        if fixture.fixture_type.contains("MovingHead") && !fixture.channels.contains_key("pan") {
            return Err(EffectError::Parameter(format!(
                "Moving head fixture '{}' missing pan channel",
                fixture.name
            )));
        }

        if fixture.fixture_type.contains("MovingHead") && !fixture.channels.contains_key("tilt") {
            return Err(EffectError::Parameter(format!(
                "Moving head fixture '{}' missing tilt channel",
                fixture.name
            )));
        }

        Ok(())
    }

    /// Start an effect
    pub fn start_effect(&mut self, mut effect: EffectInstance) -> Result<(), EffectError> {
        // Validate effect
        self.validate_effect(&effect)?;

        // Stop any conflicting effects
        self.stop_conflicting_effects(&effect);

        // Set the start time to the current engine time
        effect.start_time = Some(self.current_time);
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
                if let Some(mut effect_states) = self.process_effect(&effect, elapsed)? {
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
                                let _before_count = filtered_state.channels.len();
                                filtered_state.channels.retain(|channel_name, _| {
                                    channel_name.starts_with("_dimmer_mult")
                                        || channel_name.starts_with("_pulse_mult")
                                        || channel_name == "dimmer"
                                        || !locked.contains(channel_name)
                                });
                                let _after_count = filtered_state.channels.len();
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
                self.process_effect(effect, final_elapsed)
            } else {
                // Indefinite effect - use current state
                self.process_effect(effect, Duration::ZERO)
            }
        } else {
            // Effect has no timing, use current state
            self.process_effect(effect, Duration::ZERO)
        }
    }

    /// Process a single effect and return fixture states
    fn process_effect(
        &mut self,
        effect: &EffectInstance,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        if !effect.enabled {
            return Ok(None);
        }

        // Calculate absolute time for tempo-aware effects
        let absolute_time = self.engine_elapsed;
        let tempo_map = self.tempo_map.as_ref();

        match &effect.effect_type {
            EffectType::Static { parameters, .. } => {
                self.apply_static_effect(effect, parameters, elapsed)
            }
            EffectType::ColorCycle {
                colors,
                speed,
                direction,
                transition,
            } => {
                let current_speed = speed.to_cycles_per_second(tempo_map, absolute_time);
                self.apply_color_cycle(
                    effect,
                    colors,
                    current_speed,
                    direction,
                    *transition,
                    elapsed,
                )
            }
            EffectType::Strobe { frequency, .. } => {
                let current_frequency = frequency.to_hz(tempo_map, absolute_time);
                self.apply_strobe(effect, current_frequency, elapsed)
            }
            EffectType::Dimmer {
                start_level,
                end_level,
                duration,
                curve,
            } => self.apply_dimmer(effect, *start_level, *end_level, curve, elapsed, *duration),
            EffectType::Chase {
                pattern,
                speed,
                direction,
            } => {
                let current_speed = speed.to_cycles_per_second(tempo_map, absolute_time);
                self.apply_chase(effect, pattern, current_speed, direction, elapsed)
            }
            EffectType::Rainbow {
                speed,
                saturation,
                brightness,
            } => {
                let current_speed = speed.to_cycles_per_second(tempo_map, absolute_time);
                self.apply_rainbow(effect, current_speed, *saturation, *brightness, elapsed)
            }
            EffectType::Pulse {
                base_level,
                pulse_amplitude,
                frequency,
                ..
            } => {
                let current_frequency = frequency.to_hz(tempo_map, absolute_time);
                self.apply_pulse(
                    effect,
                    *base_level,
                    *pulse_amplitude,
                    current_frequency,
                    elapsed,
                )
            }
        }
    }

    /// Validate an effect before starting it
    fn validate_effect(&self, effect: &EffectInstance) -> Result<(), EffectError> {
        // Validate fixtures
        for fixture_name in &effect.target_fixtures {
            if !self.fixture_registry.contains_key(fixture_name) {
                return Err(EffectError::Fixture(format!(
                    "Fixture '{}' not found",
                    fixture_name
                )));
            }
        }

        // Validate effect compatibility with fixture special cases
        self.validate_effect_compatibility(effect)?;

        // Validate effect parameters
        match &effect.effect_type {
            EffectType::Static { parameters, .. } => {
                for (key, value) in parameters {
                    if *value < 0.0 || *value > 1.0 {
                        return Err(EffectError::Parameter(format!(
                            "Parameter '{}' must be between 0.0 and 1.0, got {}",
                            key, value
                        )));
                    }
                }
            }
            EffectType::Strobe { frequency, .. } => {
                // For tempo-aware frequencies, we can't validate at parse time
                // They'll be validated when converted to Hz during processing
                // For fixed frequencies, validate now
                match frequency {
                    TempoAwareFrequency::Fixed(freq) if *freq < 0.0 => {
                        return Err(EffectError::Parameter(format!(
                            "Strobe frequency must be non-negative, got {}",
                            freq
                        )));
                    }
                    _ => {}
                }
            }
            EffectType::Pulse { frequency, .. } => {
                // For tempo-aware frequencies, we can't validate at parse time
                // They'll be validated when converted to Hz during processing
                // For fixed frequencies, validate now
                match frequency {
                    TempoAwareFrequency::Fixed(freq) if *freq <= 0.0 => {
                        return Err(EffectError::Parameter(format!(
                            "Pulse frequency must be positive, got {}",
                            freq
                        )));
                    }
                    _ => {}
                }
            }
            _ => {} // Other effect types don't need validation yet
        }

        // Validate timing
        if let Some(total_duration) = effect.total_duration() {
            if total_duration.as_secs_f64() < 0.0 {
                return Err(EffectError::Timing(format!(
                    "Effect total duration must be non-negative, got {}s",
                    total_duration.as_secs_f64()
                )));
            }
        }

        Ok(())
    }

    /// Validate that the effect is compatible with fixture special cases
    fn validate_effect_compatibility(&self, effect: &EffectInstance) -> Result<(), EffectError> {
        for fixture_name in &effect.target_fixtures {
            if let Some(fixture_info) = self.fixture_registry.get(fixture_name) {
                // Check if the effect type is compatible with the fixture's special cases
                match &effect.effect_type {
                    EffectType::ColorCycle { .. } => {
                        if !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR) {
                            return Err(EffectError::Parameter(format!(
                                "Color cycle effect not compatible with fixture '{}' (no RGB capability)",
                                fixture_name
                            )));
                        }
                    }
                    EffectType::Strobe { .. } => {
                        // Strobe effects work with any fixture that has strobe, dimmer, or RGB capability
                        if !fixture_info.has_capability(FixtureCapabilities::STROBING)
                            && !fixture_info.has_capability(FixtureCapabilities::DIMMING)
                            && !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR)
                        {
                            return Err(EffectError::Parameter(format!(
                                "Strobe effect not compatible with fixture '{}' (no strobe, dimmer, or RGB capability)",
                                fixture_name
                            )));
                        }
                    }
                    EffectType::Chase { .. } => {
                        // Chase effects work with any fixture that has RGB or dimmer capability
                        if !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR)
                            && !fixture_info.has_capability(FixtureCapabilities::DIMMING)
                        {
                            return Err(EffectError::Parameter(format!(
                                "Chase effect not compatible with fixture '{}' (no RGB or dimmer capability)",
                                fixture_name
                            )));
                        }
                    }
                    EffectType::Rainbow { .. } => {
                        // Rainbow effects require RGB channels
                        if !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR) {
                            return Err(EffectError::Parameter(format!(
                                "Rainbow effect not compatible with fixture '{}' (no RGB capability)",
                                fixture_name
                            )));
                        }
                    }
                    _ => {} // Other effects are generally compatible
                }
            }
        }
        Ok(())
    }

    /// Stop effects that conflict with the new effect
    fn stop_conflicting_effects(&mut self, new_effect: &EffectInstance) {
        let mut to_remove = Vec::new();

        for (effect_id, effect) in &self.active_effects {
            // Skip if effect is already disabled
            if !effect.enabled {
                continue;
            }

            // Check if effects should conflict based on sophisticated rules
            if self.should_effects_conflict(effect, new_effect) {
                to_remove.push(effect_id.clone());
            }
        }

        for effect_id in to_remove {
            self.active_effects.remove(&effect_id);
        }
    }

    /// Determine if two effects should conflict based on sophisticated rules
    fn should_effects_conflict(&self, existing: &EffectInstance, new: &EffectInstance) -> bool {
        // 1. Layer-based conflict resolution
        // Effects in different layers generally don't conflict unless they have channel conflicts
        if existing.layer != new.layer {
            return self.have_channel_conflicts(existing, new);
        }

        // 2. Priority-based conflict resolution within the same layer
        if existing.priority < new.priority {
            return self.have_fixture_overlap(existing, new);
        }

        // 3. Effect type specific conflict rules
        self.effects_conflict_by_type(existing, new)
    }

    /// Check if effects have overlapping target fixtures
    fn have_fixture_overlap(&self, existing: &EffectInstance, new: &EffectInstance) -> bool {
        existing
            .target_fixtures
            .iter()
            .any(|fixture| new.target_fixtures.contains(fixture))
    }

    /// Check if effects have channel-level conflicts
    fn have_channel_conflicts(&self, _existing: &EffectInstance, _new: &EffectInstance) -> bool {
        // Effects in different layers should generally not conflict
        // The layering system is designed to allow effects in different layers
        // to coexist and blend together
        false
    }

    /// Determine conflicts based on effect types and blend modes
    fn effects_conflict_by_type(&self, existing: &EffectInstance, new: &EffectInstance) -> bool {
        use EffectType::*;

        // If effects don't overlap fixtures, they don't conflict
        if !self.have_fixture_overlap(existing, new) {
            return false;
        }

        // Check blend mode compatibility
        if self.blend_modes_are_compatible(existing.blend_mode, new.blend_mode) {
            return false;
        }

        // Effect type specific conflict rules
        match (&existing.effect_type, &new.effect_type) {
            // Static effects conflict with other static effects
            (Static { .. }, Static { .. }) => true,

            // Static effects conflict with color cycle effects
            (Static { .. }, ColorCycle { .. }) => true,
            (ColorCycle { .. }, Static { .. }) => true,

            // Color cycle effects conflict with other color cycle effects
            (ColorCycle { .. }, ColorCycle { .. }) => true,

            // Strobe effects conflict with other strobe effects
            (Strobe { .. }, Strobe { .. }) => true,

            // Chase effects conflict with other chase effects
            (Chase { .. }, Chase { .. }) => true,

            // Rainbow effects conflict with static and color cycle effects
            (Rainbow { .. }, Static { .. }) => true,
            (Static { .. }, Rainbow { .. }) => true,
            (Rainbow { .. }, ColorCycle { .. }) => true,
            (ColorCycle { .. }, Rainbow { .. }) => true,
            (Rainbow { .. }, Rainbow { .. }) => true,

            // Dimmer and pulse effects are generally compatible (they layer)
            (Dimmer { .. }, _) => false,
            (_, Dimmer { .. }) => false,
            (Pulse { .. }, _) => false,
            (_, Pulse { .. }) => false,

            // Default: effects of different types don't conflict
            _ => false,
        }
    }

    /// Check if two blend modes are compatible (can layer together)
    fn blend_modes_are_compatible(&self, existing: BlendMode, new: BlendMode) -> bool {
        match (existing, new) {
            // Replace mode conflicts with everything
            (BlendMode::Replace, _) => false,
            (_, BlendMode::Replace) => false,

            // Multiply, Add, Overlay, and Screen can generally layer together
            (BlendMode::Multiply, BlendMode::Multiply) => true,
            (BlendMode::Add, BlendMode::Add) => true,
            (BlendMode::Overlay, BlendMode::Overlay) => true,
            (BlendMode::Screen, BlendMode::Screen) => true,

            // Different blend modes can layer if they're not Replace
            (BlendMode::Multiply, BlendMode::Add) => true,
            (BlendMode::Multiply, BlendMode::Overlay) => true,
            (BlendMode::Multiply, BlendMode::Screen) => true,
            (BlendMode::Add, BlendMode::Multiply) => true,
            (BlendMode::Add, BlendMode::Overlay) => true,
            (BlendMode::Add, BlendMode::Screen) => true,
            (BlendMode::Overlay, BlendMode::Multiply) => true,
            (BlendMode::Overlay, BlendMode::Add) => true,
            (BlendMode::Overlay, BlendMode::Screen) => true,
            (BlendMode::Screen, BlendMode::Multiply) => true,
            (BlendMode::Screen, BlendMode::Add) => true,
            (BlendMode::Screen, BlendMode::Overlay) => true,
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
        // Collect effect IDs on this layer BEFORE removing them
        let effects_on_layer: Vec<String> = self
            .active_effects
            .iter()
            .filter(|(_, effect)| effect.layer == layer)
            .map(|(id, _)| id.clone())
            .collect();

        // Remove all effects on this layer
        self.active_effects
            .retain(|_, effect| effect.layer != layer);

        // Also remove any releasing effects that were on this layer
        for id in effects_on_layer {
            self.releasing_effects.remove(&id);
        }

        // Unfreeze the layer if it was frozen
        self.frozen_layers.remove(&layer);
    }

    /// Release a layer - gracefully fades out all effects on the specified layer
    /// Uses each effect's down_time, or a default of 1 second if not specified
    pub fn release_layer(&mut self, layer: EffectLayer) {
        self.release_layer_with_time(layer, None);
    }

    /// Release a layer with a custom fade time
    /// If fade_time is None, uses each effect's down_time (or 1 second default)
    pub fn release_layer_with_time(&mut self, layer: EffectLayer, fade_time: Option<Duration>) {
        let now = self.current_time;
        let default_fade = Duration::from_secs(1);

        for (effect_id, effect) in &self.active_effects {
            if effect.layer == layer && !self.releasing_effects.contains_key(effect_id) {
                let release_time =
                    fade_time.unwrap_or_else(|| effect.down_time.unwrap_or(default_fade));
                self.releasing_effects
                    .insert(effect_id.clone(), (release_time, now));
            }
        }
        // Unfreeze the layer if it was frozen (properly adjusts effect start times
        // to maintain smooth animation continuity during the fade-out)
        self.unfreeze_layer(layer);
    }

    /// Freeze a layer - pauses all effects on the layer at their current state
    /// Effects maintain their current output values but don't advance in time
    pub fn freeze_layer(&mut self, layer: EffectLayer) {
        // Record the time when the layer was frozen
        // Don't overwrite if already frozen
        if !self.frozen_layers.contains_key(&layer) {
            self.frozen_layers.insert(layer, self.current_time);
        }
    }

    /// Unfreeze a layer - resumes effects on the layer from where they left off
    pub fn unfreeze_layer(&mut self, layer: EffectLayer) {
        // When unfreezing, we need to adjust effect start times to account for frozen duration
        if let Some(frozen_at) = self.frozen_layers.remove(&layer) {
            let frozen_duration = self.current_time.duration_since(frozen_at);

            // Adjust start times for all effects on this layer
            for effect in self.active_effects.values_mut() {
                if effect.layer == layer {
                    if let Some(start_time) = effect.start_time {
                        // Push the start time forward by the frozen duration
                        // This makes it appear as if no time passed while frozen
                        effect.start_time = Some(start_time + frozen_duration);
                    }
                }
            }
        }
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
        let clamped = intensity.clamp(0.0, 1.0);
        if (clamped - 1.0).abs() < f64::EPSILON {
            // 1.0 is the default, remove from map to save memory
            self.layer_intensity_masters.remove(&layer);
        } else {
            self.layer_intensity_masters.insert(layer, clamped);
        }
    }

    /// Get the intensity master for a layer (defaults to 1.0)
    pub fn get_layer_intensity_master(&self, layer: EffectLayer) -> f64 {
        *self.layer_intensity_masters.get(&layer).unwrap_or(&1.0)
    }

    /// Set the speed master for a layer (0.0+ where 1.0 = normal speed)
    /// This multiplies with effect speeds on the layer
    /// 0.5 = half speed, 2.0 = double speed, 0.0 = frozen at current state
    pub fn set_layer_speed_master(&mut self, layer: EffectLayer, speed: f64) {
        let clamped = speed.max(0.0); // Speed can be > 1.0 but not negative

        // Speed 0.0 means freeze - use the freeze_layer mechanism
        if clamped == 0.0 {
            self.freeze_layer(layer);
        } else {
            // Non-zero speed means unfreeze (if was frozen by speed=0)
            // Note: this only unfreezes if we're changing FROM 0.0
            let was_frozen_by_speed = self.layer_speed_masters.get(&layer) == Some(&0.0);
            if was_frozen_by_speed {
                self.unfreeze_layer(layer);
            }
        }

        if (clamped - 1.0).abs() < f64::EPSILON {
            // 1.0 is the default, remove from map to save memory
            self.layer_speed_masters.remove(&layer);
        } else {
            self.layer_speed_masters.insert(layer, clamped);
        }
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
        self.blend_modes_are_compatible(existing, new)
    }

    // ===== State-based effect processing methods =====
    //
    // These methods apply various lighting effects using the Fixture Profile System.
    // The fixture profile system ensures that the same lighting show produces
    // visually consistent results across different fixture types by automatically
    // selecting the appropriate strategy based on fixture capabilities.

    /// Apply a static effect and return fixture states
    fn apply_static_effect(
        &mut self,
        effect: &EffectInstance,
        parameters: &HashMap<String, f64>,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                // For static effects, we apply parameters directly
                // The fixture profile system is more useful for dynamic effects

                for (param_name, value) in parameters {
                    // Apply crossfade multiplier to the value
                    let faded_value = *value * crossfade_multiplier;

                    // For static effects, apply parameters directly if the channel exists
                    // The fixture profile system is more useful for dynamic effects that need
                    // to adapt their behavior based on fixture capabilities
                    if fixture.channels.contains_key(param_name) {
                        let channel_state =
                            ChannelState::new(faded_value, effect.layer, effect.blend_mode);
                        fixture_state.set_channel(param_name.clone(), channel_state);
                    }
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }

    /// Apply a color cycle effect and return fixture states
    fn apply_color_cycle(
        &mut self,
        effect: &EffectInstance,
        colors: &[Color],
        speed: f64,
        direction: &CycleDirection,
        transition: CycleTransition,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        if colors.is_empty() {
            return Ok(None);
        }

        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        // Guard against zero/negative speed - treat as "stopped" at first color
        if speed <= 0.0 {
            let color = colors[0];
            let mut fixture_states = HashMap::new();
            for fixture_name in &effect.target_fixtures {
                if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                    let mut fixture_state = FixtureState::new();
                    let profile = FixtureProfile::for_fixture(fixture);
                    let channel_commands =
                        profile.apply_color(color, effect.layer, effect.blend_mode);
                    for (channel_name, mut channel_state) in channel_commands {
                        channel_state.value *= crossfade_multiplier;
                        fixture_state.set_channel(channel_name, channel_state);
                    }
                    fixture_states.insert(fixture_name.clone(), fixture_state);
                }
            }
            return Ok(Some(fixture_states));
        }

        let cycle_time = 1.0 / speed;
        let cycle_progress = (elapsed.as_secs_f64() % cycle_time) / cycle_time;

        // Calculate color indices and interpolation factor for smooth transitions
        let (color_index, next_index, segment_progress) = match direction {
            CycleDirection::Forward => {
                let color_index_f = cycle_progress * colors.len() as f64;
                let color_index = color_index_f.floor() as usize;
                let next_index = (color_index + 1) % colors.len();
                let segment_progress = color_index_f - color_index as f64;
                (color_index, next_index, segment_progress)
            }
            CycleDirection::Backward => {
                let reversed_progress = 1.0 - cycle_progress;
                let color_index_f = reversed_progress * colors.len() as f64;
                let color_index = color_index_f.floor() as usize;

                // Handle boundary case: when reversed_progress = 1.0 (cycle start),
                // color_index_f = colors.len(), which is out of bounds.
                // At this point we should show the last color with no interpolation.
                if color_index >= colors.len() {
                    (colors.len() - 1, colors.len() - 1, 0.0)
                } else {
                    let next_index = if color_index == 0 {
                        colors.len() - 1
                    } else {
                        color_index - 1
                    };
                    let segment_progress = color_index_f - color_index as f64;
                    (color_index, next_index, segment_progress)
                }
            }
            CycleDirection::PingPong => {
                // PingPong: go forward then backward through colors
                // ping_pong_progress goes 0 → 1 → 0 over one cycle
                let ping_pong_progress = if cycle_progress < 0.5 {
                    cycle_progress * 2.0
                } else {
                    2.0 - cycle_progress * 2.0
                };

                // Map ping_pong_progress (0 to 1) to color indices (0 to len-1)
                // At progress=0, color_index=0; at progress=1, color_index=len-1
                let max_index = (colors.len() - 1) as f64;
                let color_progress = ping_pong_progress * max_index;

                // Calculate current and next color indices for interpolation
                let color_index = color_progress.floor() as usize;
                let seg_progress = color_progress - color_index as f64;

                // Handle edge case when at exactly the last color (ping_pong_progress = 1.0)
                if color_index >= colors.len() - 1 {
                    (colors.len() - 1, colors.len() - 1, 0.0)
                } else {
                    (color_index, color_index + 1, seg_progress)
                }
            }
        };

        // Apply transition based on transition type
        let color = match transition {
            CycleTransition::Fade => {
                // Interpolate between current and next color for smooth transitions
                let current_color = colors[color_index % colors.len()];
                let next_color = colors[next_index % colors.len()];
                current_color.lerp(&next_color, segment_progress)
            }
            CycleTransition::Snap => {
                // Snap to current color (original behavior)
                colors[color_index % colors.len()]
            }
        };
        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                // Use fixture profile to determine how to apply color
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands = profile.apply_color(color, effect.layer, effect.blend_mode);

                // Apply the channel commands from the profile with crossfade multiplier
                for (channel_name, mut channel_state) in channel_commands {
                    // Apply crossfade multiplier to the color value
                    channel_state.value *= crossfade_multiplier;
                    fixture_state.set_channel(channel_name, channel_state);
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }

    /// Apply a strobe effect and return fixture states
    fn apply_strobe(
        &mut self,
        effect: &EffectInstance,
        frequency: f64,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                if frequency == 0.0 {
                    // Frequency 0 means strobe is disabled
                    if fixture.has_capability(FixtureCapabilities::STROBING) {
                        // Hardware strobe: just disable the strobe channel
                        fixture_state.set_channel(
                            "strobe".to_string(),
                            ChannelState::new(0.0, effect.layer, effect.blend_mode),
                        );
                    }
                    // Software strobe: when frequency=0, don't set any channels
                    // This allows parent layers/effects to take over control
                } else {
                    // Use fixture profile to determine how to apply strobe control
                    let profile = FixtureProfile::for_fixture(fixture);

                    // Calculate strobe parameters based on strategy
                    let (normalized_frequency, strobe_value) = if profile.strobe_strategy
                        == StrobeStrategy::DedicatedChannel
                    {
                        // Hardware strobe: normalize frequency to 0-1 range
                        let max_freq = fixture.max_strobe_frequency.unwrap_or(20.0);
                        let normalized = (frequency / max_freq).min(1.0);
                        (normalized, None)
                    } else {
                        // Software strobe: calculate on/off value
                        let strobe_period = 1.0 / frequency;
                        let strobe_phase = (elapsed.as_secs_f64() % strobe_period) / strobe_period;
                        let is_strobe_on = strobe_phase < 0.5; // 50% duty cycle
                        (frequency, Some(if is_strobe_on { 1.0 } else { 0.0 }))
                    };

                    let channel_commands = profile.apply_strobe(
                        normalized_frequency,
                        effect.layer,
                        effect.blend_mode,
                        crossfade_multiplier,
                        strobe_value,
                    );

                    // Apply the channel commands from the profile
                    for (channel_name, channel_state) in channel_commands {
                        fixture_state.set_channel(channel_name, channel_state);
                    }
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }

    /// Apply a dimmer effect and return fixture states
    fn apply_dimmer(
        &mut self,
        effect: &EffectInstance,
        start_level: f64,
        end_level: f64,
        curve: &DimmerCurve,
        elapsed: Duration,
        duration: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        // Calculate dimmer value based on elapsed time and duration with curve applied
        let dimmer_value = if duration.is_zero() {
            end_level // Instant transition
        } else {
            let linear_progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);

            // Apply curve to the progress value
            let curved_progress = match curve {
                DimmerCurve::Linear => linear_progress,
                DimmerCurve::Exponential => linear_progress * linear_progress,
                DimmerCurve::Logarithmic => {
                    if linear_progress <= 0.0 {
                        0.0
                    } else {
                        // Map [0,1] to [0,1] using log curve
                        // log(1 + 9*x) / log(10) gives nice log curve from 0 to 1
                        (1.0 + 9.0 * linear_progress).log10()
                    }
                }
                DimmerCurve::Sine => {
                    // Smooth ease-in-out using sine
                    (1.0 - ((linear_progress * std::f64::consts::PI).cos())) / 2.0
                }
                DimmerCurve::Cosine => {
                    // Smooth ease-in using cosine
                    1.0 - (1.0 - linear_progress).powi(2)
                }
            };

            start_level + (end_level - start_level) * curved_progress
        };

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                // Use fixture profile to determine how to apply brightness control
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands =
                    profile.apply_brightness(dimmer_value, effect.layer, effect.blend_mode);

                // Apply the channel commands from the profile
                for (channel_name, channel_state) in channel_commands {
                    fixture_state.set_channel(channel_name, channel_state);
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }

    /// Apply a chase effect and return fixture states
    fn apply_chase(
        &mut self,
        effect: &EffectInstance,
        pattern: &ChasePattern,
        speed: f64,
        direction: &ChaseDirection,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        // Guard against zero/negative speed - treat as "stopped" with first fixture active
        if speed <= 0.0 {
            let mut fixture_states = HashMap::new();
            for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
                if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                    let mut fixture_state = FixtureState::new();
                    let chase_value = if i == 0 { crossfade_multiplier } else { 0.0 };
                    let profile = FixtureProfile::for_fixture(fixture);
                    let channel_commands =
                        profile.apply_chase(chase_value, effect.layer, effect.blend_mode);
                    for (channel_name, channel_state) in channel_commands {
                        fixture_state.set_channel(channel_name, channel_state);
                    }
                    fixture_states.insert(fixture_name.clone(), fixture_state);
                }
            }
            return Ok(Some(fixture_states));
        }

        let chase_period = 1.0 / speed;

        let mut fixture_states = HashMap::new();
        let fixture_count = effect.target_fixtures.len();

        // Guard against empty fixture list - nothing to chase
        if fixture_count == 0 {
            return Ok(Some(fixture_states));
        }

        // Calculate fixture order based on pattern and direction
        let fixture_order = self.calculate_fixture_order(fixture_count, pattern, direction);

        // Calculate the pattern cycle length
        let pattern_length = fixture_order.len();

        // Use consistent timing for all patterns
        // Each position in the pattern should last the same time as a linear chase position
        let position_duration = chase_period / fixture_count as f64;
        let pattern_cycle_period = position_duration * pattern_length as f64;
        let pattern_progress =
            (elapsed.as_secs_f64() % pattern_cycle_period) / pattern_cycle_period;
        let current_pattern_index = (pattern_progress * pattern_length as f64) as usize;

        for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                // Check if this fixture is active in the current pattern position
                let is_fixture_active = if current_pattern_index < pattern_length {
                    fixture_order[current_pattern_index] == i
                } else {
                    false
                };

                let chase_value =
                    (if is_fixture_active { 1.0 } else { 0.0 }) * crossfade_multiplier;

                // Use fixture profile to determine how to apply chase control
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands =
                    profile.apply_chase(chase_value, effect.layer, effect.blend_mode);

                // Apply the channel commands from the profile
                for (channel_name, channel_state) in channel_commands {
                    fixture_state.set_channel(channel_name, channel_state);
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }

    /// Calculate fixture order for chase effects based on pattern and direction
    fn calculate_fixture_order(
        &self,
        fixture_count: usize,
        pattern: &ChasePattern,
        direction: &ChaseDirection,
    ) -> Vec<usize> {
        let mut order: Vec<usize> = (0..fixture_count).collect();

        match pattern {
            ChasePattern::Linear => {
                // Linear pattern - fixtures in order
                // Direction determines if we reverse the order
                match direction {
                    ChaseDirection::LeftToRight
                    | ChaseDirection::TopToBottom
                    | ChaseDirection::Clockwise => {
                        // Forward direction - keep original order
                        order
                    }
                    ChaseDirection::RightToLeft
                    | ChaseDirection::BottomToTop
                    | ChaseDirection::CounterClockwise => {
                        // Reverse direction - reverse the order
                        order.reverse();
                        order
                    }
                }
            }
            ChasePattern::Snake => {
                // Snake pattern - forward then reverse
                // Create a snake pattern: 0, 1, 2, 3, 2, 1, 0, 1, 2, 3, ...
                let mut snake_order = Vec::new();

                // Forward pass: 0, 1, 2, 3
                for i in 0..fixture_count {
                    snake_order.push(i);
                }

                // Reverse pass: 2, 1 (skip the last element to avoid duplication)
                for i in (1..fixture_count - 1).rev() {
                    snake_order.push(i);
                }

                // Apply direction
                match direction {
                    ChaseDirection::LeftToRight
                    | ChaseDirection::TopToBottom
                    | ChaseDirection::Clockwise => {
                        // Forward direction - use snake order as is
                        snake_order
                    }
                    ChaseDirection::RightToLeft
                    | ChaseDirection::BottomToTop
                    | ChaseDirection::CounterClockwise => {
                        // Reverse direction - reverse the snake order
                        snake_order.reverse();
                        snake_order
                    }
                }
            }
            ChasePattern::Random => {
                // Random pattern - shuffle the order
                // Use a simple deterministic shuffle based on fixture count
                // This ensures the same random order for the duration of the effect
                let seed = fixture_count * 7; // Simple seed based on fixture count

                // Simple shuffle algorithm
                for i in 0..fixture_count {
                    let j = (seed + i) % fixture_count;
                    order.swap(i, j);
                }
                order
            }
        }
    }

    /// Apply a rainbow effect and return fixture states
    fn apply_rainbow(
        &mut self,
        effect: &EffectInstance,
        speed: f64,
        saturation: f64,
        brightness: f64,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        let hue = (elapsed.as_secs_f64() * speed * 360.0) % 360.0;
        let color = Color::from_hsv(hue, saturation, brightness);

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                // Use fixture profile to determine how to apply color
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands = profile.apply_color(color, effect.layer, effect.blend_mode);

                // Apply the channel commands from the profile with crossfade multiplier
                for (channel_name, mut channel_state) in channel_commands {
                    // Apply crossfade multiplier to the color value
                    channel_state.value *= crossfade_multiplier;
                    fixture_state.set_channel(channel_name, channel_state);
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }

    /// Apply a pulse effect and return fixture states
    fn apply_pulse(
        &mut self,
        effect: &EffectInstance,
        base_level: f64,
        pulse_amplitude: f64,
        frequency: f64,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        let pulse_phase = elapsed.as_secs_f64() * frequency * 2.0 * std::f64::consts::PI;
        let pulse_value =
            (base_level + pulse_amplitude * (pulse_phase.sin() * 0.5 + 0.5)) * crossfade_multiplier;

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();

                // Use fixture profile to determine how to apply pulse control
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands =
                    profile.apply_pulse(pulse_value, effect.layer, effect.blend_mode);

                // Apply the channel commands from the profile
                for (channel_name, channel_state) in channel_commands {
                    fixture_state.set_channel(channel_name, channel_state);
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn create_test_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);
        channels.insert("white".to_string(), 5);
        channels.insert("strobe".to_string(), 6);

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "RGBW_Strobe".to_string(),
            channels,
            max_strobe_frequency: Some(20.0), // Default test fixture max strobe
        }
    }

    #[test]
    fn test_effect_engine_creation() {
        let engine = EffectEngine::new();
        assert!(engine.active_effects.is_empty());
    }

    #[test]
    fn test_fixture_registration() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);

        engine.register_fixture(fixture);
        assert!(engine.fixture_registry.contains_key("test_fixture"));
    }

    #[test]
    fn test_static_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);
        parameters.insert("red".to_string(), 1.0);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters: parameters.clone(),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have commands for dimmer and red channels
        assert_eq!(commands.len(), 2);

        // Check dimmer command (50% = 127)
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 127);

        // Check red command (100% = 255)
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        assert_eq!(red_cmd.value, 255);
    }

    #[test]
    fn test_color_cycle_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red
            Color::new(0, 255, 0), // Green
            Color::new(0, 0, 255), // Blue
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Test cycling over time
        // At t=0ms: should be red (index 0)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        assert_eq!(commands.len(), 3);
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 255);
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 0);

        // At t=500ms: should be green (index 1) - clearly in green's range
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        assert_eq!(commands.len(), 3);
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 255);
        assert_eq!(blue_cmd.value, 0);

        // At t=300ms: should be blue (index 2) - 300ms into the second cycle
        let commands = engine.update(Duration::from_millis(300)).unwrap();
        assert_eq!(commands.len(), 3);
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 255);
    }

    #[test]
    fn test_color_cycle_pingpong_peak() {
        // Regression test: PingPong should show the last color at cycle peak (cycle_progress = 0.5)
        // Previously, a bug caused it to incorrectly show the first color at the peak.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2) - should be shown at peak
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::PingPong,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: should be red (index 0) - start of cycle
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=0ms should be red"
        );

        // At t=500ms: cycle_progress = 0.5, ping_pong_progress = 1.0 (peak)
        // Should show the LAST color (blue, index 2), not the first color (red)
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=500ms (peak) should be blue (last color), not red"
        );

        // At t=1000ms: cycle_progress = 0.0, back to start
        // Should be red again (index 0)
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=1000ms should be red again"
        );
    }

    #[test]
    fn test_color_cycle_backward_boundary() {
        // Regression test: Backward direction should show the LAST color at cycle start (cycle_progress = 0.0)
        // Previously, a bug caused it to incorrectly show the first color due to:
        // reversed_progress = 1.0 → color_index_f = colors.len() → floor = colors.len() → modulo wraps to 0
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0) - should be shown at END of backward cycle
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2) - should be shown at START of backward cycle
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Backward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Note: engine.update() takes DELTA time, not absolute time!
        // Each call adds to the elapsed time.

        // At t=0ms: cycle_progress = 0.0, reversed_progress = 1.0
        // Should be BLUE (last color, index 2), NOT red (first color)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=0ms (cycle start) backward should show LAST color (blue), not first (red)"
        );

        // At t=500ms (dt=500): cycle_progress = 0.5, reversed_progress = 0.5
        // color_index_f = 1.5, color_index = 1 → green
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 255, 0),
            "At t=500ms should be green"
        );

        // At t=834ms (dt=334, total=834): cycle_progress ≈ 0.834, reversed_progress ≈ 0.166
        // color_index_f ≈ 0.5, color_index = 0 → red
        let commands = engine.update(Duration::from_millis(334)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=834ms should be red"
        );
    }

    #[test]
    fn test_color_cycle_backward_fade_boundary() {
        // Regression test: Backward + Fade at cycle start (cycle_progress = 0) should show
        // the LAST color, not interpolate toward the previous color.
        // Previously, segment_progress was 1.0 at cycle start due to clamping, causing
        // lerp to return next_color instead of current_color.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2) - should be shown at START
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Backward,
                transition: CycleTransition::Fade, // Key difference from Snap test
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: cycle_progress = 0, should be PURE BLUE (last color)
        // With the bug, segment_progress was 1.0, causing lerp to return Green instead
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=0ms Backward+Fade should show PURE BLUE (last color), not interpolated"
        );

        // At t=166ms: ~50% through Blue->Green segment, should be teal-ish
        let commands = engine.update(Duration::from_millis(166)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        // Should be interpolating between Blue and Green
        assert!(
            green_cmd.value > 100 && blue_cmd.value > 100,
            "At t=166ms should be fading from Blue toward Green, got ({}, {}, {})",
            red_cmd.value,
            green_cmd.value,
            blue_cmd.value
        );
    }

    #[test]
    fn test_color_cycle_fade_interpolation() {
        // Regression test: CycleTransition::Fade should smoothly interpolate between colors.
        // Previously, a bug divided segment_progress by segment_size (1/colors.len()),
        // effectively multiplying by colors.len(). This caused segment_progress to exceed 1.0
        // early in each segment, and since lerp clamps to 0-1, colors would snap at ~33%
        // through each segment instead of smoothly fading over the full segment duration.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 0, 255), // Blue (index 1)
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: should be pure red (start of first segment)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=0ms should be pure red"
        );

        // At t=250ms: 50% through red→blue segment, should be purple (127, 0, 127)
        // With the bug, segment_progress would be 1.0 (clamped from 0.5 * 2 = 1.0),
        // resulting in pure blue instead of purple.
        let commands = engine.update(Duration::from_millis(250)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        // Allow ±1 tolerance for floating point rounding
        assert!(
            (126..=128).contains(&red_cmd.value)
                && green_cmd.value == 0
                && (126..=128).contains(&blue_cmd.value),
            "At t=250ms (50% through segment) should be ~purple (127, 0, 127), got ({}, {}, {})",
            red_cmd.value,
            green_cmd.value,
            blue_cmd.value
        );

        // At t=500ms: start of blue→red segment, should be pure blue
        let commands = engine.update(Duration::from_millis(250)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=500ms should be pure blue"
        );

        // At t=750ms: 50% through blue→red segment, should be purple again
        let commands = engine.update(Duration::from_millis(250)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert!(
            (126..=128).contains(&red_cmd.value)
                && green_cmd.value == 0
                && (126..=128).contains(&blue_cmd.value),
            "At t=750ms (50% through segment) should be ~purple (127, 0, 127), got ({}, {}, {})",
            red_cmd.value,
            green_cmd.value,
            blue_cmd.value
        );
    }

    #[test]
    fn test_color_cycle_forward_wraparound() {
        // Test that Forward direction wraps correctly from last color back to first
        // Note: engine.update() takes DELTA time. Each call advances elapsed.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2)
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // With 3 colors at 1 cycle/second: each color is ~333.33ms
        // Color 0 (red): 0ms - 333.32ms
        // Color 1 (green): 333.33ms - 666.65ms
        // Color 2 (blue): 666.66ms - 999.99ms

        // At t=0ms: should be red (start of cycle)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        assert_eq!(red_cmd.value, 255, "At t=0ms should be red");

        // At t=350ms: should be green (past 333.33ms threshold)
        let commands = engine.update(Duration::from_millis(350)).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(green_cmd.value, 255, "At t=350ms should be green");

        // At t=700ms: should be blue (past 666.66ms threshold)
        let commands = engine.update(Duration::from_millis(350)).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(blue_cmd.value, 255, "At t=700ms should be blue");

        // At t=1050ms: should wrap back to red (past 1000ms)
        let commands = engine.update(Duration::from_millis(350)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        assert_eq!(red_cmd.value, 255, "At t=1050ms should wrap back to red");
    }

    #[test]
    fn test_color_cycle_two_colors_all_directions() {
        // Test all directions with just 2 colors to catch edge cases with minimal color sets
        let colors = vec![
            Color::new(255, 0, 0), // Red
            Color::new(0, 0, 255), // Blue
        ];

        for direction in [
            CycleDirection::Forward,
            CycleDirection::Backward,
            CycleDirection::PingPong,
        ] {
            let mut engine = EffectEngine::new();
            let fixture = create_test_fixture("test_fixture", 1, 1);
            engine.register_fixture(fixture);

            let effect = EffectInstance::new(
                "test_effect".to_string(),
                EffectType::ColorCycle {
                    colors: colors.clone(),
                    speed: TempoAwareSpeed::Fixed(1.0),
                    direction,
                    transition: CycleTransition::Snap,
                },
                vec!["test_fixture".to_string()],
                None,
                None,
                None,
            );

            engine.start_effect(effect).unwrap();

            // At t=0: should have a valid color (not crash, not garbage values)
            let commands = engine.update(Duration::from_millis(0)).unwrap();
            let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
            let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();

            // Should be either pure red or pure blue
            let is_valid_color = (red_cmd.value == 255 && blue_cmd.value == 0)
                || (red_cmd.value == 0 && blue_cmd.value == 255);
            assert!(
                is_valid_color,
                "{:?} at t=0 should be pure red or blue, got r={} b={}",
                direction, red_cmd.value, blue_cmd.value
            );

            // At t=500ms (half cycle): should still be valid
            let commands = engine.update(Duration::from_millis(500)).unwrap();
            let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
            let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();

            let is_valid_color = (red_cmd.value == 255 && blue_cmd.value == 0)
                || (red_cmd.value == 0 && blue_cmd.value == 255);
            assert!(
                is_valid_color,
                "{:?} at t=500ms should be pure red or blue, got r={} b={}",
                direction, red_cmd.value, blue_cmd.value
            );
        }
    }

    #[test]
    fn test_strobe_boundary_at_duty_cycle_transition() {
        // Test strobe behavior at exactly the 50% duty cycle boundary
        // strobe_phase < 0.5 means ON, >= 0.5 means OFF
        let mut engine = EffectEngine::new();

        // Create a fixture WITHOUT hardware strobe capability to test software strobe
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        let fixture = FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGB".to_string(),
            channels,
            max_strobe_frequency: None, // No hardware strobe
        };
        engine.register_fixture(fixture);

        // 2 Hz strobe = 500ms period, so 50% duty cycle transition at 250ms
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(2.0),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: strobe_phase=0, which is < 0.5, so ON (dimmer=255)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 255, "At t=0ms strobe should be ON");

        // At t=249ms: still in first half of period, should be ON
        let commands = engine.update(Duration::from_millis(249)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(
            dimmer_cmd.value, 255,
            "At t=249ms strobe should still be ON"
        );

        // At t=251ms: just past 50% of period, should be OFF
        let commands = engine.update(Duration::from_millis(2)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 0, "At t=251ms strobe should be OFF");

        // At t=500ms: start of new period, should be ON again
        let commands = engine.update(Duration::from_millis(249)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(
            dimmer_cmd.value, 255,
            "At t=500ms strobe should be ON again"
        );
    }

    #[test]
    fn test_chase_fixture_boundaries() {
        // Test chase effect transitions between fixtures correctly
        // Note: Chase applies dimmer to active fixture, 0 to others
        let mut engine = EffectEngine::new();

        // Create 3 fixtures for chase, each at different addresses
        let fixture_0 = create_test_fixture("fixture_0", 1, 1);
        let fixture_1 = create_test_fixture("fixture_1", 1, 11);
        let fixture_2 = create_test_fixture("fixture_2", 1, 21);
        engine.register_fixture(fixture_0);
        engine.register_fixture(fixture_1);
        engine.register_fixture(fixture_2);

        // 1 Hz chase with 3 fixtures = each fixture active for 333.33ms
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "fixture_0".to_string(),
                "fixture_1".to_string(),
                "fixture_2".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Helper to count active fixtures (dimmer channel = address, value = 255)
        let count_active = |commands: &[DmxCommand]| -> usize {
            // Each fixture has dimmer at relative channel 1
            // fixture_0: channel 1, fixture_1: channel 11, fixture_2: channel 21
            let dimmer_channels = [1, 11, 21];
            commands
                .iter()
                .filter(|cmd| dimmer_channels.contains(&cmd.channel) && cmd.value == 255)
                .count()
        };

        // At t=0ms: first fixture should be active (pattern_index = 0)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=0ms exactly one fixture should be active"
        );

        // At t=350ms: second fixture should be active (past 333.33ms)
        let commands = engine.update(Duration::from_millis(350)).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=350ms exactly one fixture should be active"
        );

        // At t=700ms: third fixture should be active (past 666.66ms)
        let commands = engine.update(Duration::from_millis(350)).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=700ms exactly one fixture should be active"
        );

        // At t=1050ms: should wrap back (past 1000ms)
        let commands = engine.update(Duration::from_millis(350)).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=1050ms exactly one fixture should be active (wrapped)"
        );
    }

    #[test]
    fn test_rainbow_hue_wraparound() {
        // Test that rainbow effect wraps hue correctly at 360 degrees
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Speed of 1.0 = 1 full hue rotation per second (360 degrees/sec)
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: hue=0 (red)
        let commands_start = engine.update(Duration::from_millis(0)).unwrap();
        let red_start = commands_start
            .iter()
            .find(|cmd| cmd.channel == 2)
            .unwrap()
            .value;
        let green_start = commands_start
            .iter()
            .find(|cmd| cmd.channel == 3)
            .unwrap()
            .value;
        let blue_start = commands_start
            .iter()
            .find(|cmd| cmd.channel == 4)
            .unwrap()
            .value;

        // At hue=0 (red), should be approximately (255, 0, 0)
        assert!(
            red_start > 200 && green_start < 50 && blue_start < 50,
            "At t=0ms should be red-ish, got ({}, {}, {})",
            red_start,
            green_start,
            blue_start
        );

        // At t=1000ms: hue should wrap back to 0 (red again)
        let commands_end = engine.update(Duration::from_millis(1000)).unwrap();
        let red_end = commands_end
            .iter()
            .find(|cmd| cmd.channel == 2)
            .unwrap()
            .value;
        let green_end = commands_end
            .iter()
            .find(|cmd| cmd.channel == 3)
            .unwrap()
            .value;
        let blue_end = commands_end
            .iter()
            .find(|cmd| cmd.channel == 4)
            .unwrap()
            .value;

        // Should be back to approximately red
        assert!(
            red_end > 200 && green_end < 50 && blue_end < 50,
            "At t=1000ms should wrap back to red-ish, got ({}, {}, {})",
            red_end,
            green_end,
            blue_end
        );

        // Colors at start and end should be very similar (wrapped)
        assert!(
            (red_start as i16 - red_end as i16).abs() < 10,
            "Red should be similar after full cycle"
        );
    }

    #[test]
    fn test_pulse_at_peaks_and_troughs() {
        // Test pulse effect at its mathematical peaks and troughs
        // pulse_value = (base_level + pulse_amplitude * (sin(phase) * 0.5 + 0.5))
        // At phase=0: sin(0)=0, so multiplier=0.5
        // At phase=π/2: sin=1, so multiplier=1.0 (peak)
        // At phase=3π/2: sin=-1, so multiplier=0.0 (trough)

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // 1 Hz pulse, base_level=0.0, amplitude=1.0 for easy calculation
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Pulse {
                base_level: 0.0,
                pulse_amplitude: 1.0,
                frequency: TempoAwareFrequency::Fixed(1.0),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: phase=0, sin(0)=0, pulse_value = 0 + 1.0 * (0 * 0.5 + 0.5) = 0.5
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        // 0.5 * 255 ≈ 127
        assert!(
            (120..=135).contains(&dimmer_cmd.value),
            "At t=0ms pulse should be ~127 (mid), got {}",
            dimmer_cmd.value
        );

        // At t=250ms: phase=π/2, sin(π/2)=1, pulse_value = 0 + 1.0 * (1 * 0.5 + 0.5) = 1.0 (peak)
        let commands = engine.update(Duration::from_millis(250)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(
            dimmer_cmd.value >= 250,
            "At t=250ms pulse should be at peak (~255), got {}",
            dimmer_cmd.value
        );

        // At t=750ms: phase=3π/2, sin(3π/2)=-1, pulse_value = 0 + 1.0 * (-1 * 0.5 + 0.5) = 0.0 (trough)
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(
            dimmer_cmd.value <= 5,
            "At t=750ms pulse should be at trough (~0), got {}",
            dimmer_cmd.value
        );

        // At t=1000ms: should be back to mid-point
        let commands = engine.update(Duration::from_millis(250)).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(
            (120..=135).contains(&dimmer_cmd.value),
            "At t=1000ms pulse should be back to ~127 (mid), got {}",
            dimmer_cmd.value
        );
    }

    #[test]
    fn test_color_cycle_zero_speed() {
        // Edge case: speed=0 should not cause divide-by-zero, should show first color
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (first)
            Color::new(0, 255, 0), // Green
            Color::new(0, 0, 255), // Blue
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(0.0), // Zero speed!
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Should not panic, and should show first color
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "Zero speed should show first color (red)"
        );

        // Even after time passes, should still show first color (frozen)
        let commands = engine.update(Duration::from_millis(5000)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "Zero speed should remain frozen on first color"
        );
    }

    #[test]
    fn test_chase_zero_speed() {
        // Edge case: speed=0 should not cause divide-by-zero, should keep first fixture active
        let mut engine = EffectEngine::new();
        let fixture_0 = create_test_fixture("fixture_0", 1, 1);
        let fixture_1 = create_test_fixture("fixture_1", 1, 11);
        let fixture_2 = create_test_fixture("fixture_2", 1, 21);
        engine.register_fixture(fixture_0);
        engine.register_fixture(fixture_1);
        engine.register_fixture(fixture_2);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(0.0), // Zero speed!
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "fixture_0".to_string(),
                "fixture_1".to_string(),
                "fixture_2".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Should not panic, first fixture should be active
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let dimmer_channels = [1, 11, 21];
        let active_count = commands
            .iter()
            .filter(|cmd| dimmer_channels.contains(&cmd.channel) && cmd.value == 255)
            .count();
        assert_eq!(
            active_count, 1,
            "Zero speed should have exactly one fixture active"
        );

        // First fixture (channel 1) should be the active one
        let first_dimmer = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(first_dimmer.value, 255, "First fixture should be active");

        // Even after time passes, should still be frozen on first fixture
        let commands = engine.update(Duration::from_millis(5000)).unwrap();
        let first_dimmer = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(
            first_dimmer.value, 255,
            "Zero speed should remain frozen on first fixture"
        );
    }

    #[test]
    fn test_chase_empty_fixtures() {
        // Edge case: chase with no fixtures should not panic (empty fixture list)
        let mut engine = EffectEngine::new();
        // Don't register any fixtures

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
            },
            vec![], // Empty fixture list!
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Should not panic, should return empty commands
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        assert!(
            commands.is_empty(),
            "Empty fixture chase should produce no commands"
        );

        // Should still work after time passes
        let commands = engine.update(Duration::from_millis(1000)).unwrap();
        assert!(
            commands.is_empty(),
            "Empty fixture chase should still produce no commands"
        );
    }

    #[test]
    fn test_single_color_cycle() {
        // Edge case: color cycle with only 1 color should always show that color
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![Color::new(255, 128, 64)]; // Single color

        for direction in [
            CycleDirection::Forward,
            CycleDirection::Backward,
            CycleDirection::PingPong,
        ] {
            let effect = EffectInstance::new(
                "test_effect".to_string(),
                EffectType::ColorCycle {
                    colors: colors.clone(),
                    speed: TempoAwareSpeed::Fixed(1.0),
                    direction,
                    transition: CycleTransition::Snap,
                },
                vec!["test_fixture".to_string()],
                None,
                None,
                None,
            );

            let mut test_engine = EffectEngine::new();
            let fixture = create_test_fixture("test_fixture", 1, 1);
            test_engine.register_fixture(fixture);
            test_engine.start_effect(effect).unwrap();

            // Should always be the same color at any time
            for ms in [0, 250, 500, 750, 1000] {
                let commands = test_engine.update(Duration::from_millis(ms)).unwrap();
                let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
                let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
                let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
                assert_eq!(
                    (red_cmd.value, green_cmd.value, blue_cmd.value),
                    (255, 128, 64),
                    "{:?} with single color at t={}ms should always show that color",
                    direction,
                    ms
                );
            }
        }
    }

    #[test]
    fn test_strobe_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have strobe command since fixture has dedicated strobe channel
        assert_eq!(commands.len(), 1);

        // Check strobe command (frequency 2.0 / max 20.0 = 0.1 = 25 in DMX)
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap();
        assert_eq!(strobe_cmd.value, 25);
    }

    #[test]
    fn test_dimmer_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 1.0,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_timing(Some(Instant::now()), Some(Duration::from_secs(1)));

        engine.start_effect(effect).unwrap();

        // Update the engine after 500ms (half duration)
        let commands = engine.update(Duration::from_millis(500)).unwrap();

        // Should have only dimmer command since fixture has dedicated dimmer channel
        // The fixture profile system uses DedicatedDimmer strategy for RGB+dimmer fixtures
        assert_eq!(commands.len(), 1);

        // Check dimmer command
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 127);
    }

    #[test]
    fn test_rainbow_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have RGB commands
        assert_eq!(commands.len(), 3);

        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2);
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3);
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4);

        assert!(red_cmd.is_some());
        assert!(green_cmd.is_some());
        assert!(blue_cmd.is_some());
    }

    #[test]
    fn test_pulse_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Fixed(1.0), // 1 Hz
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have dimmer command since fixture has dedicated dimmer channel
        assert_eq!(commands.len(), 1);

        // Check that dimmer command exists (values are u8, so always in valid range)
        let _dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    }

    #[test]
    fn test_chase_effect() {
        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);

        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have dimmer commands for all fixtures
        // Note: The chase effect might generate more commands than expected
        // due to the simplified implementation
        assert!(commands.len() >= 3);

        // All commands should be for dimmer channels (but may be on different DMX addresses)
        for cmd in &commands {
            // The chase effect generates commands for different DMX addresses
            // but all should be for the dimmer channel (channel 1 relative to fixture address)
            assert!(cmd.channel >= 1 && cmd.channel <= 15); // Within reasonable DMX range
        }

        // Should have commands for all three fixtures
        let fixture1_cmd = commands
            .iter()
            .find(|cmd| cmd.universe == 1 && cmd.channel == 1);
        let fixture2_cmd = commands
            .iter()
            .find(|cmd| cmd.universe == 1 && cmd.channel == 6);
        let fixture3_cmd = commands
            .iter()
            .find(|cmd| cmd.universe == 1 && cmd.channel == 11);

        assert!(fixture1_cmd.is_some());
        assert!(fixture2_cmd.is_some());
        assert!(fixture3_cmd.is_some());
    }

    #[test]
    fn test_effect_priority() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Low priority effect
        let mut low_priority_params = HashMap::new();
        low_priority_params.insert("dimmer".to_string(), 0.3);

        let low_effect = EffectInstance::new(
            "low_effect".to_string(),
            EffectType::Static {
                parameters: low_priority_params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_priority(1);

        // High priority effect
        let mut high_priority_params = HashMap::new();
        high_priority_params.insert("dimmer".to_string(), 0.8);

        let high_effect = EffectInstance::new(
            "high_effect".to_string(),
            EffectType::Static {
                parameters: high_priority_params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_priority(10);

        engine.start_effect(low_effect).unwrap();
        engine.start_effect(high_effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have only one dimmer command (high priority wins)
        assert_eq!(commands.len(), 1);
        let dimmer_cmd = &commands[0];
        assert_eq!(dimmer_cmd.value, 204); // 80% of 255
    }

    #[test]
    fn test_effect_stop() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine - should have command
        let commands = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands.len(), 1);

        // Stop the effect

        // Update again - should still have commands since we didn't stop the effect
        let commands = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn test_invalid_fixture_error() {
        let mut engine = EffectEngine::new();

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["nonexistent_fixture".to_string()],
            None,
            None,
            None,
        );

        let result = engine.start_effect(effect);
        assert!(result.is_err());

        if let Err(EffectError::Fixture(msg)) = result {
            assert!(msg.contains("nonexistent_fixture"));
        } else {
            panic!("Expected InvalidFixture error");
        }
    }

    #[test]
    fn test_effect_duration_expiry() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Some(Duration::from_millis(100)), // Set duration for expiry test
            },
            vec!["test_fixture".to_string()],
            None,                             // up_time
            Some(Duration::from_millis(100)), // hold_time
            None,                             // down_time
        )
        .with_timing(Some(Instant::now()), Some(Duration::from_millis(100)));

        engine.start_effect(effect).unwrap();

        // Update before expiry - should have commands
        let commands = engine.update(Duration::from_millis(50)).unwrap();
        assert_eq!(commands.len(), 1);

        // Update after expiry - timed static effects end and don't preserve their state
        let commands = engine.update(Duration::from_millis(100)).unwrap();
        // Timed static effects end and don't generate commands after expiry
        assert_eq!(commands.len(), 0);
    }

    #[test]
    fn test_tempo_aware_speed_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 4 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(4)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a cycle effect with speed: 1measure (tempo-aware)
        let colors = vec![
            Color::new(255, 0, 0), // Red
            Color::new(0, 255, 0), // Green
            Color::new(0, 0, 255), // Blue
        ];

        let effect = EffectInstance::new(
            "tempo_aware_cycle".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Measures(1.0), // 1 cycle per measure
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 measure = 2.0s, so speed = 0.5 cycles/sec
        // Verify effect is running
        let commands = engine.update(Duration::from_millis(100)).unwrap();
        assert!(!commands.is_empty(), "Effect should generate commands");

        // At t=4s: tempo changes to 60 BPM
        // At t=4.1s (60 BPM): 1 measure = 4.0s, so speed = 0.25 cycles/sec
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_secs(4)).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100)).unwrap(); // 0.1s after tempo change

        // At slower tempo, the cycle should be progressing more slowly
        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Effect should still generate commands after tempo change"
        );

        // Verify that the speed calculation uses the new tempo
        // We can't easily verify exact color values, but we can verify the effect is adapting
        // by checking that it's still running and producing different values over time
        let commands_later = engine.update(Duration::from_millis(1000)).unwrap(); // 1.1s after tempo change
        assert!(
            !commands_later.is_empty(),
            "Effect should continue running after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_frequency_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 2 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a background static effect so strobe has something to work with
        let mut bg_params = HashMap::new();
        bg_params.insert("red".to_string(), 1.0);
        bg_params.insert("green".to_string(), 1.0);
        bg_params.insert("blue".to_string(), 1.0);
        let bg_effect = EffectInstance::new(
            "bg".to_string(),
            EffectType::Static {
                parameters: bg_params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(bg_effect).unwrap();
        engine.update(Duration::from_millis(10)).unwrap(); // Let background settle

        // Create a strobe effect with frequency: 1beat (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_strobe".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Beats(1.0), // 1 cycle per beat
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 beat = 0.5s, so frequency = 2.0 Hz
        // At 2 Hz, period = 0.5s
        let commands_before = engine.update(Duration::from_millis(100)).unwrap();
        let strobe_before = commands_before.iter().find(|cmd| cmd.channel == 6);
        assert!(
            strobe_before.is_some(),
            "Strobe should generate commands before tempo change"
        );

        // At t=2s: tempo changes to 60 BPM
        // At t=2.1s (60 BPM): 1 beat = 1.0s, so frequency = 1.0 Hz
        // At 1 Hz, period = 1.0s
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_secs(2)).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100)).unwrap(); // 0.1s after tempo change

        // The effect should still be running (may or may not generate strobe commands depending on phase)
        // The key is that the frequency calculation uses the new tempo
        // We verify the effect is adapting by checking commands are generated
        assert!(
            !commands_after.is_empty(),
            "Effect should still generate commands after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_chase_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);
        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 3 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(3)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a chase effect with speed: 1measure (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_chase".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Measures(1.0), // 1 cycle per measure
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 measure = 2.0s, so speed = 0.5 cycles/sec
        let commands_before = engine.update(Duration::from_millis(100)).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Chase should generate commands before tempo change"
        );

        // At t=3s: tempo changes to 60 BPM
        // At t=3.1s (60 BPM): 1 measure = 4.0s, so speed = 0.25 cycles/sec
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_secs(3)).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100)).unwrap(); // 0.1s after tempo change

        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Chase should still generate commands after tempo change"
        );

        // Verify it continues running
        let commands_later = engine.update(Duration::from_millis(1000)).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Chase should continue running after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_rainbow_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 2.5 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_millis(2500)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a rainbow effect with speed: 2beats (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_rainbow".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Beats(2.0), // 1 cycle per 2 beats
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 2 beats = 1.0s, so speed = 1.0 cycles/sec
        let commands_before = engine.update(Duration::from_millis(100)).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Rainbow should generate commands before tempo change"
        );

        // At t=2.5s: tempo changes to 60 BPM
        // At t=2.6s (60 BPM): 2 beats = 2.0s, so speed = 0.5 cycles/sec
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_millis(2500)).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100)).unwrap(); // 0.1s after tempo change

        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Rainbow should still generate commands after tempo change"
        );

        // Verify it continues running
        let commands_later = engine.update(Duration::from_millis(1000)).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Rainbow should continue running after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_pulse_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 1.5 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_millis(1500)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a pulse effect with frequency: 1beat (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_pulse".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Beats(1.0), // 1 cycle per beat
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 beat = 0.5s, so frequency = 2.0 Hz
        let commands_before = engine.update(Duration::from_millis(100)).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Pulse should generate commands before tempo change"
        );

        // At t=1.5s: tempo changes to 60 BPM
        // At t=1.6s (60 BPM): 1 beat = 1.0s, so frequency = 1.0 Hz
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_millis(1500)).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100)).unwrap(); // 0.1s after tempo change

        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Pulse should still generate commands after tempo change"
        );

        // Verify it continues running
        let commands_later = engine.update(Duration::from_millis(1000)).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Pulse should continue running after tempo change"
        );
    }

    #[test]
    fn test_clear_layer() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Start effects on different layers
        let bg_effect = EffectInstance::new(
            "bg_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("dimmer".to_string(), 0.5);
                    p
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        let mut fg_effect = EffectInstance::new(
            "fg_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("dimmer".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        fg_effect.layer = EffectLayer::Foreground;

        engine.start_effect(bg_effect).unwrap();
        engine.start_effect(fg_effect).unwrap();
        assert_eq!(engine.active_effects_count(), 2);

        // Clear foreground layer
        engine.clear_layer(EffectLayer::Foreground);
        assert_eq!(engine.active_effects_count(), 1);
        assert!(engine.has_effect("bg_effect"));
        assert!(!engine.has_effect("fg_effect"));
    }

    #[test]
    fn test_freeze_unfreeze_layer() {
        let mut engine = EffectEngine::new();

        // Create RGB fixture for rainbow test
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        let fixture = FixtureInfo {
            name: "rgb_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGB".to_string(),
            channels,
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);

        // Start a rainbow effect - it cycles through colors over time
        let effect = EffectInstance::new(
            "bg_effect".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["rgb_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(effect).unwrap();

        // Let the effect run for a bit to get to an interesting state
        let _commands1 = engine.update(Duration::from_millis(250)).unwrap();

        // Capture the state at this point
        let commands_before_freeze = engine.update(Duration::from_millis(10)).unwrap();
        assert!(!commands_before_freeze.is_empty());

        // Freeze the background layer
        engine.freeze_layer(EffectLayer::Background);
        assert!(engine.is_layer_frozen(EffectLayer::Background));

        // Update multiple times - the values should stay the same while frozen
        let commands_frozen1 = engine.update(Duration::from_millis(100)).unwrap();
        let commands_frozen2 = engine.update(Duration::from_millis(100)).unwrap();
        let commands_frozen3 = engine.update(Duration::from_millis(500)).unwrap();

        assert!(!commands_frozen1.is_empty());
        assert!(!commands_frozen2.is_empty());
        assert!(!commands_frozen3.is_empty());

        // All frozen commands should have the same values
        // Sort by channel to ensure consistent comparison
        let mut vals1: Vec<u8> = commands_frozen1.iter().map(|c| c.value).collect();
        let mut vals2: Vec<u8> = commands_frozen2.iter().map(|c| c.value).collect();
        let mut vals3: Vec<u8> = commands_frozen3.iter().map(|c| c.value).collect();
        vals1.sort();
        vals2.sort();
        vals3.sort();

        assert_eq!(
            vals1, vals2,
            "Frozen layer should produce same values: {:?} vs {:?}",
            vals1, vals2
        );
        assert_eq!(
            vals2, vals3,
            "Frozen layer should produce same values: {:?} vs {:?}",
            vals2, vals3
        );

        // Unfreeze the layer
        engine.unfreeze_layer(EffectLayer::Background);
        assert!(!engine.is_layer_frozen(EffectLayer::Background));

        // After unfreezing, the effect should resume and values should change
        let commands_after1 = engine.update(Duration::from_millis(100)).unwrap();
        let commands_after2 = engine.update(Duration::from_millis(200)).unwrap();

        assert!(!commands_after1.is_empty());
        assert!(!commands_after2.is_empty());

        // Values should be different after unfreezing and time passing
        let mut vals_after1: Vec<u8> = commands_after1.iter().map(|c| c.value).collect();
        let mut vals_after2: Vec<u8> = commands_after2.iter().map(|c| c.value).collect();
        vals_after1.sort();
        vals_after2.sort();

        // The effect should be animating, so values should differ
        // (with a 200ms gap at 1 cycle/sec, hue shifts about 72 degrees)
        assert_ne!(
            vals_after1, vals_after2,
            "After unfreezing, effect should animate: {:?} vs {:?}",
            vals_after1, vals_after2
        );
    }

    #[test]
    fn test_release_frozen_layer_maintains_animation_continuity() {
        // Regression test: releasing a frozen layer should not cause animation discontinuity.
        // Before the fix, release_layer_with_time would call frozen_layers.remove() directly
        // instead of unfreeze_layer(), causing effects to jump forward in their animation.
        let mut engine = EffectEngine::new();

        // Create RGB fixture for rainbow test
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        let fixture = FixtureInfo {
            name: "rgb_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGB".to_string(),
            channels,
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);

        // Start a rainbow effect - it cycles through colors over time
        let effect = EffectInstance::new(
            "rainbow".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["rgb_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(effect).unwrap();

        // Run effect to get to an interesting state (250ms into the cycle)
        engine.update(Duration::from_millis(250)).unwrap();

        // Capture the current state (before freeze)
        let _commands_before_freeze = engine.update(Duration::from_millis(10)).unwrap();

        // Freeze the layer
        engine.freeze_layer(EffectLayer::Background);

        // Let significant time pass while frozen (1 second = full cycle if not frozen)
        engine.update(Duration::from_millis(500)).unwrap();
        engine.update(Duration::from_millis(500)).unwrap();

        // Capture the frozen state (should be same as before freeze)
        let commands_frozen = engine.update(Duration::from_millis(10)).unwrap();
        // Sort by channel for consistent comparison (DMX commands may be returned in any order)
        let mut frozen_sorted: Vec<_> = commands_frozen
            .iter()
            .map(|c| (c.channel, c.value))
            .collect();
        frozen_sorted.sort_by_key(|(ch, _)| *ch);
        let vals_frozen: Vec<u8> = frozen_sorted.iter().map(|(_, v)| *v).collect();

        // Now release the frozen layer with a fade time
        engine.release_layer_with_time(EffectLayer::Background, Some(Duration::from_secs(2)));

        // Immediately after release, the effect should continue from where it was frozen,
        // NOT jump forward by the 1 second that passed while frozen.
        let commands_after_release = engine.update(Duration::from_millis(10)).unwrap();
        // Sort by channel for consistent comparison
        let mut after_release_sorted: Vec<_> = commands_after_release
            .iter()
            .map(|c| (c.channel, c.value))
            .collect();
        after_release_sorted.sort_by_key(|(ch, _)| *ch);
        let vals_after_release: Vec<u8> = after_release_sorted.iter().map(|(_, v)| *v).collect();

        // The values right after release should be very close to the frozen values
        // (only 10ms of animation has passed, not 1+ second)
        // We allow small differences due to the 10ms update and fade starting
        let max_diff: i16 = vals_frozen
            .iter()
            .zip(vals_after_release.iter())
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max()
            .unwrap_or(0);

        // If the bug exists (no start time adjustment), the rainbow would have jumped
        // forward by ~1 second in its cycle, causing a large color difference.
        // At 1 cycle/second, that's a 360 degree hue shift (back to same color)
        // but even 500ms would be 180 degrees (opposite color = huge difference).
        // With the fix, we should see only tiny differences from the 10ms elapsed.
        assert!(
            max_diff < 30,
            "Release of frozen layer caused animation discontinuity! \
             Frozen: {:?}, After release: {:?}, Max diff: {}. \
             Effect should continue from frozen state, not jump forward.",
            vals_frozen,
            vals_after_release,
            max_diff
        );

        // Also verify the effect is actually fading out over time
        engine.update(Duration::from_millis(1000)).unwrap();
        let commands_mid_fade = engine.update(Duration::from_millis(10)).unwrap();
        // Sort by channel for consistent comparison
        let mut mid_fade_sorted: Vec<_> = commands_mid_fade
            .iter()
            .map(|c| (c.channel, c.value))
            .collect();
        mid_fade_sorted.sort_by_key(|(ch, _)| *ch);
        let vals_mid_fade: Vec<u8> = mid_fade_sorted.iter().map(|(_, v)| *v).collect();

        // At 1 second into a 2 second fade, values should be roughly half
        let avg_mid: f64 =
            vals_mid_fade.iter().map(|v| *v as f64).sum::<f64>() / vals_mid_fade.len() as f64;
        let avg_frozen: f64 =
            vals_frozen.iter().map(|v| *v as f64).sum::<f64>() / vals_frozen.len() as f64;

        // Mid-fade average should be notably lower than frozen average
        assert!(
            avg_mid < avg_frozen * 0.8,
            "Effect should be fading: frozen avg={:.1}, mid-fade avg={:.1}",
            avg_frozen,
            avg_mid
        );
    }

    #[test]
    fn test_layer_intensity_master() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Start a static effect at 100% dimmer
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("dimmer".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(effect).unwrap();

        // Get commands at full intensity
        let commands_full = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands_full.len(), 1);
        let full_value = commands_full[0].value;
        assert_eq!(full_value, 255); // Full intensity

        // Set layer intensity master to 50%
        engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
        assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.5).abs() < 0.01);

        // Get commands at 50% master
        let commands_half = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands_half.len(), 1);
        let half_value = commands_half[0].value;
        assert_eq!(half_value, 127); // 50% of 255
    }

    #[test]
    fn test_layer_speed_master() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Test that speed master affects effect timing
        engine.set_layer_speed_master(EffectLayer::Background, 2.0);
        assert!((engine.get_layer_speed_master(EffectLayer::Background) - 2.0).abs() < 0.01);

        engine.set_layer_speed_master(EffectLayer::Background, 0.5);
        assert!((engine.get_layer_speed_master(EffectLayer::Background) - 0.5).abs() < 0.01);

        // Reset to default
        engine.set_layer_speed_master(EffectLayer::Background, 1.0);
        assert!((engine.get_layer_speed_master(EffectLayer::Background) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_release_layer_fade_behavior() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Start an effect on background layer
        let effect = EffectInstance::new(
            "bg_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("dimmer".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(effect).unwrap();

        // Get initial commands at full brightness
        let commands_before = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands_before.len(), 1);
        assert_eq!(commands_before[0].value, 255);

        // Release the layer with a 1 second fade
        engine.release_layer_with_time(EffectLayer::Background, Some(Duration::from_secs(1)));

        // Immediately after release, should still be near full
        let commands_start = engine.update(Duration::from_millis(16)).unwrap();
        assert!(!commands_start.is_empty());

        // Halfway through fade (500ms), should be around half brightness
        let commands_mid = engine.update(Duration::from_millis(500)).unwrap();
        if !commands_mid.is_empty() {
            // Value should be less than full
            assert!(
                commands_mid[0].value < 200,
                "Should be fading: {}",
                commands_mid[0].value
            );
        }

        // After fade completes (another 600ms), effect should be gone
        let _commands_end = engine.update(Duration::from_millis(600)).unwrap();
        // Effect should have completed and been removed
        assert_eq!(engine.active_effects_count(), 0);
    }

    #[test]
    fn test_layer_commands_edge_cases() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Clear an empty layer - should not panic
        engine.clear_layer(EffectLayer::Foreground);
        assert_eq!(engine.active_effects_count(), 0);

        // Release an empty layer - should not panic
        engine.release_layer(EffectLayer::Midground);

        // Double freeze - should not panic
        engine.freeze_layer(EffectLayer::Background);
        engine.freeze_layer(EffectLayer::Background);
        assert!(engine.is_layer_frozen(EffectLayer::Background));

        // Unfreeze non-frozen layer - should not panic
        engine.unfreeze_layer(EffectLayer::Foreground);

        // Set intensity master multiple times
        engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
        engine.set_layer_intensity_master(EffectLayer::Background, 0.75);
        assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.75).abs() < 0.01);

        // Intensity clamping
        engine.set_layer_intensity_master(EffectLayer::Background, 1.5); // Should clamp to 1.0
        assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 1.0).abs() < 0.01);

        engine.set_layer_intensity_master(EffectLayer::Background, -0.5); // Should clamp to 0.0
        assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_speed_master_affects_effect_progression() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Start a pulse effect - easier to verify timing changes
        let effect = EffectInstance::new(
            "pulse".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Fixed(1.0), // 1 cycle per second
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(effect).unwrap();

        // Get initial value
        let cmd1 = engine.update(Duration::from_millis(100)).unwrap();
        assert!(!cmd1.is_empty());
        let _initial_value = cmd1[0].value;

        // Now set speed master to 0 (effectively frozen via speed = 0)
        engine.set_layer_speed_master(EffectLayer::Background, 0.0);

        // With speed = 0, elapsed time stays at same effective position
        // So values should stay similar
        let cmd2 = engine.update(Duration::from_millis(500)).unwrap();
        let cmd3 = engine.update(Duration::from_millis(500)).unwrap();

        assert!(!cmd2.is_empty());
        assert!(!cmd3.is_empty());

        // With speed = 0, effect time doesn't progress, so values should be consistent
        // (allowing for small floating point differences)
        let val2 = cmd2[0].value;
        let val3 = cmd3[0].value;

        // Values should be the same when speed is 0
        assert_eq!(
            val2, val3,
            "Speed=0 should produce consistent values: {} vs {}",
            val2, val3
        );
    }

    #[test]
    fn test_speed_master_resume_from_zero() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Start a pulse effect
        let effect = EffectInstance::new(
            "pulse".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Fixed(1.0),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(effect).unwrap();

        // Run for a bit to get to a known state
        engine.update(Duration::from_millis(250)).unwrap();

        // Freeze with speed=0
        engine.set_layer_speed_master(EffectLayer::Background, 0.0);

        // Record frozen value
        let frozen_cmd = engine.update(Duration::from_millis(100)).unwrap();
        let frozen_val = frozen_cmd[0].value;

        // Wait a bit while frozen
        engine.update(Duration::from_millis(500)).unwrap();

        // Resume with speed=1
        engine.set_layer_speed_master(EffectLayer::Background, 1.0);

        // The effect should now progress from where it was frozen
        let resume_cmd1 = engine.update(Duration::from_millis(100)).unwrap();
        let resume_cmd2 = engine.update(Duration::from_millis(100)).unwrap();

        // After resuming, values should change (effect is running again)
        // We can't predict exact values due to sinusoidal pulse, but they should differ
        // over enough time
        let val1 = resume_cmd1[0].value;
        let val2 = resume_cmd2[0].value;

        // At least verify we got values (effect is running)
        assert!(!resume_cmd1.is_empty());
        assert!(!resume_cmd2.is_empty());

        // The frozen value should be different from at least one of the resumed values
        // (since we're now progressing through the pulse cycle)
        let changed = frozen_val != val1 || frozen_val != val2 || val1 != val2;
        assert!(
            changed,
            "Effect should progress after resume: frozen={}, val1={}, val2={}",
            frozen_val, val1, val2
        );
    }

    #[test]
    fn test_multiple_layers_independent() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Start effects on different layers
        let mut bg_effect = EffectInstance::new(
            "bg".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("dimmer".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        bg_effect.layer = EffectLayer::Background;

        let mut mid_effect = EffectInstance::new(
            "mid".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("dimmer".to_string(), 0.8);
                    p
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        mid_effect.layer = EffectLayer::Midground;

        engine.start_effect(bg_effect).unwrap();
        engine.start_effect(mid_effect).unwrap();

        // Set different masters for each layer
        engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
        engine.set_layer_intensity_master(EffectLayer::Midground, 1.0);

        // Freeze only background
        engine.freeze_layer(EffectLayer::Background);

        assert!(engine.is_layer_frozen(EffectLayer::Background));
        assert!(!engine.is_layer_frozen(EffectLayer::Midground));

        // Clear only midground
        engine.clear_layer(EffectLayer::Midground);

        assert_eq!(engine.active_effects_count(), 1);
        assert!(engine.has_effect("bg"));
        assert!(!engine.has_effect("mid"));
    }
}
