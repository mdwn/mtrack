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
use crate::lighting::effects::*;
use crate::lighting::engine::tests::common::create_test_fixture;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

fn create_fixture_with_strobe_offset(
    name: &str,
    universe: u16,
    address: u16,
    max_freq: f64,
    min_freq: f64,
    dmx_offset: u8,
) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);
    channels.insert("white".to_string(), 5);
    channels.insert("strobe".to_string(), 6);

    let mut fixture = FixtureInfo::new(
        name.to_string(),
        universe,
        address,
        "RGBW_Strobe".to_string(),
        channels,
        Some(max_freq),
    );
    fixture.min_strobe_frequency = Some(min_freq);
    fixture.strobe_dmx_offset = Some(dmx_offset);
    fixture
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

    let fixture = FixtureInfo::new(
        "test_fixture".to_string(),
        1,
        1,
        "RGB".to_string(),
        channels,
        None,
    );
    engine.register_fixture(fixture);

    // 2 Hz strobe = 500ms period, so 50% duty cycle transition at 250ms
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0ms: strobe_phase=0, which is < 0.5, so ON (dimmer=255)
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 255, "At t=0ms strobe should be ON");

    // At t=249ms: still in first half of period, should be ON
    let commands = engine.update(Duration::from_millis(249), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        dimmer_cmd.value, 255,
        "At t=249ms strobe should still be ON"
    );

    // At t=251ms: just past 50% of period, should be OFF
    let commands = engine.update(Duration::from_millis(2), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 0, "At t=251ms strobe should be OFF");

    // At t=500ms: start of new period, should be ON again
    let commands = engine.update(Duration::from_millis(249), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        dimmer_cmd.value, 255,
        "At t=500ms strobe should be ON again"
    );
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
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update the engine
    let commands = engine.update(Duration::from_millis(16), None).unwrap();

    // Should have strobe command since fixture has dedicated strobe channel
    assert_eq!(commands.len(), 1);

    // Check strobe command (frequency 2.0 / max 20.0 = 0.1 = 25 in DMX)
    let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap();
    assert_eq!(strobe_cmd.value, 25);
}

#[test]
fn test_clear_layer_resets_strobe_channel() {
    // Test that clearing a layer with a strobe effect resets the strobe channel to 0
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a strobe effect on foreground layer
    let mut strobe_effect = EffectInstance::new(
        "strobe_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(5.0), // 5 Hz
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    strobe_effect.layer = EffectLayer::Foreground;

    engine.start_effect(strobe_effect).unwrap();

    // Update to process the strobe effect
    let commands_before = engine.update(Duration::from_millis(16), None).unwrap();

    // Verify strobe channel has a non-zero value
    let strobe_cmd_before = commands_before.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_before.is_some(),
        "Should have strobe command before clear"
    );
    let strobe_value_before = strobe_cmd_before.unwrap().value;
    assert!(
        strobe_value_before > 0,
        "Strobe channel should be non-zero before clear: {}",
        strobe_value_before
    );

    // Clear the foreground layer
    engine.clear_layer(EffectLayer::Foreground);

    // Verify the effect is stopped
    assert_eq!(engine.active_effects_count(), 0);
    assert!(!engine.has_effect("strobe_effect"));

    // Update again - no active effects, so no strobe commands
    let commands_after = engine.update(Duration::from_millis(16), None).unwrap();

    // After clear, with no active effects, strobe channel is simply not set
    let strobe_cmd_after = commands_after.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_after.is_none(),
        "No strobe command after clear (no active effects)"
    );
}

