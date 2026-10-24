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

    // Should have commands for dimmer and red channels
    assert_eq!(commands.len(), 2);

    // Check dimmer command (50% = 127)
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 127);

    // Check red command (100% = 255)
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    assert_eq!(red_cmd.value, 255);
}

/// An RGB-only fixture (no dimmer), like a PixelBrick in RGBS mode.
fn rgb_fixture(name: &str) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    FixtureInfo::new(name.to_string(), 1, 1, "RGB".to_string(), channels, None)
}

fn color_static(
    id: &str,
    (r, g, b): (f64, f64, f64),
    layer: EffectLayer,
    blend_mode: BlendMode,
    fades: Option<(Duration, Duration, Duration)>,
) -> EffectInstance {
    let mut parameters = HashMap::new();
    parameters.insert("red".to_string(), r);
    parameters.insert("green".to_string(), g);
    parameters.insert("blue".to_string(), b);
    let (up, hold, down) = match fades {
        Some((up, hold, down)) => (Some(up), Some(hold), Some(down)),
        None => (None, None, None),
    };
    let mut effect = EffectInstance::new(
        id.to_string(),
        EffectType::Static {
            parameters,
            duration: Duration::from_secs(10),
        },
        vec!["brick".to_string()],
        up,
        hold,
        down,
    );
    effect.layer = layer;
    effect.blend_mode = blend_mode;
    effect
}

fn rgb_at(engine: &mut EffectEngine, dt: Duration) -> (u8, u8, u8) {
    let commands = engine.update(dt, None).unwrap();
    let get = |ch: u16| {
        commands
            .iter()
            .find(|c| c.channel == ch)
            .map(|c| c.value)
            .unwrap_or(0)
    };
    (get(1), get(2), get(3))
}

#[test]
fn a_replace_static_fades_from_the_bed_not_from_black() {
    // #521: a yellow lift over a blue bed, 1s up, 1s hold, 1s down. Halfway
    // up it is halfway between blue and yellow — never darker than either.
    let mut engine = EffectEngine::new();
    engine.register_fixture(rgb_fixture("brick"));
    engine
        .start_effect(color_static(
            "bed",
            (0.0, 0.0, 1.0),
            EffectLayer::Background,
            BlendMode::Replace,
            None,
        ))
        .unwrap();
    let second = Duration::from_secs(1);
    engine
        .start_effect(color_static(
            "lift",
            (1.0, 1.0, 0.0),
            EffectLayer::Midground,
            BlendMode::Replace,
            Some((second, second, second)),
        ))
        .unwrap();

    let near = |got: u8, want: u8| (got as i32 - want as i32).abs() <= 3;

    // Half way up: half bed, half lift.
    let (r, g, b) = rgb_at(&mut engine, Duration::from_millis(500));
    assert!(
        near(r, 127) && near(g, 127) && near(b, 127),
        "{:?}",
        (r, g, b)
    );

    // Holding: the lift.
    let (r, g, b) = rgb_at(&mut engine, Duration::from_millis(1000));
    assert_eq!((r, g, b), (255, 255, 0));

    // Half way down: back toward the bed, not toward black.
    let (r, g, b) = rgb_at(&mut engine, Duration::from_millis(1000));
    assert!(
        near(r, 127) && near(g, 127) && near(b, 127),
        "{:?}",
        (r, g, b)
    );
}

#[test]
fn a_replace_static_over_nothing_still_fades_from_black() {
    let mut engine = EffectEngine::new();
    engine.register_fixture(rgb_fixture("brick"));
    let second = Duration::from_secs(1);
    engine
        .start_effect(color_static(
            "lift",
            (1.0, 1.0, 0.0),
            EffectLayer::Midground,
            BlendMode::Replace,
            Some((second, second, second)),
        ))
        .unwrap();
    let (r, g, b) = rgb_at(&mut engine, Duration::from_millis(500));
    assert!(
        (r as i32 - 127).abs() <= 3 && (g as i32 - 127).abs() <= 3,
        "{:?}",
        (r, g, b)
    );
    assert_eq!(b, 0);
}
