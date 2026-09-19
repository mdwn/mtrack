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

//! Per-cell control (design §17.3): a pixel fixture's cells as
//! sub-fixtures, `per: cell` expansion, the layered merge of a cell's
//! state over its fixture's, and the wire bytes that come out.

use std::collections::HashMap;
use std::time::Duration;

use crate::lighting::effects::*;
use crate::lighting::engine::EffectEngine;
use crate::lighting::types::{Cell, ChannelDef};

/// A three-cell RGB bar at address 1: dimmer 1, cells at 2-4, 5-7, 8-10,
/// the fixture-level colour ganged across them.
fn bar(name: &str, position: Option<[f64; 3]>) -> FixtureInfo {
    let cell = |n: &str, base: u16, x: f64| Cell {
        name: n.to_string(),
        channels: HashMap::from([
            ("red".to_string(), ChannelDef::at(base)),
            ("green".to_string(), ChannelDef::at(base + 1)),
            ("blue".to_string(), ChannelDef::at(base + 2)),
        ]),
        offset: [x, 0.0, 0.0],
    };
    let cells = vec![cell("1", 2, -0.3), cell("2", 5, 0.0), cell("3", 8, 0.3)];
    let ganged = crate::lighting::types::ganged_from_cells(&cells).unwrap();
    let mut channels: HashMap<String, u16> =
        ganged.iter().map(|(n, d)| (n.clone(), d.offset)).collect();
    channels.insert("dimmer".to_string(), 1);
    let mut defs = ganged;
    defs.insert("dimmer".to_string(), ChannelDef::at(1));
    let mut info = FixtureInfo::new(name.to_string(), 1, 1, "Bar".to_string(), channels, None);
    info.channel_defs = defs;
    info.cells = cells;
    info.position = position;
    info.rotation = Some([0.0, 0.0, 0.0]);
    info
}

fn byte(commands: &[DmxCommand], channel: u16) -> u8 {
    commands
        .iter()
        .find(|c| c.channel == channel)
        .map(|c| c.value)
        .unwrap_or(0)
}

fn static_red(targets: Vec<String>, per_cell: bool) -> EffectInstance {
    let mut effect = EffectInstance::new(
        "bed".to_string(),
        EffectType::Static {
            parameters: HashMap::from([
                ("red".to_string(), 1.0),
                ("green".to_string(), 0.0),
                ("blue".to_string(), 0.0),
                ("dimmer".to_string(), 1.0),
            ]),
            duration: Duration::from_secs(10),
        },
        targets,
        None,
        None,
        None,
    );
    effect.per_cell = per_cell;
    effect
}

#[test]
fn cells_register_as_sub_fixtures_placed_by_their_offsets() {
    let mut engine = EffectEngine::new();
    engine.register_fixture(bar("Bar", Some([1.0, 2.0, 3.0])));
    let registry = engine.get_fixture_registry();
    assert!(registry.contains_key("Bar"));
    let sub = registry.get("Bar/3").expect("sub-fixture");
    assert_eq!(sub.parent.as_deref(), Some("Bar"));
    assert_eq!(sub.address, 1);
    assert_eq!(sub.channels["red"], 8);
    assert!(!sub.channels.contains_key("dimmer"));
    let p = sub.position.unwrap();
    assert!((p[0] - 1.3).abs() < 1e-9 && (p[1] - 2.0).abs() < 1e-9);
    assert!(sub.cells.is_empty(), "a cell has no cells");
}

#[test]
fn a_show_that_never_says_per_cell_writes_the_ganged_bytes() {
    let mut engine = EffectEngine::new();
    engine.register_fixture(bar("Bar", None));
    engine
        .start_effect(static_red(vec!["Bar".to_string()], false))
        .unwrap();
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    for red in [2, 5, 8] {
        assert_eq!(byte(commands, red), 255, "red on every cell");
    }
    for off in [3, 4, 6, 7, 9, 10] {
        assert_eq!(byte(commands, off), 0);
    }
    assert_eq!(byte(commands, 1), 255, "dimmer");
    // Every byte written exactly once.
    let mut channels: Vec<u16> = commands.iter().map(|c| c.channel).collect();
    channels.sort();
    channels.dedup();
    assert_eq!(channels.len(), commands.len(), "{commands:?}");
}