#[test]
fn test_clear_all_layers_resets_strobe_channel() {
    // Test that clearing all layers removes strobe effects
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start strobe effects on multiple layers
    let mut bg_strobe = EffectInstance::new(
        "bg_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(3.0),
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    bg_strobe.layer = EffectLayer::Background;

    let mut fg_strobe = EffectInstance::new(
        "fg_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(4.0),
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_strobe.layer = EffectLayer::Foreground;

    engine.start_effect(bg_strobe).unwrap();
    engine.start_effect(fg_strobe).unwrap();

    // Update to process the strobe effects
    let commands_before = engine.update(Duration::from_millis(16), None).unwrap();

    // Verify strobe channel has a non-zero value
    let strobe_cmd_before = commands_before.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_before.is_some(),
        "Should have strobe command before clear"
    );
    let strobe_value_before = strobe_cmd_before.unwrap().value;
    assert!(
        strobe_value_before > 0,
        "Strobe channel should be non-zero before clear: {}",
        strobe_value_before
    );

    // Clear all layers
    engine.clear_all_layers();

    // Verify all effects are stopped
    assert_eq!(engine.active_effects_count(), 0);

    // Update again - no active effects, so no strobe commands
    let commands_after = engine.update(Duration::from_millis(16), None).unwrap();

    // After clear, with no active effects, strobe channel is simply not set
    let strobe_cmd_after = commands_after.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_after.is_none(),
        "No strobe command after clear_all_layers (no active effects)"
    );
}

#[test]
fn test_strobe_with_dmx_offset() {
    // Test that strobe normalization accounts for DMX offset and min frequency
    // PixelBrick: max=25Hz, min=0.4Hz, dmx_offset=7
    // At 10Hz: min_norm = 7/255 = 0.027, normalized = 0.027 + (9.6/24.6) * 0.973 = 0.407
    // DMX = 0.407 * 255 = ~104
    let mut engine = EffectEngine::new();
    let fixture = create_fixture_with_strobe_offset("test_fixture", 1, 1, 25.0, 0.4, 7);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap();

    // Period-linear interpolation:
    // max_period = 1/0.4 = 2.5s, min_period = 1/25 = 0.04s
    // desired_period = 1/10 = 0.1s
    // period_fraction = (2.5 - 0.1) / (2.5 - 0.04) = 2.4/2.46 = 0.97561
    // min_normalized = 7/255 = 0.02745
    // normalized = 0.02745 + 0.97561 * 0.97255 = 0.97628
    // DMX = (0.97628 * 255) as u8 = 248
    assert_eq!(
        strobe_cmd.value, 248,
        "10Hz strobe with offset should produce DMX 248"
    );
}

#[test]
fn test_strobe_without_offset_unchanged() {
    // Verify that fixtures without offset still work as before
    // Fixture with max=20Hz, no offset: 2Hz → 2/20 = 0.1 → DMX 25
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap();
    assert_eq!(
        strobe_cmd.value, 25,
        "2Hz strobe without offset should still produce DMX 25"
    );
}

/// Create a fixture matching the Astera PixelBrick: RGB + Strobe, no dimmer.
fn create_pixelbrick_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);

    let mut fixture = FixtureInfo::new(
        name.to_string(),
        universe,
        address,
        "Astera-PixelBrick".to_string(),
        channels,
        Some(25.0),
    );
    fixture.min_strobe_frequency = Some(0.4);
    fixture.strobe_dmx_offset = Some(7);
    fixture
}

