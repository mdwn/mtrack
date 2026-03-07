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

// ── format_active_effects: all effect types ──────────────────────

#[test]
fn test_format_active_effects_all_effect_types() {
    // Tests all effect type name branches in format_active_effects
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("fix1", 1, 1);
    engine.register_fixture(fixture);

    // Static
    let static_effect = EffectInstance::new(
        "static_eff".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(static_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(output.contains("Static"), "Missing Static in: {}", output);
    engine.clear_layer(EffectLayer::Background);

    // ColorCycle
    let cc_effect = EffectInstance::new(
        "cc_eff".to_string(),
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 255, 0)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(cc_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(
        output.contains("ColorCycle"),
        "Missing ColorCycle in: {}",
        output
    );
    engine.clear_layer(EffectLayer::Background);

    // Strobe
    let strobe_effect = EffectInstance::new(
        "strobe_eff".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: None,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(strobe_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(output.contains("Strobe"), "Missing Strobe in: {}", output);
    engine.clear_layer(EffectLayer::Background);

    // Dimmer
    let dimmer_effect = EffectInstance::new(
        "dimmer_eff".to_string(),
        EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(dimmer_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(output.contains("Dimmer"), "Missing Dimmer in: {}", output);
    engine.clear_layer(EffectLayer::Background);

    // Chase
    let chase_effect = EffectInstance::new(
        "chase_eff".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(chase_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(output.contains("Chase"), "Missing Chase in: {}", output);
    engine.clear_layer(EffectLayer::Background);

    // Rainbow
    let rainbow_effect = EffectInstance::new(
        "rainbow_eff".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(rainbow_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(output.contains("Rainbow"), "Missing Rainbow in: {}", output);
    engine.clear_layer(EffectLayer::Background);

    // Pulse
    let pulse_effect = EffectInstance::new(
        "pulse_eff".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: None,
        },
        vec!["fix1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(pulse_effect).unwrap();
    let output = engine.format_active_effects();
    assert!(output.contains("Pulse"), "Missing Pulse in: {}", output);
}
