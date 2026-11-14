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

//! The brightness model (design 3.4), end to end through the engine: the
//! same show on a fixture with a dimmer channel and on one without gives
//! the same light. Layers A, B and C are background, midground and
//! foreground.

use crate::lighting::effects::*;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

use EffectLayer::{Background as A, Foreground as C, Midground as B};

const RED: [(&str, f64); 3] = [("red", 1.0), ("green", 0.0), ("blue", 0.0)];
const BLUE: [(&str, f64); 3] = [("red", 0.0), ("green", 0.0), ("blue", 1.0)];
const WHITE: [(&str, f64); 3] = [("red", 1.0), ("green", 1.0), ("blue", 1.0)];

/// What a fixture was sent: DMX 1 to 4 for the dimmer fixture (dimmer, red,
/// green, blue), 10 to 12 for the RGB-only one.
struct Frame(HashMap<u16, u8>);

impl Frame {
    /// The dimmer-channel fixture: (dimmer, red, green, blue).
    fn dimmer(&self) -> (u8, [u8; 3]) {
        (self.0[&1], [self.0[&2], self.0[&3], self.0[&4]])
    }

    /// The RGB-only fixture.
    fn rgb(&self) -> [u8; 3] {
        [self.0[&10], self.0[&11], self.0[&12]]
    }
}

fn engine() -> EffectEngine {
    let mut engine = EffectEngine::new();
    let channels = |names: &[&str]| -> HashMap<String, u16> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i as u16 + 1))
            .collect()
    };
    engine.register_fixture(FixtureInfo::new(
        "dim".to_string(),
        1,
        1,
        "RGBD".to_string(),
        channels(&["dimmer", "red", "green", "blue"]),
        None,
    ));
    engine.register_fixture(FixtureInfo::new(
        "rgb".to_string(),
        1,
        10,
        "RGB".to_string(),
        channels(&["red", "green", "blue"]),
        None,
    ));
    engine
}

fn static_look(
    id: &str,
    layer: EffectLayer,
    blend: BlendMode,
    params: &[(&str, f64)],
) -> EffectInstance {
    let mut effect = EffectInstance::new(
        id.to_string(),
        EffectType::Static {
            parameters: params.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
            duration: Duration::from_secs(10),
        },
        vec!["dim".to_string(), "rgb".to_string()],
        None,
        None,
        None,
    );
    effect.layer = layer;
    effect.blend_mode = blend;
    effect
}

fn level(id: &str, layer: EffectLayer, blend: BlendMode, v: f64) -> EffectInstance {
    static_look(id, layer, blend, &[("dimmer", v)])
}

fn show(effects: Vec<EffectInstance>) -> Frame {
    let mut engine = engine();
    for effect in effects {
        engine.start_effect(effect).unwrap();
    }
    frame(&mut engine)
}

fn frame(engine: &mut EffectEngine) -> Frame {
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    Frame(commands.iter().map(|c| (c.channel, c.value)).collect())
}

use BlendMode::{Add, Multiply, Replace};

// ── The worked example (design 3.4) ─────────────────────────────────

/// A red, A dimmer 50%; C multiply white: 50% red, as A alone.
#[test]
fn a_multiply_white_above_a_dimmed_look_changes_nothing() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        level("ad", A, Replace, 0.5),
        static_look("c", C, Multiply, &WHITE),
    ]);
    assert_eq!(f.dimmer(), (128, [255, 0, 0]));
    assert_eq!(f.rgb(), [128, 0, 0]);
}

/// A red, A dimmer 50%; C replace blue: full blue.
#[test]
fn a_foreground_colour_replace_is_full_not_dimmed_by_the_background() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        level("ad", A, Replace, 0.5),
        static_look("c", C, Replace, &BLUE),
    ]);
    assert_eq!(f.dimmer(), (255, [0, 0, 255]));
    assert_eq!(f.rgb(), [0, 0, 255]);
}

/// A red, A dimmer 50%; B replace blue: full blue.
#[test]
fn a_midground_colour_replace_is_full_not_dimmed_by_the_background() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        level("ad", A, Replace, 0.5),
        static_look("b", B, Replace, &BLUE),
    ]);
    assert_eq!(f.dimmer(), (255, [0, 0, 255]));
    assert_eq!(f.rgb(), [0, 0, 255]);
}

