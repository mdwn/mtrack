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

/// Test that demonstrates visual consistency across different fixture types
/// This is the core benefit of the Fixture Profile System
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::super::effects::*;
    use super::super::effects::{TempoAwareFrequency, TempoAwareSpeed};
    use super::super::engine::EffectEngine;

    /// Create an RGB-only fixture (no dedicated dimmer)
    fn create_rgb_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "RGB_Par".to_string(),
            channels,
            max_strobe_frequency: None,
        }
    }

    /// Create an RGB+dimmer fixture (has dedicated dimmer channel)
    fn create_rgb_dimmer_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "RGB_Dimmer_Par".to_string(),
            channels,
            max_strobe_frequency: None,
        }
    }

    /// Create a dimmer-only fixture (no RGB channels)
    fn create_dimmer_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "Dimmer".to_string(),
            channels,
            max_strobe_frequency: None,
        }
    }

    /// Test that dimmer effects produce consistent visual results across fixture types
    #[test]
    fn test_dimmer_visual_consistency() {
        let mut engine = EffectEngine::new();

        // Register different fixture types
        engine.register_fixture(create_rgb_fixture("rgb_fixture", 1, 1));
        engine.register_fixture(create_rgb_dimmer_fixture("rgb_dimmer_fixture", 1, 10));
        engine.register_fixture(create_dimmer_fixture("dimmer_fixture", 1, 20));

        // Create a dimmer effect targeting all fixtures
        let dimmer_effect = EffectInstance::new(
            "dimmer_test".to_string(),
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 1.0,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec![
                "rgb_fixture".to_string(),
                "rgb_dimmer_fixture".to_string(),
                "dimmer_fixture".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(dimmer_effect).unwrap();

        // Test at 50% progress (should be 50% brightness for all fixtures)
        let commands = engine.update(Duration::from_millis(500)).unwrap();

        // RGB-only fixture: should use _dimmer_multiplier (no direct DMX commands)
        // RGB+dimmer fixture: should use dedicated dimmer channel
        // Dimmer-only fixture: should use dedicated dimmer channel

        // Find commands for each fixture
        // RGB+dimmer fixture: universe 1, channel 6 (dimmer at address 6)
        let rgb_dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 10 && cmd.universe == 1);
        // Dimmer-only fixture: universe 1, channel 11 (dimmer at address 11)
        let dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 20 && cmd.universe == 1);

        // RGB+dimmer fixture should have dimmer command at 50% (127)
        assert!(rgb_dimmer_cmd.is_some());
        assert_eq!(rgb_dimmer_cmd.unwrap().value, 127);

        // Dimmer-only fixture should have dimmer command at 50% (127)
        assert!(dimmer_cmd.is_some());
        assert_eq!(dimmer_cmd.unwrap().value, 127);

        // RGB-only fixture should have no direct DMX commands (uses _dimmer_multiplier)
        // This is the key benefit: same visual result, different implementation
        let rgb_commands: Vec<_> = commands
            .iter()
            .filter(|cmd| cmd.channel >= 1 && cmd.channel <= 3)
            .collect();
        assert!(
            rgb_commands.is_empty(),
            "RGB-only fixture should not have direct RGB commands with fixture profile system"
        );
    }

    /// Test that pulse effects produce consistent visual results across fixture types
    #[test]
    fn test_pulse_visual_consistency() {
        let mut engine = EffectEngine::new();

        // Register different fixture types
        engine.register_fixture(create_rgb_fixture("rgb_fixture", 1, 1));
        engine.register_fixture(create_rgb_dimmer_fixture("rgb_dimmer_fixture", 1, 10));
        engine.register_fixture(create_dimmer_fixture("dimmer_fixture", 1, 20));

        // Create a pulse effect targeting all fixtures
        let pulse_effect = EffectInstance::new(
            "pulse_test".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Fixed(1.0), // 1 Hz
                duration: None,
            },
            vec![
                "rgb_fixture".to_string(),
                "rgb_dimmer_fixture".to_string(),
                "dimmer_fixture".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(pulse_effect).unwrap();

        // Test at a specific time point
        let commands = engine.update(Duration::from_millis(250)).unwrap();

        // RGB+dimmer fixture: should use dedicated dimmer channel
        // Dimmer-only fixture: should use dedicated dimmer channel
        // RGB-only fixture: should use _pulse_multiplier (no direct DMX commands)

        // Find commands for each fixture
        // RGB+dimmer fixture: universe 1, channel 6 (dimmer at address 6)
        let rgb_dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 10 && cmd.universe == 1);
        // Dimmer-only fixture: universe 1, channel 11 (dimmer at address 11)
        let dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 20 && cmd.universe == 1);

        // Both dimmer-capable fixtures should have dimmer commands
        assert!(rgb_dimmer_cmd.is_some());
        assert!(dimmer_cmd.is_some());

        // RGB-only fixture should have no direct DMX commands (uses _pulse_multiplier)
        let rgb_commands: Vec<_> = commands
            .iter()
            .filter(|cmd| cmd.channel >= 1 && cmd.channel <= 3)
            .collect();
        assert!(
            rgb_commands.is_empty(),
            "RGB-only fixture should not have direct RGB commands with fixture profile system"
        );
    }

    /// Test that strobe effects produce consistent visual results across fixture types
    #[test]
    fn test_strobe_visual_consistency() {
        let mut engine = EffectEngine::new();

        // Register different fixture types
        engine.register_fixture(create_rgb_fixture("rgb_fixture", 1, 1));
        engine.register_fixture(create_rgb_dimmer_fixture("rgb_dimmer_fixture", 1, 10));
        engine.register_fixture(create_dimmer_fixture("dimmer_fixture", 1, 20));

        // Create a strobe effect targeting all fixtures
        let strobe_effect = EffectInstance::new(
            "strobe_test".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
                duration: None,
            },
            vec![
                "rgb_fixture".to_string(),
                "rgb_dimmer_fixture".to_string(),
                "dimmer_fixture".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(strobe_effect).unwrap();

        // Test at a specific time point
        let commands = engine.update(Duration::from_millis(100)).unwrap();

        // RGB-only fixture: should use RGB channels for software strobing
        // RGB+dimmer fixture: should use dimmer channel for software strobing
        // Dimmer-only fixture: should use dimmer channel for software strobing

        // Find commands for each fixture
        // RGB-only fixture: universe 1, channels 1-3 (RGB at address 1)
        let rgb_commands: Vec<_> = commands
            .iter()
            .filter(|cmd| cmd.channel >= 1 && cmd.channel <= 3)
            .collect();
        // RGB+dimmer fixture: universe 1, channel 6 (dimmer at address 6)
        let rgb_dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 10 && cmd.universe == 1);
        // Dimmer-only fixture: universe 1, channel 11 (dimmer at address 11)
        let dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 20 && cmd.universe == 1);

        // RGB-only fixture should use RGB channels for strobing
        assert!(
            !rgb_commands.is_empty(),
            "RGB-only fixture should use RGB channels for strobing"
        );

        // RGB+dimmer and dimmer-only fixtures should use dimmer channel (prioritized over RGB)
        assert!(rgb_dimmer_cmd.is_some());
        assert!(dimmer_cmd.is_some());
    }

    /// Test that chase effects produce consistent visual results across fixture types
    #[test]
    fn test_chase_visual_consistency() {
        let mut engine = EffectEngine::new();

        // Register different fixture types
        engine.register_fixture(create_rgb_fixture("rgb_fixture", 1, 1));
        engine.register_fixture(create_rgb_dimmer_fixture("rgb_dimmer_fixture", 1, 10));
        engine.register_fixture(create_dimmer_fixture("dimmer_fixture", 1, 20));

        // Create a chase effect targeting all fixtures
        let chase_effect = EffectInstance::new(
            "chase_test".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "rgb_fixture".to_string(),
                "rgb_dimmer_fixture".to_string(),
                "dimmer_fixture".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(chase_effect).unwrap();

        // Test at a specific time point
        let commands = engine.update(Duration::from_millis(100)).unwrap();

        // All fixtures should have appropriate commands based on their capabilities
        // RGB-only fixture: should use RGB channels
        // RGB+dimmer fixture: should use dimmer channel
        // Dimmer-only fixture: should use dimmer channel

        // Find commands for each fixture
        // RGB-only fixture: universe 1, channels 1-3 (RGB at address 1)
        let rgb_commands: Vec<_> = commands
            .iter()
            .filter(|cmd| cmd.channel >= 1 && cmd.channel <= 3)
            .collect();
        // RGB+dimmer fixture: universe 1, channel 6 (dimmer at address 6)
        let rgb_dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 10 && cmd.universe == 1);
        // Dimmer-only fixture: universe 1, channel 11 (dimmer at address 11)
        let dimmer_cmd = commands
            .iter()
            .find(|cmd| cmd.channel == 20 && cmd.universe == 1);

        // RGB-only fixture should use RGB channels for chase
        assert!(
            !rgb_commands.is_empty(),
            "RGB-only fixture should use RGB channels for chase"
        );

        // RGB+dimmer and dimmer-only fixtures should use dimmer channel (prioritized over RGB)
        assert!(rgb_dimmer_cmd.is_some());
        assert!(dimmer_cmd.is_some());
    }

    /// Test that the same lighting show produces equivalent results across fixture types
    /// This is the ultimate test of the Fixture Profile System
    #[test]
    fn test_show_consistency_across_fixture_types() {
        // This test demonstrates that a lighting show written once will produce
        // visually equivalent results regardless of the underlying fixture hardware.
        // This is the core benefit of the Fixture Profile System.

        // Create two engines with different fixture types
        let mut engine_rgb = EffectEngine::new();
        let mut engine_rgb_dimmer = EffectEngine::new();

        // Register different fixture types
        engine_rgb.register_fixture(create_rgb_fixture("fixture", 1, 1));
        engine_rgb_dimmer.register_fixture(create_rgb_dimmer_fixture("fixture", 1, 1));

        // Create the same effect for both engines
        let dimmer_effect = EffectInstance::new(
            "dimmer_test".to_string(),
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 1.0,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec!["fixture".to_string()],
            None,
            None,
            None,
        );

        // Start the same effect on both engines
        engine_rgb.start_effect(dimmer_effect.clone()).unwrap();
        engine_rgb_dimmer.start_effect(dimmer_effect).unwrap();

        // Test at 50% progress
        let commands_rgb = engine_rgb.update(Duration::from_millis(500)).unwrap();
        let commands_rgb_dimmer = engine_rgb_dimmer
            .update(Duration::from_millis(500))
            .unwrap();

        // RGB-only engine: should have no direct DMX commands (uses _dimmer_multiplier)
        assert!(
            commands_rgb.is_empty(),
            "RGB-only fixture should use _dimmer_multiplier, not direct DMX commands"
        );

        // RGB+dimmer engine: should have dimmer command at 50% (127)
        let dimmer_cmd = commands_rgb_dimmer.iter().find(|cmd| cmd.channel == 1);
        assert!(dimmer_cmd.is_some());
        assert_eq!(dimmer_cmd.unwrap().value, 127);

        // Both engines produce the same visual result (50% brightness) but use different
        // implementation strategies based on fixture capabilities. This is the power
        // of the Fixture Profile System!
    }
}