#[test]
fn a_per_cell_chase_runs_along_the_cells_over_the_bed() {
    let mut engine = EffectEngine::new();
    engine.register_fixture(bar("Bar", Some([0.0, 3.0, 4.0])));
    engine
        .start_effect(static_red(vec!["Bar".to_string()], false))
        .unwrap();
    let mut chase = EffectInstance::new(
        "chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::from_secs(10),
        },
        vec!["Bar".to_string()],
        None,
        None,
        None,
    );
    chase.per_cell = true;
    chase.layer = EffectLayer::Midground;
    engine.start_effect(chase).unwrap();
    assert_eq!(
        engine.get_active_effects()["chase"].target_fixtures,
        vec!["Bar/1", "Bar/2", "Bar/3"],
        "per: cell expands the bar to its cells"
    );

    // One cell lit at a time, the others dark, walking left to right —
    // and the dimmer, which no cell owns, still from the bed.
    let lit_at = |engine: &mut EffectEngine, ms: u64| -> Vec<u16> {
        let commands = engine.update(Duration::from_millis(ms), None).unwrap();
        assert_eq!(byte(commands, 1), 255);
        [2u16, 5, 8]
            .into_iter()
            .filter(|red| byte(commands, *red) == 255)
            .collect()
    };
    // One cell per step (update advances by the delta), and over a cycle
    // every cell has its turn. (Which
    // end the walk starts from is the direction convention's business;
    // the plot-level chase tests own that.)
    let mut seen = Vec::new();
    for ms in [50, 350, 350] {
        let lit = lit_at(&mut engine, ms);
        assert_eq!(lit.len(), 1, "at {ms} ms: {lit:?}");
        seen.push(lit[0]);
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen, vec![2, 5, 8], "every cell lit once across the cycle");
}

#[test]
fn per_cell_on_a_plain_fixture_leaves_it_whole() {
    let mut engine = EffectEngine::new();
    let plain = crate::lighting::engine::tests::common::create_test_fixture("Par", 1, 20);
    engine.register_fixture(plain);
    engine.register_fixture(bar("Bar", None));
    engine
        .start_effect(static_red(vec!["Par".to_string(), "Bar".to_string()], true))
        .unwrap();
    let targets = &engine.get_active_effects()["bed"].target_fixtures;
    assert_eq!(targets, &vec!["Par", "Bar/1", "Bar/2", "Bar/3"]);
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    assert_eq!(byte(commands, 2), 255);
    assert_eq!(byte(commands, 8), 255);
}

#[test]
fn spread_paints_a_rainbow_along_the_targets() {
    let mut engine = EffectEngine::new();
    engine.register_fixture(bar("Bar", Some([0.0, 3.0, 4.0])));
    let mut rainbow = EffectInstance::new(
        "rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(0.0),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::from_secs(10),
        },
        vec!["Bar".to_string()],
        None,
        None,
        None,
    );
    rainbow.per_cell = true;
    rainbow.spread = 360.0;
    engine.start_effect(rainbow).unwrap();
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    // Hues 0, 120, 240 across the three cells: red, green, blue.
    assert_eq!(
        (byte(commands, 2), byte(commands, 3), byte(commands, 4)),
        (255, 0, 0)
    );
    assert_eq!(
        (byte(commands, 5), byte(commands, 6), byte(commands, 7)),
        (0, 255, 0)
    );
    assert_eq!(
        (byte(commands, 8), byte(commands, 9), byte(commands, 10)),
        (0, 0, 255)
    );
}

/// `spread` on a `cycle` works `per: fixture` too (design §17.3): the
/// group's targets in spatial order each get their share of the spread,
/// which at `spread: 360deg` over three targets and three colours means
/// every colour of the cycle appears once across the group, not one
/// colour on all three.
#[test]
fn cycle_spread_paints_the_palette_across_placed_fixtures() {
    let mut engine = EffectEngine::new();
    let placed = |name: &str, address: u16, x: f64| {
        let mut fixture =
            crate::lighting::engine::tests::common::create_test_fixture(name, 1, address);
        fixture.position = Some([x, 0.0, 0.0]);
        fixture
    };
    engine.register_fixture(placed("F1", 1, -1.0));
    engine.register_fixture(placed("F2", 10, 0.0));
    engine.register_fixture(placed("F3", 19, 1.0));

    let colors = vec![
        Color::new(255, 0, 0),
        Color::new(0, 255, 0),
        Color::new(0, 0, 255),
    ];
    let mut cycle = EffectInstance::new(
        "cycle".to_string(),
        EffectType::ColorCycle {
            colors,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::from_secs(10),
        },
        vec!["F1".to_string(), "F2".to_string(), "F3".to_string()],
        None,
        None,
        None,
    );
    cycle.spread = 360.0;
    engine.start_effect(cycle).unwrap();

    // Not per-cell, so target_fixtures stays the group list — spread alone
    // is what spatially spaces the three fixtures across the palette.
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    // F1 (address 1): red = ch2, green = ch3, blue = ch4.
    assert_eq!(
        (byte(commands, 2), byte(commands, 3), byte(commands, 4)),
        (255, 0, 0),
        "F1 red"
    );
    // F2 (address 10): red = ch11, green = ch12, blue = ch13.
    assert_eq!(
        (byte(commands, 11), byte(commands, 12), byte(commands, 13)),
        (0, 255, 0),
        "F2 green"
    );
    // F3 (address 19): red = ch20, green = ch21, blue = ch22.
    assert_eq!(
        (byte(commands, 20), byte(commands, 21), byte(commands, 22)),
        (0, 0, 255),
        "F3 blue"
    );
}
