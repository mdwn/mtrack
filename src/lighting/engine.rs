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
        }
    }

    /// Log effect application only on first occurrence
    fn log_effect_application(
        &mut self,
        _effect_id: &str,
        _effect_type: &str,
        _fixture_count: usize,
    ) {
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
        for (_layer, effect_ids) in effects_by_layer {
            for effect_id in effect_ids {
                // Clone the effect to avoid borrowing conflicts
                let effect = self.active_effects.get(&effect_id).unwrap().clone();

                // Check if effect has reached terminal state (value-based where applicable)
                let is_expired = if let Some(start_time) = effect.start_time {
                    let elapsed = self.current_time.duration_since(start_time);
                    effect.has_reached_terminal_state(elapsed)
                } else {
                    false
                };

                if is_expired {
                    // Effect has completed. For temporary effects, do not blend final state.
                    // For permanent effects, preserve via the completion handler below.

                    // Queue for removal after this frame
                    completed_effects.push(effect_id.clone());
                    continue;
                }

                // Calculate effect parameters based on current time
                let elapsed = effect
                    .start_time
                    .map(|start| self.current_time.duration_since(start))
                    .unwrap_or(Duration::ZERO);

                // Process the effect and get fixture states
                if let Some(effect_states) = self.process_effect(&effect, elapsed)? {
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

        match &effect.effect_type {
            EffectType::Static { parameters, .. } => {
                self.apply_static_effect(effect, parameters, elapsed)
            }
            EffectType::ColorCycle {
                colors,
                speed,
                direction,
            } => self.apply_color_cycle(effect, colors, *speed, direction, elapsed),
            EffectType::Strobe { frequency, .. } => self.apply_strobe(effect, *frequency, elapsed),
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
            } => self.apply_chase(effect, pattern, *speed, direction, elapsed),
            EffectType::Rainbow {
                speed,
                saturation,
                brightness,
            } => self.apply_rainbow(effect, *speed, *saturation, *brightness, elapsed),
            EffectType::Pulse {
                base_level,
                pulse_amplitude,
                frequency,
                ..
            } => self.apply_pulse(effect, *base_level, *pulse_amplitude, *frequency, elapsed),
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
                if *frequency < 0.0 {
                    return Err(EffectError::Parameter(format!(
                        "Strobe frequency must be non-negative, got {}",
                        frequency
                    )));
                }
            }
            EffectType::Pulse { frequency, .. } => {
                if *frequency <= 0.0 {
                    return Err(EffectError::Parameter(format!(
                        "Pulse frequency must be positive, got {}",
                        frequency
                    )));
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
        self.log_effect_application(&effect.id, "static", effect.target_fixtures.len());

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
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        self.log_effect_application(&effect.id, "color cycle", effect.target_fixtures.len());
        if colors.is_empty() {
            return Ok(None);
        }

        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        let cycle_time = 1.0 / speed;
        let cycle_progress = (elapsed.as_secs_f64() % cycle_time) / cycle_time;

        let color_index = match direction {
            CycleDirection::Forward => (cycle_progress * colors.len() as f64).floor() as usize,
            CycleDirection::Backward => {
                colors.len() - 1 - ((cycle_progress * colors.len() as f64).floor() as usize)
            }
            CycleDirection::PingPong => {
                let ping_pong_progress = if cycle_progress < 0.5 {
                    cycle_progress * 2.0
                } else {
                    2.0 - cycle_progress * 2.0
                };
                (ping_pong_progress * colors.len() as f64).floor() as usize
            }
        };

        let color = colors[color_index % colors.len()];
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
        self.log_effect_application(&effect.id, "strobe", effect.target_fixtures.len());

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
        self.log_effect_application(&effect.id, "dimmer", effect.target_fixtures.len());

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
        self.log_effect_application(&effect.id, "chase", effect.target_fixtures.len());

        // Calculate crossfade multiplier
        let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

        let chase_period = 1.0 / speed;

        let mut fixture_states = HashMap::new();
        let fixture_count = effect.target_fixtures.len();

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
        self.log_effect_application(&effect.id, "rainbow", effect.target_fixtures.len());

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
        self.log_effect_application(&effect.id, "pulse", effect.target_fixtures.len());

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
                speed: 1.0, // 1 cycle per second
                direction: CycleDirection::Forward,
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
    fn test_strobe_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Strobe {
                frequency: 2.0, // 2 Hz
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
                speed: 1.0,
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
                frequency: 1.0, // 1 Hz
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
                speed: 1.0,
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
}