/// A red; C replace blue, C dimmer 50%: 50% blue.
#[test]
fn a_layers_own_dimmer_dims_its_own_colour() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        static_look("c", C, Replace, &BLUE),
        level("cd", C, Replace, 0.5),
    ]);
    assert_eq!(f.dimmer(), (128, [0, 0, 255]));
    assert_eq!(f.rgb(), [0, 0, 128]);
}

// ── Around the table ────────────────────────────────────────────────

/// Colour a higher layer adds is not dimmed by the dimmer beneath it; the
/// lower colour still is.
#[test]
fn an_added_colour_is_not_dimmed_by_a_lower_dimmer() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        level("ad", A, Replace, 0.5),
        static_look("c", C, Add, &BLUE),
    ]);
    assert_eq!(f.dimmer(), (255, [128, 0, 255]));
    assert_eq!(f.rgb(), [128, 0, 255]);
}

/// A dimmer replace above a lower dimmer sets the level, on both kinds.
#[test]
fn a_dimmer_replace_above_a_dimmer_sets_the_level() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        level("ad", A, Replace, 0.25),
        level("bd", B, Replace, 0.75),
    ]);
    assert_eq!(f.dimmer(), (191, [255, 0, 0]));
    assert_eq!(f.rgb(), [191, 0, 0]);
}

/// A dimmer multiply above a lower dimmer scales it.
#[test]
fn a_dimmer_multiply_above_a_dimmer_scales_it() {
    let f = show(vec![
        static_look("a", A, Replace, &RED),
        level("ad", A, Replace, 0.5),
        level("bd", B, Multiply, 0.5),
    ]);
    assert_eq!(f.dimmer(), (64, [255, 0, 0]));
    assert_eq!(f.rgb(), [64, 0, 0]);
}

/// A colour crossfading in over a dimmed look moves smoothly from the
/// dimmed light to its own: no jump in brightness at either end.
#[test]
fn a_colour_replace_fading_in_has_no_brightness_jump() {
    let mut engine = engine();
    engine
        .start_effect(static_look("a", A, Replace, &RED))
        .unwrap();
    engine.start_effect(level("ad", A, Replace, 0.5)).unwrap();
    let mut fade = static_look("c", C, Replace, &BLUE);
    fade.up_time = Some(Duration::from_secs(1));
    engine.start_effect(fade).unwrap();
    let mut previous = frame(&mut engine).rgb();
    let mut last = previous;
    for _ in 0..20 {
        let now = engine.update(Duration::from_millis(50), None).unwrap();
        let now = Frame(now.iter().map(|c| (c.channel, c.value)).collect()).rgb();
        for i in 0..3 {
            assert!(
                (i16::from(now[i]) - i16::from(previous[i])).abs() <= 20,
                "{previous:?} -> {now:?}"
            );
        }
        previous = now;
        last = now;
    }
    assert_eq!(last, [0, 0, 255], "the fade ends at full blue");
}

/// Colour with nothing dimming it is full brightness on a dimmer fixture.
#[test]
fn colour_alone_opens_the_dimmer() {
    let f = show(vec![static_look("a", A, Replace, &RED)]);
    assert_eq!(f.dimmer(), (255, [255, 0, 0]));
    assert_eq!(f.rgb(), [255, 0, 0]);
}

/// A level with no colour beneath it is white at that level.
#[test]
fn a_level_with_no_colour_is_white() {
    let f = show(vec![level("a", A, Replace, 0.5)]);
    assert_eq!(f.dimmer(), (128, [255, 255, 255]));
    assert_eq!(f.rgb(), [128, 128, 128]);
}

fn strobe(layer: EffectLayer) -> EffectInstance {
    let mut effect = EffectInstance::new(
        "strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::from_secs(10),
        },
        vec!["dim".to_string(), "rgb".to_string()],
        None,
        None,
        None,
    );
    effect.layer = layer;
    effect
}

/// A strobe with no colour beneath it flashes white, on both kinds.
#[test]
fn a_strobe_over_no_colour_flashes_white() {
    let mut engine = engine();
    engine.start_effect(strobe(C)).unwrap();
    let on = frame(&mut engine);
    assert_eq!(on.dimmer(), (255, [255, 255, 255]));
    assert_eq!(on.rgb(), [255, 255, 255]);
    let off = Frame(
        engine
            .update(Duration::from_millis(600), None)
            .unwrap()
            .iter()
            .map(|c| (c.channel, c.value))
            .collect(),
    );
    assert_eq!(off.dimmer().0, 0);
    assert_eq!(off.rgb(), [0, 0, 0]);
}

