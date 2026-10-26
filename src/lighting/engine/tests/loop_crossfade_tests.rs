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

/// `mover` lit by a `static` setting `levels`, a frame rendered.
fn lit(engine: &mut EffectEngine, levels: &[(&str, f64)]) {
    use crate::lighting::effects::{EffectInstance, EffectType};
    let parameters = levels
        .iter()
        .map(|&(name, level)| (name.to_string(), level))
        .collect();
    engine
        .start_effect(EffectInstance::new(
            "look".to_string(),
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
    engine
        .update(std::time::Duration::from_millis(25), None)
        .unwrap();
}

#[test]
fn the_states_shown_cross_by_the_same_rules_as_the_dmx() {
    // What the web UI and get_fixture_state show mid-crossfade (#555).
    let mut outgoing = engine();
    lit(
        &mut outgoing,
        &[("dimmer", 0.8), ("red", 1.0), ("gobo", 0.2)],
    );
    let mut incoming = outgoing.sibling();
    lit(&mut incoming, &[("dimmer", 0.4), ("gobo", 0.6)]);

    let level = |states: &HashMap<String, crate::lighting::effects::FixtureState>, name: &str| {
        states["mover"].channels.get(name).map(|c| c.value)
    };
    let (quarter, _) = incoming.crossfade_states(&outgoing, 0.25);
    assert!(
        (level(&quarter, "dimmer").unwrap() - 0.7).abs() < 1e-9,
        "dimmer fades"
    );
    assert!(
        (level(&quarter, "red").unwrap() - 0.75).abs() < 1e-9,
        "red fades out, only the outgoing side driving it"
    );
    assert_eq!(level(&quarter, "gobo"), Some(0.2), "gobo holds the old");
    let (three_quarters, _) = incoming.crossfade_states(&outgoing, 0.75);
    assert_eq!(
        level(&three_quarters, "gobo"),
        Some(0.6),
        "gobo takes the new"
    );
}

/// The bytes the view would show for `states` -- the web UI's snapshot of
/// them -- by DMX channel, for a fixture at universe 1 address 1.
fn shown_bytes(
    engine: &EffectEngine,
    states: &HashMap<String, crate::lighting::effects::FixtureState>,
) -> HashMap<u16, u8> {
    let registry = engine.get_fixture_registry();
    let has_dimmer: HashMap<String, bool> = registry
        .iter()
        .map(|(name, info)| (name.clone(), info.channels.contains_key("dimmer")))
        .collect();
    let mut bytes = HashMap::new();
    for snapshot in crate::state::fixture_snapshots_with_cells(states, &has_dimmer, registry) {
        let info = &registry[&snapshot.name];
        for (channel, &value) in &snapshot.channels {
            if let Some(&offset) = info.channels.get(channel) {
                bytes.insert(info.address + offset - 1, value);
            }
        }
        for (cell, values) in &snapshot.cells {
            let def = info.cells.iter().find(|c| &c.name == cell).unwrap();
            for (channel, &value) in values {
                bytes.insert(info.address + def.channels[channel].offset - 1, value);
            }
        }
    }
    bytes
}

/// Asserts the view of `incoming` crossing from `outgoing` at `t` shows the
/// bytes the DMX blend sends, within rounding.
fn view_matches_dmx(outgoing: &mut EffectEngine, incoming: &mut EffectEngine, t: f64) {
    let dt = std::time::Duration::from_millis(25);
    let old = outgoing.update(dt, None).unwrap().to_vec();
    let new = incoming.update(dt, None).unwrap().to_vec();
    let sent = incoming.crossfade_commands(&old, &new, t);
    let (states, _) = incoming.crossfade_states(outgoing, t);
    let shown = shown_bytes(incoming, &states);
    for command in &sent {
        let view = shown.get(&command.channel).copied().unwrap_or(0);
        assert!(
            (view as i16 - command.value as i16).abs() <= 1,
            "channel {} at t {t}: view {view}, sent {}",
            command.channel,
            command.value
        );
    }
}

#[test]
fn the_view_of_a_crossfade_is_what_goes_out() {
    use crate::lighting::effects::{BlendMode, EffectInstance, EffectLayer, EffectType};
    use crate::lighting::types::{Cell, ChannelDef};
    // A three-cell RGB bar with no dimmer, so colour carries the layer
    // multipliers, at address 1.
    let cell = |n: &str, base: u16| Cell {
        name: n.to_string(),
        channels: HashMap::from([
            ("red".to_string(), ChannelDef::at(base)),
            ("green".to_string(), ChannelDef::at(base + 1)),
            ("blue".to_string(), ChannelDef::at(base + 2)),
        ]),
        offset: [0.0, 0.0, 0.0],
    };
    let cells = vec![cell("1", 1), cell("2", 4), cell("3", 7)];
    let defs = crate::lighting::types::ganged_from_cells(&cells).unwrap();
    let channels = defs.iter().map(|(n, d)| (n.clone(), d.offset)).collect();
    let mut info = FixtureInfo::new("bar".to_string(), 1, 1, "Bar".to_string(), channels, None);
    info.channel_defs = defs;
    info.cells = cells;
    let mut outgoing = EffectEngine::new();
    outgoing.register_fixture(info);
    let mut incoming = outgoing.sibling();

    let effect = |id: &str, parameters: &[(&str, f64)], per_cell: bool| {
        let mut effect = EffectInstance::new(
            id.to_string(),
            EffectType::Static {
                parameters: parameters
                    .iter()
                    .map(|&(n, v)| (n.to_string(), v))
                    .collect(),
                duration: std::time::Duration::from_secs(10),
            },
            vec!["bar".to_string()],
            None,
            None,
            None,
        );
        effect.per_cell = per_cell;
        effect
    };
    // Outgoing: the whole bar red. Incoming: each cell blue, under a
    // dimmer multiplied to half on the mid layer -- per-cell state and a
    // multiplier only one side has.
    outgoing
        .start_effect(effect("red", &[("red", 1.0)], false))
        .unwrap();
    incoming
        .start_effect(effect("blue", &[("blue", 1.0)], true))
        .unwrap();
    let mut half = effect("half", &[("dimmer", 0.5)], false);
    half.layer = EffectLayer::Midground;
    half.blend_mode = BlendMode::Multiply;
    incoming.start_effect(half).unwrap();

    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        view_matches_dmx(&mut outgoing, &mut incoming, t);
    }
}
