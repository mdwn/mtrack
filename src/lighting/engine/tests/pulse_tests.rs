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

use std::time::Duration;

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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    // 0.5 * 255 ≈ 127
    assert!(
        (120..=135).contains(&dimmer_cmd.value),
        "At t=0ms pulse should be ~127 (mid), got {}",
        dimmer_cmd.value
    );

    // At t=250ms: phase=π/2, sin(π/2)=1, pulse_value = 0 + 1.0 * (1 * 0.5 + 0.5) = 1.0 (peak)
    let commands = engine.update(Duration::from_millis(250), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert!(
        dimmer_cmd.value >= 250,
        "At t=250ms pulse should be at peak (~255), got {}",
        dimmer_cmd.value
    );

    // At t=750ms: phase=3π/2, sin(3π/2)=-1, pulse_value = 0 + 1.0 * (-1 * 0.5 + 0.5) = 0.0 (trough)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert!(
        dimmer_cmd.value <= 5,
        "At t=750ms pulse should be at trough (~0), got {}",
        dimmer_cmd.value
    );

    // At t=1000ms: should be back to mid-point
    let commands = engine.update(Duration::from_millis(250), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert!(
        (120..=135).contains(&dimmer_cmd.value),
        "At t=1000ms pulse should be back to ~127 (mid), got {}",
        dimmer_cmd.value
    );
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
    let commands = engine.update(Duration::from_millis(16), None).unwrap();

    // Should have dimmer command since fixture has dedicated dimmer channel
    assert_eq!(commands.len(), 1);

    // Check that dimmer command exists (values are u8, so always in valid range)
    let _dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
}
