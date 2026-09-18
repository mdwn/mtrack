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

//! The `move` effect end to end through the engine: focus-point aiming
//! through a placed fixture's mounting, pose memory across effects and
//! after arrival, the slew clamp, and a clear releasing the memory
//! (venue-exchange design §15.3–§15.4).

use std::collections::HashMap;
use std::time::Duration;

use super::super::effects::*;
use super::super::engine::EffectEngine;
use crate::lighting::types::{ChannelDef, MovementLimits, PhysicalRange, PhysicalUnit};

/// A 16-bit mover with a ±270° pan and ±135° tilt, hung at `position`
/// with `rotation`, patched at address 1: pan 1/2, tilt 3/4, dimmer 5.
fn mover(name: &str, position: [f64; 3], rotation: [f64; 3]) -> FixtureInfo {
    let channels: HashMap<String, u16> = [("pan", 1), ("tilt", 3), ("dimmer", 5)]
        .into_iter()
        .map(|(n, o)| (n.to_string(), o))
        .collect();
    let mut info = FixtureInfo::new(name.to_string(), 1, 1, "Mover".to_string(), channels, None);
    let mut defs = HashMap::new();
    defs.insert(
        "pan".to_string(),
        ChannelDef {
            offset: 1,
            fine: Some(2),
            range: Some(PhysicalRange {
                from: -270.0,
                to: 270.0,
                unit: PhysicalUnit::Degrees,
            }),
            functions: Vec::new(),
        },
    );
    defs.insert(
        "tilt".to_string(),
        ChannelDef {
            offset: 3,
            fine: Some(4),
            range: Some(PhysicalRange {
                from: -135.0,
                to: 135.0,
                unit: PhysicalUnit::Degrees,
            }),
            functions: Vec::new(),
        },
    );
    defs.insert("dimmer".to_string(), ChannelDef::at(5));
    info = info.with_channel_defs(defs);
    info.position = Some(position);
    info.rotation = Some(rotation);
    info
}

fn move_to(id: &str, to: MoveTarget, from: Option<MoveTarget>, secs: f64) -> EffectInstance {
    EffectInstance::new(
        id.to_string(),
        EffectType::Move {
            to,
            from,
            easing: Easing::Linear,
            duration: Duration::from_secs_f64(secs),
        },
        vec!["m".to_string()],
        None,
        None,
        None,
    )
}

/// The pan the engine emitted, in degrees, read back from the 16-bit bytes.
fn emitted_pan(commands: &[DmxCommand]) -> f64 {
    let coarse = commands
        .iter()
        .find(|c| c.channel == 1)
        .expect("pan coarse")
        .value;
    let fine = commands
        .iter()
        .find(|c| c.channel == 2)
        .expect("pan fine")
        .value;
    let value16 = u16::from_be_bytes([coarse, fine]) as f64;
    -270.0 + value16 / 65535.0 * 540.0
}

fn engine_with(fixture: FixtureInfo, focus: &[(&str, [f64; 3])]) -> EffectEngine {
    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture);
    engine.set_focus_points(focus.iter().map(|(n, p)| (n.to_string(), *p)).collect());
    engine
}

#[test]
fn a_move_to_a_focus_point_aims_through_the_mounting() {
    // Rear-truss mover facing the audience (rotation 0,0,180). A target
    // 3 m to stage-left at the same height is to its right: pan −90.
    let mut engine = engine_with(
        mover("m", [0.0, 3.5, 4.2], [0.0, 0.0, 180.0]),
        &[("left", [3.0, 3.5, 4.2])],
    );
    engine
        .start_effect(move_to("a", MoveTarget::Focus("left".into()), None, 1.0))
        .unwrap();
    // No pose memory yet: the first cue snaps to its target.
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    assert!(
        (emitted_pan(commands) - -90.0).abs() < 0.02,
        "{}",
        emitted_pan(commands)
    );
    assert!((engine.poses()["m"].pan - -90.0).abs() < 1e-9);
    assert!(engine.poses()["m"].tilt.abs() < 1e-9);
}

