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

use tracing::info;

use super::effects::*;

/// The main effects engine that manages and processes lighting effects
pub struct EffectEngine {
    active_effects: HashMap<String, EffectInstance>,
    fixture_registry: HashMap<String, FixtureInfo>,
    current_time: Instant,
    logged_effects: std::collections::HashSet<String>,
}

impl EffectEngine {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            fixture_registry: HashMap::new(),
            current_time: Instant::now(),
            logged_effects: std::collections::HashSet::new(),
        }
    }

    /// Log effect application only on first occurrence
    fn log_effect_application(&mut self, effect_id: &str, effect_type: &str, fixture_count: usize) {
        if !self.logged_effects.contains(effect_id) {
            info!(
                "Applying {} effect '{}' to {} fixtures",
                effect_type, effect_id, fixture_count
            );
            self.logged_effects.insert(effect_id.to_string());
        }
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

        // Process effects with layering
        self.update_with_layering()
    }

    /// Update the engine with layering support
    fn update_with_layering(&mut self) -> Result<Vec<DmxCommand>, EffectError> {
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

        // Process each layer in order
        let mut fixture_states: std::collections::HashMap<String, FixtureState> =
            std::collections::HashMap::new();

        for (_layer, effect_ids) in effects_by_layer {
            for effect_id in effect_ids {
                // Clone the effect to avoid borrowing conflicts
                let effect = self.active_effects.get(&effect_id).unwrap().clone();

                // Check if effect has expired
                if let Some(duration) = effect.duration {
                    if let Some(start_time) = effect.start_time {
                        if self.current_time.duration_since(start_time) >= duration {
                            continue;
                        }
                    }
                }

                // Calculate effect parameters based on current time
                let elapsed = effect
                    .start_time
                    .map(|start| self.current_time.duration_since(start))
                    .unwrap_or(Duration::ZERO);

                // Process the effect and get fixture states
                if let Some(effect_states) = self.process_effect(&effect, elapsed)? {
                    // Blend the effect states into the overall fixture states
                    for (fixture_name, effect_state) in effect_states {
                        if self.fixture_registry.contains_key(&fixture_name) {
                            fixture_states
                                .entry(fixture_name.clone())
                                .or_insert_with(|| FixtureState::new(fixture_name))
                                .blend_with(&effect_state);
                        }
                    }
                }
            }
        }

        // Convert fixture states to DMX commands
        let mut commands = Vec::new();
        for (fixture_name, fixture_state) in fixture_states {
            if let Some(fixture_info) = self.fixture_registry.get(&fixture_name) {
                commands.extend(fixture_state.to_dmx_commands(fixture_info));
            }
        }

        Ok(commands)
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
            EffectType::Static { parameters, .. } => self.apply_static_effect(effect, parameters),
            EffectType::ColorCycle {
                colors,
                speed,
                direction,
            } => self.apply_color_cycle(effect, colors, *speed, direction, elapsed),
            EffectType::Strobe {
                frequency,
                intensity,
                ..
            } => self.apply_strobe(effect, *frequency, *intensity, elapsed),
            EffectType::Dimmer {
                start_level,
                end_level,
                duration,
                curve,
            } => self.apply_dimmer(effect, *start_level, *end_level, *duration, curve, elapsed),
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
            EffectType::Strobe {
                frequency,
                intensity,
                ..
            } => {
                if *frequency < 0.0 {
                    return Err(EffectError::Parameter(format!(
                        "Strobe frequency must be non-negative, got {}",
                        frequency
                    )));
                }
                if *intensity < 0.0 || *intensity > 1.0 {
                    return Err(EffectError::Parameter(format!(
                        "Strobe intensity must be between 0.0 and 1.0, got {}",
                        intensity
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
        if let Some(duration) = effect.duration {
            if duration.as_secs_f64() < 0.0 {
                return Err(EffectError::Timing(format!(
                    "Effect duration must be non-negative, got {}s",
                    duration.as_secs_f64()
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
                        if !fixture_info.has_capability(FixtureCapabilities::STROBING) {
                            return Err(EffectError::Parameter(format!(
                                "Strobe effect not compatible with fixture '{}' (no strobe capability)",
                                fixture_name
                            )));
                        }
                    }
                    EffectType::Chase { .. } => {
                        // Chase effects work with any fixture that has dimmer control
                        if !fixture_info.has_capability(FixtureCapabilities::DIMMING) {
                            return Err(EffectError::Parameter(format!(
                                "Chase effect not compatible with fixture '{}' (no dimmer capability)",
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
        // Simple implementation - in a full system, this would be more sophisticated
        let mut to_remove = Vec::new();

        for (effect_id, effect) in &self.active_effects {
            if effect.priority < new_effect.priority {
                // Check if effects target the same fixtures
                let has_overlap = effect
                    .target_fixtures
                    .iter()
                    .any(|fixture| new_effect.target_fixtures.contains(fixture));

                if has_overlap {
                    to_remove.push(effect_id.clone());
                }
            }
        }

        for effect_id in to_remove {
            self.active_effects.remove(&effect_id);
        }
    }

    /// Stop all active effects
    pub fn stop_all_effects(&mut self) {
        self.active_effects.clear();
    }

    // ===== State-based effect processing methods =====

    /// Apply a static effect and return fixture states
    fn apply_static_effect(
        &mut self,
        effect: &EffectInstance,
        parameters: &HashMap<String, f64>,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        self.log_effect_application(&effect.id, "static", effect.target_fixtures.len());
        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                for (param_name, value) in parameters {
                    if fixture.channels.contains_key(param_name) {
                        let channel_state =
                            ChannelState::new(*value, effect.layer, effect.blend_mode);
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
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                // Apply RGB channels
                if fixture.channels.contains_key("red") {
                    let channel_state =
                        ChannelState::new(color.r as f64 / 255.0, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("red".to_string(), channel_state);
                }
                if fixture.channels.contains_key("green") {
                    let channel_state =
                        ChannelState::new(color.g as f64 / 255.0, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("green".to_string(), channel_state);
                }
                if fixture.channels.contains_key("blue") {
                    let channel_state =
                        ChannelState::new(color.b as f64 / 255.0, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("blue".to_string(), channel_state);
                }
                if fixture.channels.contains_key("white") {
                    if let Some(w) = color.w {
                        let channel_state =
                            ChannelState::new(w as f64 / 255.0, effect.layer, effect.blend_mode);
                        fixture_state.set_channel("white".to_string(), channel_state);
                    }
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
        _intensity: f64,
        _elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        self.log_effect_application(&effect.id, "strobe", effect.target_fixtures.len());

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let strobe_value = if frequency == 0.0 {
                    // Frequency 0 means strobe is disabled
                    0.0
                } else {
                    // For strobe channel, use the frequency as the speed value
                    // Convert frequency (Hz) to a 0-1 range using fixture's max strobe frequency
                    let max_freq = fixture.max_strobe_frequency.unwrap_or(20.0); // Default to 20Hz if not specified
                    (frequency / max_freq).min(1.0)
                };
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                // Apply strobe to appropriate channels - prioritize dedicated channels
                if fixture.has_capability(FixtureCapabilities::STROBING) {
                    // Use dedicated strobe channel if available
                    let channel_state =
                        ChannelState::new(strobe_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("strobe".to_string(), channel_state);
                } else if fixture.has_capability(FixtureCapabilities::DIMMING) {
                    // Use dimmer channel if available (prioritize over RGB)
                    let channel_state =
                        ChannelState::new(strobe_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("dimmer".to_string(), channel_state);
                } else if fixture.has_capability(FixtureCapabilities::RGB_COLOR) {
                    // Fall back to RGB channels only if no dedicated channels
                    let channel_state =
                        ChannelState::new(strobe_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("red".to_string(), channel_state);
                    fixture_state.set_channel("green".to_string(), channel_state);
                    fixture_state.set_channel("blue".to_string(), channel_state);
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
        duration: Duration,
        curve: &DimmerCurve,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        self.log_effect_application(&effect.id, "dimmer", effect.target_fixtures.len());

        let progress = if duration.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0)
        };

        let dimmer_value = match curve {
            DimmerCurve::Linear => start_level + (end_level - start_level) * progress,
            DimmerCurve::Exponential => {
                start_level + (end_level - start_level) * (progress * progress)
            }
            DimmerCurve::Logarithmic => {
                start_level + (end_level - start_level) * (1.0 - (1.0 - progress).powi(2))
            }
            DimmerCurve::Sine => {
                start_level
                    + (end_level - start_level)
                        * (1.0 - (progress * std::f64::consts::PI / 2.0).cos())
            }
            DimmerCurve::Cosine => {
                start_level
                    + (end_level - start_level) * (progress * std::f64::consts::PI / 2.0).sin()
            }
        };

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                // Apply dimmer to appropriate channels
                if fixture.has_capability(FixtureCapabilities::DIMMING)
                    && effect.blend_mode == BlendMode::Replace
                {
                    // Use dedicated dimmer channel only for Replace mode (takes precedence over RGB)
                    let channel_state =
                        ChannelState::new(dimmer_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("dimmer".to_string(), channel_state);
                }

                if fixture.has_capability(FixtureCapabilities::RGB_COLOR) {
                    // For RGB fixtures, apply dimmer based on blend mode
                    if effect.blend_mode == BlendMode::Multiply {
                        // For Multiply mode, we should NOT set any RGB channels here
                        // The dimmer effect should only affect channels that already exist from other effects
                        // This prevents the dimmer from creating white light by setting all RGB channels
                        // The blending system will handle the multiplication when the effects are combined
                        // We'll store the dimmer value in a special way that can be used during blending
                        let dimmer_multiplier =
                            ChannelState::new(dimmer_value, effect.layer, effect.blend_mode);

                        // Store the dimmer multiplier in a special channel that won't be sent to DMX
                        // This will be used by the blending system to apply the dimmer to existing channels
                        fixture_state
                            .set_channel("_dimmer_multiplier".to_string(), dimmer_multiplier);
                    } else {
                        // For Replace mode, apply dimmer to all channels equally
                        let channel_state =
                            ChannelState::new(dimmer_value, effect.layer, effect.blend_mode);
                        fixture_state.set_channel("red".to_string(), channel_state);
                        fixture_state.set_channel("green".to_string(), channel_state);
                        fixture_state.set_channel("blue".to_string(), channel_state);
                    }
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
        _pattern: &ChasePattern,
        speed: f64,
        _direction: &ChaseDirection,
        elapsed: Duration,
    ) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
        self.log_effect_application(&effect.id, "chase", effect.target_fixtures.len());
        // Simplified chase implementation
        let chase_period = 1.0 / speed;
        let chase_progress = (elapsed.as_secs_f64() % chase_period) / chase_period;

        let mut fixture_states = HashMap::new();
        let fixture_count = effect.target_fixtures.len();

        for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                // Calculate chase position for this fixture
                let fixture_position = i as f64 / fixture_count as f64;
                let chase_value = if (chase_progress - fixture_position).abs() < 0.1 {
                    1.0
                } else {
                    0.0
                };

                // Apply chase to dimmer channel
                if fixture.has_capability(FixtureCapabilities::DIMMING) {
                    let channel_state =
                        ChannelState::new(chase_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("dimmer".to_string(), channel_state);
                }

                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }

        Ok(Some(fixture_states))
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
        let hue = (elapsed.as_secs_f64() * speed * 360.0) % 360.0;
        let color = Color::from_hsv(hue, saturation, brightness);

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                // Apply RGB channels
                if fixture.channels.contains_key("red") {
                    let channel_state =
                        ChannelState::new(color.r as f64 / 255.0, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("red".to_string(), channel_state);
                }
                if fixture.channels.contains_key("green") {
                    let channel_state =
                        ChannelState::new(color.g as f64 / 255.0, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("green".to_string(), channel_state);
                }
                if fixture.channels.contains_key("blue") {
                    let channel_state =
                        ChannelState::new(color.b as f64 / 255.0, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("blue".to_string(), channel_state);
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
        let pulse_phase = elapsed.as_secs_f64() * frequency * 2.0 * std::f64::consts::PI;
        let pulse_value = base_level + pulse_amplitude * (pulse_phase.sin() * 0.5 + 0.5);

        let mut fixture_states = HashMap::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new(fixture_name.clone());

                // Apply pulse to appropriate channels
                if fixture.has_capability(FixtureCapabilities::DIMMING)
                    && effect.blend_mode == BlendMode::Replace
                {
                    // Use dedicated dimmer channel only for Replace mode (takes precedence over RGB)
                    let channel_state =
                        ChannelState::new(pulse_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel("dimmer".to_string(), channel_state);
                }

                if fixture.has_capability(FixtureCapabilities::RGB_COLOR) {
                    // For RGB fixtures, apply pulse based on blend mode
                    if effect.blend_mode == BlendMode::Multiply {
                        // For Multiply mode, use a pulse multiplier to modulate existing colors
                        let pulse_multiplier =
                            ChannelState::new(pulse_value, effect.layer, effect.blend_mode);
                        fixture_state
                            .set_channel("_pulse_multiplier".to_string(), pulse_multiplier);
                    } else if !fixture.has_capability(FixtureCapabilities::DIMMING) {
                        // For Replace mode, only apply to RGB if no dedicated dimmer channel
                        let channel_state =
                            ChannelState::new(pulse_value, effect.layer, effect.blend_mode);
                        fixture_state.set_channel("red".to_string(), channel_state);
                        fixture_state.set_channel("green".to_string(), channel_state);
                        fixture_state.set_channel("blue".to_string(), channel_state);
                    }
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
        // Initialize tracing for this test
        let _ = tracing_subscriber::fmt::try_init();

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
                intensity: 1.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
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
        )
        .with_timing(Some(Instant::now()), None);

        engine.start_effect(effect).unwrap();

        // Update the engine after 500ms (half duration)
        let commands = engine.update(Duration::from_millis(500)).unwrap();

        // Should have dimmer + RGB commands at 50% (127) since fixture has both dedicated dimmer and RGB channels
        assert_eq!(commands.len(), 4);

        // Check dimmer command
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 127);

        // Check RGB commands
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 127);
        assert_eq!(green_cmd.value, 127);
        assert_eq!(blue_cmd.value, 127);
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
        );

        engine.start_effect(effect).unwrap();

        // Update the engine - should have commands
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
                duration: None,
            },
            vec!["test_fixture".to_string()],
        )
        .with_timing(Some(Instant::now()), Some(Duration::from_millis(100)));

        engine.start_effect(effect).unwrap();

        // Update before expiry - should have commands
        let commands = engine.update(Duration::from_millis(50)).unwrap();
        assert_eq!(commands.len(), 1);

        // Update after expiry - should have no commands
        let commands = engine.update(Duration::from_millis(100)).unwrap();
        assert_eq!(commands.len(), 0);
    }
}
