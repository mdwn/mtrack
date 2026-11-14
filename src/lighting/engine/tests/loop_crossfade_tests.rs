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
/// gobo 5, green 6, blue 7.
fn engine() -> EffectEngine {
    let mut channels = HashMap::new();
    for (name, offset) in [
        ("dimmer", 1),
        ("red", 2),
        ("pan", 3),
        ("strobe", 4),
        ("gobo", 5),
        ("green", 6),
        ("blue", 7),
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
    // The incoming look is a level with no colour, so it shows white
    // (design 3.4, rule 7): red is driven on both sides, at full.
    assert!(
        (level(&quarter, "red").unwrap() - 1.0).abs() < 1e-9,
        "red holds, the incoming white driving it too"
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
        .map(|(name, info)| (name.clone(), info.has_dimmer()))
        .collect();
    let mut bytes = HashMap::new();
    for snapshot in crate::state::fixture_snapshots_with_cells(
        states,
        &has_dimmer,
        registry,
        // The states alone: what the view would show of them.
        &Default::default(),
        crate::state::StrobeRates::Live,
    ) {
        let info = &registry[&snapshot.name];
        for (channel, &value) in &snapshot.channels {
            if let Some(&offset) = info.channels.get(channel) {
                bytes.insert(info.dmx_channel(offset).unwrap(), value);
            }
        }
        for (cell, values) in &snapshot.cells {
            let def = info.cells.iter().find(|c| &c.name == cell).unwrap();
            for (channel, &value) in values {
                bytes.insert(
                    info.dmx_channel(def.channels[channel].offset).unwrap(),
                    value,
                );
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
    half.layer = EffectLayer::MIDGROUND;
    half.blend_mode = BlendMode::Multiply;
    incoming.start_effect(half).unwrap();

    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        view_matches_dmx(&mut outgoing, &mut incoming, t);
    }
}

/// A head at universe 1, address 1, with a 16-bit dimmer (1, fine 2), red
/// 3, a 16-bit pan (4, fine 5) and a gobo 6.
fn sixteen_bit_engine() -> EffectEngine {
    use crate::lighting::types::ChannelDef;
    let sixteen = |offset: u16| ChannelDef {
        fine: Some(offset + 1),
        ..ChannelDef::at(offset)
    };
    let defs = HashMap::from([
        ("dimmer".to_string(), sixteen(1)),
        ("red".to_string(), ChannelDef::at(3)),
        ("pan".to_string(), sixteen(4)),
        ("gobo".to_string(), ChannelDef::at(6)),
    ]);
    let channels = defs.iter().map(|(n, d)| (n.clone(), d.offset)).collect();
    let info = FixtureInfo::new(
        "mover".to_string(),
        1,
        1,
        "Head".to_string(),
        channels,
        None,
    )
    .with_channel_defs(defs);
    let mut engine = EffectEngine::new();
    engine.register_fixture(info);
    engine
}

/// The 16-bit value at `coarse` and the channel after it.
fn value16(commands: &[DmxCommand], coarse: u16) -> Option<u16> {
    Some(u16::from_be_bytes([
        value(commands, coarse)?,
        value(commands, coarse + 1)?,
    ]))
}

#[test]
fn a_sixteen_bit_channel_crosses_as_one_value() {
    let engine = sixteen_bit_engine();
    // Pan 0x00FF to 0x0100, one count, across the fine byte's rollover.
    let old = frame(&[(4, 0x00), (5, 0xFF), (6, 10)]);
    let new = frame(&[(4, 0x01), (5, 0x00), (6, 90)]);
    let mut last = 0x00FF;
    for step in 0..=1000 {
        let t = f64::from(step) / 1000.0;
        let pan = value16(&engine.crossfade_commands(&old, &new, t), 4).unwrap();
        assert!(
            (last..=0x0100).contains(&pan),
            "pan steps back or overshoots at t {t}: {pan:#06x} after {last:#06x}"
        );
        last = pan;
    }
    let at = |t| engine.crossfade_commands(&old, &new, t);
    assert_eq!(value16(&at(0.25), 4), Some(0x00FF));
    assert_eq!(value16(&at(0.5), 4), Some(0x0100));
    // A setting still switches at the midpoint.
    assert_eq!(value(&at(0.49), 6), Some(10));
    assert_eq!(value(&at(0.5), 6), Some(90));

    // A head only one side points holds there, both bytes of it; a 16-bit
    // dimmer only one side drives fades from full as one value.
    let lit = frame(&[(1, 0xFF), (2, 0xFF), (4, 0x12), (5, 0x34)]);
    let quarter = engine.crossfade_commands(&lit, &[], 0.25);
    assert_eq!(value16(&quarter, 4), Some(0x1234));
    assert_eq!(value16(&quarter, 1), Some(49151));
    let quarter = engine.crossfade_commands(&[], &lit, 0.25);
    assert_eq!(value16(&quarter, 4), Some(0x1234));
    assert_eq!(value16(&quarter, 1), Some(16384));
}

#[test]
fn the_dmx_of_a_crossfade_is_the_states_blend() {
    let mut outgoing = sixteen_bit_engine();
    // Levels exactly on the 16-bit scale, so each side's bytes are its
    // state's.
    let full = f64::from(u16::MAX);
    lit(
        &mut outgoing,
        &[("dimmer", 1.0), ("pan", 255.0 / full), ("gobo", 0.2)],
    );
    let mut incoming = outgoing.sibling();
    lit(
        &mut incoming,
        &[
            ("dimmer", 25535.0 / full),
            ("pan", 256.0 / full),
            ("gobo", 0.6),
        ],
    );
    let info = incoming.get_fixture_registry()["mover"].clone();
    let sorted = |mut commands: Vec<DmxCommand>| {
        commands.sort_by_key(|c| c.channel);
        commands
            .into_iter()
            .map(|c| (c.channel, c.value))
            .collect::<Vec<_>>()
    };
    for t in [0.0, 0.1, 0.25, 0.3, 0.6, 0.75, 0.9, 1.0] {
        let sent =
            incoming.crossfade_commands(outgoing.last_commands(), incoming.last_commands(), t);
        let (states, _) = incoming.crossfade_states(&outgoing, t);
        assert_eq!(
            sorted(sent),
            sorted(states["mover"].to_dmx_commands(&info)),
            "at t {t}"
        );
    }
}

/// A fixture `name` at universe 1, `address`, with these channels, each
/// 16-bit where it names a fine offset.
fn patched(name: &str, address: u16, channels: &[(&str, u16, Option<u16>)]) -> FixtureInfo {
    use crate::lighting::types::ChannelDef;
    let defs: HashMap<String, ChannelDef> = channels
        .iter()
        .map(|&(channel, offset, fine)| {
            (
                channel.to_string(),
                ChannelDef {
                    fine,
                    ..ChannelDef::at(offset)
                },
            )
        })
        .collect();
    let offsets = defs.iter().map(|(n, d)| (n.clone(), d.offset)).collect();
    FixtureInfo::new(
        name.to_string(),
        1,
        address,
        "Head".to_string(),
        offsets,
        None,
    )
    .with_channel_defs(defs)
}

/// Where patches overlap, every channel either side drives still goes
/// out, and the frame is the same whatever order the rig's maps hold the
/// fixtures and channels in.
#[test]
fn overlapping_patches_cross_the_same_every_time_and_drop_nothing() {
    let pan16 = [("pan", 1, Some(2))];
    let rigs: [Vec<FixtureInfo>; 3] = [
        // An 8-bit dimmer over the pan's coarse byte.
        vec![
            patched("mover", 1, &pan16),
            patched("par", 1, &[("dimmer", 1, None)]),
        ],
        // ...and over its fine byte.
        vec![
            patched("mover", 1, &pan16),
            patched("par", 2, &[("dimmer", 1, None)]),
        ],
        // A second head's 16-bit tilt over the pan's fine byte.
        vec![
            patched("mover", 1, &pan16),
            patched("other", 2, &[("tilt", 1, Some(2))]),
        ],
    ];
    let old = frame(&[(1, 0x00), (2, 0xFF), (3, 0x40)]);
    let new = frame(&[(1, 0x01), (2, 0x00), (3, 0x80)]);
    for rig in rigs {
        let mut first: Option<Vec<Vec<(u16, u8)>>> = None;
        for _ in 0..32 {
            let mut engine = EffectEngine::new();
            for fixture in &rig {
                engine.register_fixture(fixture.clone());
            }
            let frames: Vec<Vec<(u16, u8)>> = [0.0, 0.25, 0.5, 0.75, 1.0]
                .iter()
                .map(|&t| {
                    let mut out: Vec<(u16, u8)> = engine
                        .crossfade_commands(&old, &new, t)
                        .into_iter()
                        .map(|c| (c.channel, c.value))
                        .collect();
                    out.sort();
                    out
                })
                .collect();
            for out in &frames {
                let channels: Vec<u16> = out.iter().map(|&(c, _)| c).collect();
                assert_eq!(channels, [1, 2, 3], "every channel goes out");
            }
            assert_eq!(frames[0], [(1, 0x00), (2, 0xFF), (3, 0x40)]);
            assert_eq!(frames[4], [(1, 0x01), (2, 0x00), (3, 0x80)]);
            match &first {
                Some(first) => assert_eq!(&frames, first, "the same every time"),
                None => first = Some(frames),
            }
        }
    }
}

/// Two channels of one fixture patched over one offset (a v1 `channels:
/// { dimmer: 1, strobe: 1 }`, which nothing refuses) cross by one rule,
/// whatever order the fixture's maps hold them in: the fade.
#[test]
fn an_overlap_inside_one_fixture_crosses_the_same_every_time() {
    let channels = HashMap::from([("dimmer".to_string(), 1), ("strobe".to_string(), 1)]);
    let old = frame(&[(1, 0)]);
    let new = frame(&[(1, 200)]);
    for _ in 0..32 {
        let mut engine = EffectEngine::new();
        engine.register_fixture(FixtureInfo::new(
            "par".to_string(),
            1,
            1,
            "Par".to_string(),
            channels.clone(),
            None,
        ));
        assert_eq!(
            value(&engine.crossfade_commands(&old, &new, 0.25), 1),
            Some(50)
        );
    }
}
