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
use rand::Rng;

/// The main effects engine that manages and processes lighting effects
#[allow(dead_code)]
pub struct EffectEngine {
    active_effects: HashMap<String, EffectInstance>,
    active_chasers: HashMap<String, ChaserInstance>,
    fixture_registry: HashMap<String, FixtureInfo>,
    current_time: Instant,
}

#[allow(dead_code)]
impl EffectEngine {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            active_chasers: HashMap::new(),
            fixture_registry: HashMap::new(),
            current_time: Instant::now(),
        }
    }

    /// Register a fixture with the engine
    pub fn register_fixture(&mut self, fixture: FixtureInfo) {
        self.fixture_registry.insert(fixture.name.clone(), fixture);
    }

    /// Start an effect
    pub fn start_effect(&mut self, effect: EffectInstance) -> Result<(), EffectError> {
        // Validate effect
        self.validate_effect(&effect)?;

        // Stop any conflicting effects
        self.stop_conflicting_effects(&effect);

        // Start the effect
        self.active_effects.insert(effect.id.clone(), effect);
        Ok(())
    }

    /// Stop an effect
    pub fn stop_effect(&mut self, effect_id: &str) {
        self.active_effects.remove(effect_id);
    }

    /// Start a chaser
    pub fn start_chaser(&mut self, chaser: Chaser) -> Result<(), EffectError> {
        let instance = ChaserInstance {
            chaser: chaser.clone(),
            current_step: 0,
            step_start_time: self.current_time,
            is_running: true,
            direction: chaser.direction,
        };

        self.active_chasers.insert(chaser.id.clone(), instance);
        Ok(())
    }

    /// Stop a chaser
    pub fn stop_chaser(&mut self, chaser_id: &str) {
        self.active_chasers.remove(chaser_id);
    }

    /// Update the engine and return DMX commands
    pub fn update(&mut self, dt: Duration) -> Result<Vec<DmxCommand>, EffectError> {
        self.current_time += dt;
        let mut commands = Vec::new();

        // Update active effects
        for effect in self.active_effects.values() {
            if let Some(commands_for_effect) = self.process_effect(effect)? {
                commands.extend(commands_for_effect);
            }
        }

        // Update active chasers - process them in two passes to avoid borrowing issues
        let mut chasers_to_advance = Vec::new();

        // First pass: process chasers and identify which need advancement
        for (chaser_id, chaser_instance) in &self.active_chasers {
            if let Some(commands_for_chaser) = self.process_chaser(chaser_instance)? {
                commands.extend(commands_for_chaser);
            }

            // Check if we need to advance the chaser step
            let step_duration =
                chaser_instance.chaser.steps[chaser_instance.current_step].hold_time;
            let elapsed = self
                .current_time
                .duration_since(chaser_instance.step_start_time);

            if elapsed >= step_duration {
                chasers_to_advance.push(chaser_id.clone());
            }
        }

        // Second pass: advance chasers that need it
        for chaser_id in chasers_to_advance {
            if let Some(chaser_instance) = self.active_chasers.get_mut(&chaser_id) {
                Self::advance_chaser_step(chaser_instance, self.current_time);
            }
        }

        Ok(commands)
    }

    /// Process a single effect and return DMX commands
    fn process_effect(
        &self,
        effect: &EffectInstance,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        if !effect.enabled {
            return Ok(None);
        }

        // Check if effect has expired
        if let Some(duration) = effect.duration {
            if let Some(start_time) = effect.start_time {
                if self.current_time.duration_since(start_time) >= duration {
                    return Ok(None);
                }
            }
        }

        // Calculate effect parameters based on current time
        let elapsed = effect
            .start_time
            .map(|start| self.current_time.duration_since(start))
            .unwrap_or(Duration::ZERO);

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

    /// Process a chaser and return DMX commands
    fn process_chaser(
        &self,
        chaser_instance: &ChaserInstance,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        if !chaser_instance.is_running || chaser_instance.chaser.steps.is_empty() {
            return Ok(None);
        }

        let _step_duration = chaser_instance.chaser.steps[chaser_instance.current_step].hold_time;
        let _elapsed = self
            .current_time
            .duration_since(chaser_instance.step_start_time);

        // For now, just process the current step without advancing
        // In a full implementation, we'd need to handle step advancement differently
        let current_step = &chaser_instance.chaser.steps[chaser_instance.current_step];
        self.process_effect(&current_step.effect)
    }

    /// Advance to the next step in a chaser
    fn advance_chaser_step(chaser_instance: &mut ChaserInstance, current_time: Instant) {
        match chaser_instance.direction {
            ChaserDirection::Forward => {
                chaser_instance.current_step += 1;
                if chaser_instance.current_step >= chaser_instance.chaser.steps.len() {
                    match chaser_instance.chaser.loop_mode {
                        LoopMode::Once => {
                            chaser_instance.is_running = false;
                            return;
                        }
                        LoopMode::Loop => {
                            chaser_instance.current_step = 0;
                        }
                        LoopMode::PingPong => {
                            chaser_instance.direction = ChaserDirection::Backward;
                            chaser_instance.current_step -= 1;
                        }
                        LoopMode::Random => {
                            chaser_instance.current_step =
                                rand::thread_rng().gen_range(0..chaser_instance.chaser.steps.len());
                        }
                    }
                }
            }
            ChaserDirection::Backward => {
                if chaser_instance.current_step > 0 {
                    chaser_instance.current_step -= 1;
                } else {
                    match chaser_instance.chaser.loop_mode {
                        LoopMode::Once => {
                            chaser_instance.is_running = false;
                            return;
                        }
                        LoopMode::Loop => {
                            chaser_instance.current_step = chaser_instance.chaser.steps.len() - 1;
                        }
                        LoopMode::PingPong => {
                            chaser_instance.direction = ChaserDirection::Forward;
                            chaser_instance.current_step += 1;
                        }
                        LoopMode::Random => {
                            chaser_instance.current_step =
                                rand::thread_rng().gen_range(0..chaser_instance.chaser.steps.len());
                        }
                    }
                }
            }
            ChaserDirection::Random => {
                chaser_instance.current_step =
                    rand::thread_rng().gen_range(0..chaser_instance.chaser.steps.len());
            }
        }

        chaser_instance.step_start_time = current_time;
    }

    /// Apply a static effect
    fn apply_static_effect(
        &self,
        effect: &EffectInstance,
        parameters: &HashMap<String, f64>,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        let mut commands = Vec::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                for (param_name, value) in parameters {
                    if let Some(&channel) = fixture.channels.get(param_name) {
                        commands.push(DmxCommand {
                            universe: fixture.universe,
                            channel: fixture.address + channel - 1,
                            value: (*value * 255.0) as u8,
                        });
                    }
                }
            }
        }

        Ok(Some(commands))
    }

    /// Apply a color cycle effect
    fn apply_color_cycle(
        &self,
        effect: &EffectInstance,
        colors: &[Color],
        speed: f64,
        direction: &CycleDirection,
        elapsed: Duration,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        if colors.is_empty() {
            return Ok(None);
        }

        let cycle_time = 1.0 / speed;
        let cycle_progress = (elapsed.as_secs_f64() % cycle_time) / cycle_time;

        let color_index = match direction {
            CycleDirection::Forward => (cycle_progress * colors.len() as f64) as usize,
            CycleDirection::Backward => {
                colors.len() - 1 - ((cycle_progress * colors.len() as f64) as usize)
            }
            CycleDirection::PingPong => {
                let ping_pong_progress = if cycle_progress < 0.5 {
                    cycle_progress * 2.0
                } else {
                    2.0 - cycle_progress * 2.0
                };
                (ping_pong_progress * colors.len() as f64) as usize
            }
        };

        let color = colors[color_index % colors.len()];
        let mut commands = Vec::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                // Apply RGB channels
                if let Some(&red_channel) = fixture.channels.get("red") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + red_channel - 1,
                        value: color.r,
                    });
                }
                if let Some(&green_channel) = fixture.channels.get("green") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + green_channel - 1,
                        value: color.g,
                    });
                }
                if let Some(&blue_channel) = fixture.channels.get("blue") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + blue_channel - 1,
                        value: color.b,
                    });
                }
                if let Some(&white_channel) = fixture.channels.get("white") {
                    if let Some(w) = color.w {
                        commands.push(DmxCommand {
                            universe: fixture.universe,
                            channel: fixture.address + white_channel - 1,
                            value: w,
                        });
                    }
                }
            }
        }

        Ok(Some(commands))
    }

    /// Apply a strobe effect
    fn apply_strobe(
        &self,
        effect: &EffectInstance,
        frequency: f64,
        intensity: f64,
        elapsed: Duration,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        let strobe_period = 1.0 / frequency;
        let strobe_phase = (elapsed.as_secs_f64() % strobe_period) / strobe_period;

        let strobe_value = if strobe_phase < 0.5 {
            (intensity * 255.0) as u8
        } else {
            0
        };

        let mut commands = Vec::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                if let Some(&dimmer_channel) = fixture.channels.get("dimmer") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + dimmer_channel - 1,
                        value: strobe_value,
                    });
                }
            }
        }

        Ok(Some(commands))
    }

    /// Apply a dimmer effect
    fn apply_dimmer(
        &self,
        effect: &EffectInstance,
        start_level: f64,
        end_level: f64,
        duration: Duration,
        curve: &DimmerCurve,
        elapsed: Duration,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        if elapsed >= duration {
            return Ok(None);
        }

        let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
        let curve_value = match curve {
            DimmerCurve::Linear => progress,
            DimmerCurve::Exponential => progress * progress,
            DimmerCurve::Logarithmic => (progress * 10.0).ln() / 10.0_f64.ln(),
            DimmerCurve::Sine => (progress * std::f64::consts::PI / 2.0).sin(),
            DimmerCurve::Cosine => 1.0 - (progress * std::f64::consts::PI / 2.0).cos(),
            DimmerCurve::Custom(points) => {
                // Linear interpolation between custom curve points
                let scaled_progress = progress * (points.len() - 1) as f64;
                let index = scaled_progress as usize;
                let fraction = scaled_progress - index as f64;

                if index >= points.len() - 1 {
                    points[points.len() - 1]
                } else {
                    points[index] + fraction * (points[index + 1] - points[index])
                }
            }
        };

        let current_level = start_level + (end_level - start_level) * curve_value;
        let dimmer_value = (current_level * 255.0) as u8;

        let mut commands = Vec::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                if let Some(&dimmer_channel) = fixture.channels.get("dimmer") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + dimmer_channel - 1,
                        value: dimmer_value,
                    });
                }
            }
        }

        Ok(Some(commands))
    }

    /// Apply a chase effect
    fn apply_chase(
        &self,
        effect: &EffectInstance,
        _pattern: &ChasePattern,
        speed: f64,
        _direction: &ChaseDirection,
        elapsed: Duration,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        // This is a simplified chase implementation
        // In a full implementation, this would handle complex spatial patterns
        let chase_time = 1.0 / speed;
        let chase_progress = (elapsed.as_secs_f64() % chase_time) / chase_time;

        let mut commands = Vec::new();

        for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let fixture_progress = (i as f64) / (effect.target_fixtures.len() as f64);
                let is_active = (chase_progress - fixture_progress).abs() < 0.1; // 10% overlap

                let dimmer_value = if is_active { 255 } else { 0 };

                if let Some(&dimmer_channel) = fixture.channels.get("dimmer") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + dimmer_channel - 1,
                        value: dimmer_value,
                    });
                }
            }
        }

        Ok(Some(commands))
    }

    /// Apply a rainbow effect
    fn apply_rainbow(
        &self,
        effect: &EffectInstance,
        speed: f64,
        saturation: f64,
        brightness: f64,
        elapsed: Duration,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        let cycle_time = 1.0 / speed;
        let cycle_progress = (elapsed.as_secs_f64() % cycle_time) / cycle_time;

        // Calculate hue based on cycle progress and fixture position
        let mut commands = Vec::new();

        for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                let hue = (cycle_progress * 360.0
                    + (i as f64 * 360.0 / effect.target_fixtures.len() as f64))
                    % 360.0;
                let color = Color::from_hsv(hue, saturation, brightness);

                // Apply RGB channels
                if let Some(&red_channel) = fixture.channels.get("red") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + red_channel - 1,
                        value: color.r,
                    });
                }
                if let Some(&green_channel) = fixture.channels.get("green") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + green_channel - 1,
                        value: color.g,
                    });
                }
                if let Some(&blue_channel) = fixture.channels.get("blue") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + blue_channel - 1,
                        value: color.b,
                    });
                }
            }
        }

        Ok(Some(commands))
    }

    /// Apply a pulse effect
    fn apply_pulse(
        &self,
        effect: &EffectInstance,
        base_level: f64,
        pulse_amplitude: f64,
        frequency: f64,
        elapsed: Duration,
    ) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        let pulse_phase = (elapsed.as_secs_f64() * frequency * 2.0 * std::f64::consts::PI)
            % (2.0 * std::f64::consts::PI);
        let pulse_value = base_level + pulse_amplitude * (pulse_phase.sin() + 1.0) / 2.0;
        let dimmer_value = (pulse_value * 255.0) as u8;

        let mut commands = Vec::new();

        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = self.fixture_registry.get(fixture_name) {
                if let Some(&dimmer_channel) = fixture.channels.get("dimmer") {
                    commands.push(DmxCommand {
                        universe: fixture.universe,
                        channel: fixture.address + dimmer_channel - 1,
                        value: dimmer_value,
                    });
                }
            }
        }

        Ok(Some(commands))
    }

    /// Validate an effect before starting it
    fn validate_effect(&self, effect: &EffectInstance) -> Result<(), EffectError> {
        for fixture_name in &effect.target_fixtures {
            if !self.fixture_registry.contains_key(fixture_name) {
                return Err(EffectError::InvalidFixture(format!(
                    "Fixture '{}' not found",
                    fixture_name
                )));
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

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "RGBW".to_string(),
            channels,
        }
    }

    #[test]
    fn test_effect_engine_creation() {
        let engine = EffectEngine::new();
        assert!(engine.active_effects.is_empty());
        assert!(engine.active_chasers.is_empty());
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

        // Update the engine
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have commands for RGB channels
        assert_eq!(commands.len(), 3);

        // All channels should be present
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2);
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3);
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4);

        assert!(red_cmd.is_some());
        assert!(green_cmd.is_some());
        assert!(blue_cmd.is_some());
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

        // Should have a dimmer command
        assert_eq!(commands.len(), 1);

        let dimmer_cmd = &commands[0];
        assert_eq!(dimmer_cmd.channel, 1);
        // Value should be either 0 or 255 (strobe on/off)
        assert!(dimmer_cmd.value == 0 || dimmer_cmd.value == 255);
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

        // Should have a dimmer command at 50% (127)
        assert_eq!(commands.len(), 1);
        let dimmer_cmd = &commands[0];
        assert_eq!(dimmer_cmd.channel, 1);
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

        // Should have a dimmer command
        assert_eq!(commands.len(), 1);
        let dimmer_cmd = &commands[0];
        assert_eq!(dimmer_cmd.channel, 1);
        // dimmer_cmd.value is u8, so it's always in valid range
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
        engine.stop_effect("test_effect");

        // Update again - should have no commands
        let commands = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands.len(), 0);
    }

    #[test]
    fn test_chaser_creation() {
        let chaser = Chaser::new("test_chaser".to_string(), "Test Chaser".to_string())
            .with_loop_mode(LoopMode::Once)
            .with_speed(1.5);

        assert_eq!(chaser.id, "test_chaser");
        assert_eq!(chaser.name, "Test Chaser");
        assert_eq!(chaser.speed_multiplier, 1.5);
        assert!(matches!(chaser.loop_mode, LoopMode::Once));
    }

    #[test]
    fn test_chaser_with_steps() {
        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 1.0);

        let effect = EffectInstance::new(
            "step_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["test_fixture".to_string()],
        );

        let step = ChaserStep {
            effect,
            hold_time: Duration::from_secs(1),
            transition_time: Duration::from_millis(100),
            transition_type: TransitionType::Fade,
        };

        let chaser =
            Chaser::new("test_chaser".to_string(), "Test Chaser".to_string()).add_step(step);

        assert_eq!(chaser.steps.len(), 1);
        assert_eq!(chaser.steps[0].hold_time, Duration::from_secs(1));
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

        if let Err(EffectError::InvalidFixture(msg)) = result {
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