#[test]
fn test_pixelbrick_strobe_with_concurrent_effects() {
    // Reproduce the Esaweg @236/1 scenario:
    // - Background: color cycle (replace)
    // - Midground: chase (add)
    // - Foreground: two chases (screen), pulse (overlay), strobe (overlay)
    // Verify the strobe channel appears in the DMX output.
    let mut engine = EffectEngine::new();
    let fixture = create_pixelbrick_fixture("Brick1", 1, 1);
    engine.register_fixture(fixture);

    // Background: color cycle
    let mut bg_cycle = EffectInstance::new(
        "bg_cycle".to_string(),
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 255, 255), Color::new(255, 68, 0)],
            speed: TempoAwareSpeed::Fixed(0.5),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    bg_cycle.layer = EffectLayer::Background;
    bg_cycle.blend_mode = BlendMode::Replace;
    engine.start_effect(bg_cycle).unwrap();

    // Midground: chase
    let mut mid_chase = EffectInstance::new(
        "mid_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Random,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    mid_chase.layer = EffectLayer::Midground;
    mid_chase.blend_mode = BlendMode::Add;
    engine.start_effect(mid_chase).unwrap();

    // Foreground: chase (screen)
    let mut fg_chase = EffectInstance::new(
        "fg_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Random,
            speed: TempoAwareSpeed::Fixed(2.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    fg_chase.layer = EffectLayer::Foreground;
    fg_chase.blend_mode = BlendMode::Screen;
    engine.start_effect(fg_chase).unwrap();

    // Foreground: pulse (overlay)
    let mut fg_pulse = EffectInstance::new(
        "fg_pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(3.0),
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    fg_pulse.layer = EffectLayer::Foreground;
    fg_pulse.blend_mode = BlendMode::Overlay;
    engine.start_effect(fg_pulse).unwrap();

    // Foreground: strobe 10Hz (overlay) — the effect under test
    let mut fg_strobe = EffectInstance::new(
        "fg_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    fg_strobe.layer = EffectLayer::Foreground;
    fg_strobe.blend_mode = BlendMode::Overlay;
    engine.start_effect(fg_strobe).unwrap();

    // Run several frames and verify strobe channel is always present
    for frame in 0..10 {
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Find the strobe DMX command: Brick1 address=1, strobe offset=4, so DMX channel = 1+4-1 = 4
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 4);
        assert!(
            strobe_cmd.is_some(),
            "Frame {}: strobe DMX channel 4 missing from output (got {} commands: {:?})",
            frame,
            commands.len(),
            commands
                .iter()
                .map(|c| (c.channel, c.value))
                .collect::<Vec<_>>()
        );

        let strobe_value = strobe_cmd.unwrap().value;
        // 10Hz with PixelBrick params (period-linear) should give DMX 248
        assert_eq!(
            strobe_value, 248,
            "Frame {}: strobe DMX value should be 248, got {}",
            frame, strobe_value
        );

        // Also verify RGB channels are present (from other effects)
        let has_red = commands.iter().any(|cmd| cmd.channel == 1);
        let has_green = commands.iter().any(|cmd| cmd.channel == 2);
        let has_blue = commands.iter().any(|cmd| cmd.channel == 3);
        assert!(has_red, "Frame {}: red channel missing", frame);
        assert!(has_green, "Frame {}: green channel missing", frame);
        assert!(has_blue, "Frame {}: blue channel missing", frame);
    }
}

#[test]
fn test_esaweg_timeline_strobe_sequence() {
    // Full timeline simulation of the Esaweg show's final section:
    // @221/1: clear + cycle bg + chases fg + chase mid
    // @228/1: pulse fg (added alongside existing effects)
    // @236/1: strobe fg (added alongside existing effects)
    // @236/4.75: clear + black static
    //
    // This verifies the strobe is visible for the 1.4s window between
    // @236/1 and @236/4.75.
    let mut engine = EffectEngine::new();
    let fixture = create_pixelbrick_fixture("Brick1", 1, 1);
    engine.register_fixture(fixture);

    let dt = Duration::from_millis(23); // ~44Hz

    // --- @221/1: Start climax section ---
    engine.clear_all_layers();

    // Background: color cycle (replace)
    let mut bg_cycle = EffectInstance::new(
        "bg_cycle".to_string(),
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 255, 255), Color::new(255, 68, 0)],
            speed: TempoAwareSpeed::Fixed(0.5),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    bg_cycle.layer = EffectLayer::Background;
    bg_cycle.blend_mode = BlendMode::Replace;
    engine.start_effect(bg_cycle).unwrap();

    // Foreground: left chase (screen)
    let mut left_chase = EffectInstance::new(
        "left_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Random,
            speed: TempoAwareSpeed::Fixed(2.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    left_chase.layer = EffectLayer::Foreground;
    left_chase.blend_mode = BlendMode::Screen;
    engine.start_effect(left_chase).unwrap();

    // Foreground: right chase (screen)
    let mut right_chase = EffectInstance::new(
        "right_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Random,
            speed: TempoAwareSpeed::Fixed(2.0),
            direction: ChaseDirection::RightToLeft,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    right_chase.layer = EffectLayer::Foreground;
    right_chase.blend_mode = BlendMode::Screen;
    engine.start_effect(right_chase).unwrap();

    // Midground: all_wash chase (add)
    let mut mid_chase = EffectInstance::new(
        "mid_chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Random,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    mid_chase.layer = EffectLayer::Midground;
    mid_chase.blend_mode = BlendMode::Add;
    engine.start_effect(mid_chase).unwrap();

    // Run for several frames (simulating @221/1 to @228/1)
    for _ in 0..20 {
        let commands = engine.update(dt, None).unwrap();
        // No strobe channel should be present yet
        let strobe = commands.iter().find(|c| c.channel == 4);
        assert!(
            strobe.is_none(),
            "Strobe channel should not be present before @228/1"
        );
    }

    // --- @228/1: Add pulse on foreground ---
    let mut fg_pulse = EffectInstance::new(
        "fg_pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(3.0),
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    fg_pulse.layer = EffectLayer::Foreground;
    fg_pulse.blend_mode = BlendMode::Overlay;
    engine.start_effect(fg_pulse).unwrap();

    // Run for several frames (simulating @228/1 to @236/1)
    for _ in 0..40 {
        let commands = engine.update(dt, None).unwrap();
        // Still no strobe channel
        let strobe = commands.iter().find(|c| c.channel == 4);
        assert!(
            strobe.is_none(),
            "Strobe channel should not be present before @236/1"
        );
    }

    // --- @236/1: Add strobe on foreground (the critical moment) ---
    let mut fg_strobe = EffectInstance::new(
        "fg_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    fg_strobe.layer = EffectLayer::Foreground;
    fg_strobe.blend_mode = BlendMode::Overlay;
    engine.start_effect(fg_strobe).unwrap();

    // Verify: all 6 effects should be active (pulse doesn't conflict with strobe)
    assert_eq!(
        engine.active_effects_count(),
        6,
        "All 6 effects should be active: bg_cycle, left_chase, right_chase, mid_chase, fg_pulse, fg_strobe"
    );

    // Run for ~1.4 seconds (61 frames at 23ms = 1403ms)
    // This simulates the strobe window from @236/1 to @236/4.75
    let mut strobe_present_count = 0;
    for frame in 0..61 {
        let commands = engine.update(dt, None).unwrap();

        // Strobe channel (Brick1 addr=1, strobe offset=4, DMX channel = 4) should be present
        let strobe_cmd = commands.iter().find(|c| c.channel == 4);
        assert!(
            strobe_cmd.is_some(),
            "Frame {} after strobe start: strobe channel 4 should be present (got {:?})",
            frame,
            commands
                .iter()
                .map(|c| (c.channel, c.value))
                .collect::<Vec<_>>()
        );

        let strobe_value = strobe_cmd.unwrap().value;
        assert_eq!(
            strobe_value, 248,
            "Frame {} after strobe start: strobe should be DMX 248, got {}",
            frame, strobe_value
        );
        strobe_present_count += 1;

        // RGB channels should also be present
        assert!(
            commands.iter().any(|c| c.channel == 1),
            "Frame {}: red missing",
            frame
        );
    }
    assert_eq!(
        strobe_present_count, 61,
        "Strobe should be present on all 61 frames"
    );

    // --- @236/4.75: Clear all + black static ---
    engine.clear_all_layers();

    // Add black static
    let mut black_static = EffectInstance::new(
        "black_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 0.0);
                p.insert("green".to_string(), 0.0);
                p.insert("blue".to_string(), 0.0);
                p
            },
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    black_static.layer = EffectLayer::Background;
    black_static.blend_mode = BlendMode::Replace;
    engine.start_effect(black_static).unwrap();

    // After clear: strobe channel is not set (no strobe effect active),
    // RGB should be 0 from the black static effect
    let commands = engine.update(dt, None).unwrap();

    // Strobe channel is not emitted after clear (no active strobe effect)
    let strobe_cmd = commands.iter().find(|c| c.channel == 4);
    assert!(
        strobe_cmd.is_none(),
        "Strobe channel should not be present after clear (no active strobe)"
    );

    let red_cmd = commands.iter().find(|c| c.channel == 1);
    assert!(
        red_cmd.is_some(),
        "Red channel should be present after clear"
    );
    assert_eq!(red_cmd.unwrap().value, 0, "Red should be 0 (black)");
}

#[test]
fn test_pixelbrick_orange_static_with_strobe_dmx_values() {
    // Verify the exact DMX values sent for a PixelBrick fixture with:
    // - Static orange color (R=255, G=102, B=0) on the background
    // - Strobe at 10Hz on the foreground (overlay)
    //
    // PixelBrick channel map: red=1, green=2, blue=3, strobe=4
    // PixelBrick address=1, universe=1, so DMX channels are 1,2,3,4
    let mut engine = EffectEngine::new();
    let fixture = create_pixelbrick_fixture("Brick1", 1, 1);
    engine.register_fixture(fixture);

    // Background: static orange (R=1.0, G=0.4, B=0.0)
    let orange_static = EffectInstance::new(
        "orange_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 1.0);
                p.insert("green".to_string(), 0.4);
                p.insert("blue".to_string(), 0.0);
                p
            },
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(orange_static).unwrap();

    // Foreground: strobe at 10Hz (overlay)
    let mut strobe = EffectInstance::new(
        "strobe_10hz".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::from_secs(5),
        },
        vec!["Brick1".to_string()],
        None,
        None,
        None,
    );
    strobe.layer = EffectLayer::Foreground;
    strobe.blend_mode = BlendMode::Overlay;
    engine.start_effect(strobe).unwrap();

    // Update and collect all DMX commands
    let commands = engine.update(Duration::from_millis(16), None).unwrap();

    // Sort commands by channel for deterministic inspection
    let mut sorted: Vec<(u16, u8)> = commands.iter().map(|c| (c.channel, c.value)).collect();
    sorted.sort_by_key(|&(ch, _)| ch);

    // Print all DMX commands for diagnostic visibility
    eprintln!("DMX commands for PixelBrick orange+strobe: {:?}", sorted);

    // Verify we have exactly 4 commands (red, green, blue, strobe)
    assert_eq!(
        sorted.len(),
        4,
        "Expected 4 DMX commands (R,G,B,strobe), got {}: {:?}",
        sorted.len(),
        sorted
    );

    // Channel 1 = red: 1.0 * 255 = 255
    let red = sorted.iter().find(|&&(ch, _)| ch == 1).unwrap();
    assert_eq!(red.1, 255, "Red channel should be 255, got {}", red.1);

    // Channel 2 = green: 0.4 * 255 = 102
    let green = sorted.iter().find(|&&(ch, _)| ch == 2).unwrap();
    assert_eq!(green.1, 102, "Green channel should be 102, got {}", green.1);

    // Channel 3 = blue: 0.0 * 255 = 0
    let blue = sorted.iter().find(|&&(ch, _)| ch == 3).unwrap();
    assert_eq!(blue.1, 0, "Blue channel should be 0, got {}", blue.1);

    // Channel 4 = strobe: 10Hz with PixelBrick params (period-linear, max=25, min=0.4, offset=7) → DMX 248
    let strobe_val = sorted.iter().find(|&&(ch, _)| ch == 4).unwrap();
    assert_eq!(
        strobe_val.1, 248,
        "Strobe channel should be 248, got {}",
        strobe_val.1
    );
}