/// A strobe over red flashes red, also on a fixture with no dimmer or
/// strobe channel.
#[test]
fn a_strobe_over_red_flashes_red() {
    let mut engine = engine();
    engine
        .start_effect(static_look("a", A, Replace, &RED))
        .unwrap();
    engine.start_effect(strobe(C)).unwrap();
    let on = frame(&mut engine);
    assert_eq!(on.dimmer(), (255, [255, 0, 0]));
    assert_eq!(on.rgb(), [255, 0, 0]);
    let off = Frame(
        engine
            .update(Duration::from_millis(600), None)
            .unwrap()
            .iter()
            .map(|c| (c.channel, c.value))
            .collect(),
    );
    assert_eq!(off.dimmer().0, 0);
    assert_eq!(off.rgb(), [0, 0, 0]);
}

/// A show on one layer, `static color: red, dimmer: 50%`, gives on a
/// dimmer fixture the bytes it always did.
#[test]
fn a_one_layer_look_with_its_dimmer_is_what_it_was() {
    let f = show(vec![static_look(
        "a",
        A,
        Replace,
        &[("red", 1.0), ("green", 0.0), ("blue", 0.0), ("dimmer", 0.5)],
    )]);
    assert_eq!(f.dimmer(), (128, [255, 0, 0]));
    assert_eq!(f.rgb(), [128, 0, 0]);
}

/// A fixture that is only a dimmer blends its levels as its channel did.
#[test]
fn a_plain_dimmer_fixture_is_unchanged() {
    let mut engine = EffectEngine::new();
    engine.register_fixture(FixtureInfo::new(
        "lamp".to_string(),
        1,
        1,
        "Dimmer".to_string(),
        HashMap::from([("dimmer".to_string(), 1)]),
        None,
    ));
    let mut start = |id: &str, layer, blend, v| {
        let mut effect = level(id, layer, blend, v);
        effect.target_fixtures = vec!["lamp".to_string()];
        engine.start_effect(effect).unwrap();
    };
    start("a", A, Replace, 0.4);
    start("b", B, Add, 0.2);
    start("c", C, Multiply, 0.5);
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 1, "only its dimmer is written");
    // (0.4 + 0.2) * 0.5 = 0.3
    assert_eq!(commands[0].value, 77);
}

/// A pixel bar with a master dimmer and cells that are only colour: the
/// dimmer is the brightest cell's level and every cell's light is exact.
#[test]
fn a_pixel_bar_with_a_master_dimmer_keeps_every_cells_light_exact() {
    use crate::lighting::types::{Cell, ChannelDef};
    let cell = |name: &str, base: u16| Cell {
        name: name.to_string(),
        channels: ["red", "green", "blue"]
            .iter()
            .enumerate()
            .map(|(i, c)| (c.to_string(), ChannelDef::at(base + i as u16)))
            .collect(),
        offset: [0.0; 3],
    };
    let mut bar = FixtureInfo::new(
        "bar".to_string(),
        1,
        1,
        "Bar".to_string(),
        // The fixture lists its first cell's channels too, as a type's do.
        HashMap::from([
            ("dimmer".to_string(), 1),
            ("red".to_string(), 2),
            ("green".to_string(), 3),
            ("blue".to_string(), 4),
        ]),
        None,
    );
    bar.cells = vec![cell("1", 2), cell("2", 5)];
    let mut engine = EffectEngine::new();
    engine.register_fixture(bar);

    let mut start = |effect: EffectInstance| engine.start_effect(effect).unwrap();
    let mut look = static_look("red", A, Replace, &RED);
    look.target_fixtures = vec!["bar".to_string()];
    start(look);
    // Cell 1 is dimmed to a quarter, cell 2 to half.
    for (cell, v) in [("1", 0.25), ("2", 0.5)] {
        let mut dim = level(&format!("dim{cell}"), B, Replace, v);
        dim.target_fixtures = vec![format!("bar/{cell}")];
        start(dim);
    }
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let byte = |channel: u16| {
        commands
            .iter()
            .find(|c| c.channel == channel)
            .unwrap()
            .value
    };
    // The dimmer is the highest peak, half; cell 2 is red at full against it,
    // cell 1 at half of it: a quarter of the light overall.
    assert_eq!(byte(1), 128, "master dimmer");
    assert_eq!(byte(5), 255, "cell 2 red");
    assert_eq!(byte(2), 128, "cell 1 red");
}