#[test]
fn a_move_travels_from_the_remembered_pose_and_holds_after_arrival() {
    let mut engine = engine_with(
        mover("m", [0.0, 0.0, 4.0], [0.0; 3]),
        &[("left", [4.0, 0.0, 4.0]), ("ahead", [0.0, 4.0, 4.0])],
    );
    // Snap to "left" (pan 90), then a 2 s linear move to "ahead" (pan 0).
    engine
        .start_effect(move_to("a", MoveTarget::Focus("left".into()), None, 0.5))
        .unwrap();
    engine.update(Duration::from_millis(0), None).unwrap();
    engine.update(Duration::from_millis(600), None).unwrap();
    engine
        .start_effect(move_to("b", MoveTarget::Focus("ahead".into()), None, 2.0))
        .unwrap();
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        (emitted_pan(commands) - 45.0).abs() < 0.05,
        "halfway from 90 to 0: {}",
        emitted_pan(commands)
    );
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        (emitted_pan(commands)).abs() < 0.05,
        "{}",
        emitted_pan(commands)
    );

    // The effect has run its duration; the pose holds with nothing driving it.
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        (emitted_pan(commands)).abs() < 0.05,
        "held after arrival: {}",
        emitted_pan(commands)
    );
    assert!((engine.poses()["m"].pan).abs() < 1e-9);
}

#[test]
fn an_explicit_from_overrides_the_memory_and_angles_move_one_axis() {
    let mut engine = engine_with(mover("m", [0.0, 0.0, 4.0], [0.0; 3]), &[]);
    engine
        .start_effect(move_to(
            "a",
            MoveTarget::Angles {
                pan: Some(0.0),
                tilt: Some(-30.0),
            },
            Some(MoveTarget::Angles {
                pan: Some(180.0),
                tilt: Some(-30.0),
            }),
            2.0,
        ))
        .unwrap();
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        (emitted_pan(commands) - 90.0).abs() < 0.05,
        "{}",
        emitted_pan(commands)
    );
    engine.update(Duration::from_millis(1000), None).unwrap();
    // A later move giving only tilt keeps the pan where it is.
    engine
        .start_effect(move_to(
            "b",
            MoveTarget::Angles {
                pan: None,
                tilt: Some(10.0),
            },
            None,
            0.0,
        ))
        .unwrap();
    engine.update(Duration::from_millis(10), None).unwrap();
    assert!(engine.poses()["m"].pan.abs() < 1e-9);
    assert!((engine.poses()["m"].tilt - 10.0).abs() < 1e-9);
}

#[test]
fn a_declared_slew_limit_clamps_the_travel() {
    let mut fixture = mover("m", [0.0, 0.0, 4.0], [0.0; 3]);
    fixture.movement = MovementLimits {
        max_pan_speed: Some(90.0),
        max_tilt_speed: None,
    };
    let mut engine = engine_with(fixture, &[]);
    // Sit at 0, then ask for 180° in a tenth of a second: the fixture can
    // only do 90°/s, so after 1 s it is at 90, after 2 s at 180.
    engine
        .start_effect(move_to(
            "a",
            MoveTarget::Angles {
                pan: Some(0.0),
                tilt: Some(0.0),
            },
            None,
            0.0,
        ))
        .unwrap();
    engine.update(Duration::from_millis(10), None).unwrap();
    engine
        .start_effect(move_to(
            "b",
            MoveTarget::Angles {
                pan: Some(180.0),
                tilt: Some(0.0),
            },
            None,
            0.1,
        ))
        .unwrap();
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        (emitted_pan(commands) - 90.0).abs() < 0.05,
        "{}",
        emitted_pan(commands)
    );
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        (emitted_pan(commands) - 180.0).abs() < 0.05,
        "{}",
        emitted_pan(commands)
    );
}

#[test]
fn a_clear_releases_pose_memory() {
    let mut engine = engine_with(mover("m", [0.0, 0.0, 4.0], [0.0; 3]), &[]);
    engine
        .start_effect(move_to(
            "a",
            MoveTarget::Angles {
                pan: Some(45.0),
                tilt: Some(0.0),
            },
            None,
            0.0,
        ))
        .unwrap();
    engine.update(Duration::from_millis(10), None).unwrap();
    assert!(engine.poses().contains_key("m"));
    engine.clear_all_layers();
    assert!(
        engine.poses().is_empty(),
        "a clear forgets where movers were"
    );
}

#[test]
fn an_unbound_focus_point_leaves_the_fixture_where_it_is() {
    let mut engine = engine_with(mover("m", [0.0, 0.0, 4.0], [0.0; 3]), &[]);
    engine
        .start_effect(move_to("a", MoveTarget::Focus("nowhere".into()), None, 1.0))
        .unwrap();
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(
        !commands.iter().any(|c| c.channel <= 4),
        "no pan/tilt written"
    );
    assert!(engine.poses().is_empty());
}
