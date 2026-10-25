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
//! Crossing two passes of a looping song's lights (`loop_crossfade`).

use crate::lighting::effects::{DmxCommand, FixtureInfo};
use crate::lighting::EffectEngine;
use std::collections::HashMap;

/// A mover at universe 1, address 1: dimmer 1, red 2, pan 3, strobe 4,
/// gobo 5.
fn engine() -> EffectEngine {
    let mut channels = HashMap::new();
    for (name, offset) in [
        ("dimmer", 1),
        ("red", 2),
        ("pan", 3),
        ("strobe", 4),
        ("gobo", 5),
    ] {
        channels.insert(name.to_string(), offset);
    }
    let mut engine = EffectEngine::new();
    engine.register_fixture(FixtureInfo::new(
        "mover".to_string(),
        1,
        1,
        "Generic".to_string(),
        channels,
        None,
    ));
    engine
}

fn frame(values: &[(u16, u8)]) -> Vec<DmxCommand> {
    values
        .iter()
        .map(|&(channel, value)| DmxCommand {
            universe: 1,
            channel,
            value,
        })
        .collect()
}

fn value(commands: &[DmxCommand], channel: u16) -> Option<u8> {
    commands
        .iter()
        .find(|c| c.universe == 1 && c.channel == channel)
        .map(|c| c.value)
}

#[test]
fn continuous_channels_fade_and_discrete_ones_switch_at_the_midpoint() {
    let engine = engine();
    let old = frame(&[(1, 200), (2, 255), (3, 0), (4, 100), (5, 30)]);
    let new = frame(&[(1, 100), (2, 0), (3, 200), (4, 0), (5, 60)]);

    let quarter = engine.crossfade_commands(&old, &new, 0.25);
    assert_eq!(value(&quarter, 1), Some(175), "dimmer fades");
    assert_eq!(value(&quarter, 2), Some(191), "red fades");
    assert_eq!(value(&quarter, 3), Some(50), "pan fades");
    assert_eq!(value(&quarter, 4), Some(100), "strobe holds the old rate");
    assert_eq!(value(&quarter, 5), Some(30), "gobo holds the old gobo");

    let three_quarters = engine.crossfade_commands(&old, &new, 0.75);
    assert_eq!(value(&three_quarters, 4), Some(0), "strobe takes the new");
    assert_eq!(value(&three_quarters, 5), Some(60), "gobo takes the new");

    // The ends are the frames themselves.
    let start = engine.crossfade_commands(&old, &new, 0.0);
    let end = engine.crossfade_commands(&old, &new, 1.0);
    for channel in 1..=5 {
        assert_eq!(value(&start, channel), value(&old, channel));
        assert_eq!(value(&end, channel), value(&new, channel));
    }
}

#[test]
fn a_channel_one_side_does_not_drive_fades_to_zero_but_a_position_holds() {
    let engine = engine();
    // The old pass lights the mover and points it; the new one has not
    // reached either yet.
    let old = frame(&[(1, 255), (3, 180)]);
    let new = frame(&[]);
    let half = engine.crossfade_commands(&old, &new, 0.5);
    assert_eq!(value(&half, 1), Some(128), "dimmer fades out");
    assert_eq!(value(&half, 3), Some(180), "pan stays where it was");

    // The other way: the new pass brings a light up from nothing.
    let half = engine.crossfade_commands(&new, &old, 0.5);
    assert_eq!(value(&half, 1), Some(128), "dimmer fades in");
    assert_eq!(value(&half, 3), Some(180), "pan is already there");
}

#[test]
fn a_sibling_drives_the_same_rig_from_a_clean_slate() {
    use crate::lighting::effects::EffectType;
    let mut engine = engine();
    let mut parameters = HashMap::new();
    parameters.insert("dimmer".to_string(), 1.0);
    engine
        .start_effect(crate::lighting::EffectInstance::new(
            "lit".to_string(),
            EffectType::Static {
                parameters,
                duration: std::time::Duration::from_secs(10),
            },
            vec!["mover".to_string()],
            None,
            None,
            None,
        ))
        .unwrap();
    let mut sibling = engine.sibling();
    assert!(engine.has_effect("lit"));
    assert_eq!(sibling.active_effects_count(), 0, "no effects carried over");

    // Same fixtures: the sibling knows the strobe is a strobe.
    let old = frame(&[(4, 100)]);
    let new = frame(&[(4, 0)]);
    assert_eq!(
        value(&sibling.crossfade_commands(&old, &new, 0.25), 4),
        Some(100)
    );
    // And its frame leaves the mover dark where the original lights it.
    let dt = std::time::Duration::from_millis(23);
    let lit = engine.update(dt, None).unwrap().to_vec();
    let dark = sibling.update(dt, None).unwrap().to_vec();
    assert_eq!(value(&lit, 1), Some(255));
    assert_ne!(value(&dark, 1), Some(255));
}
