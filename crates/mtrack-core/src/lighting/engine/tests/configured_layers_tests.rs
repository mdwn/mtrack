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

//! A player configured with more (or other) layers than the default three:
//! shows parse against the set, each layer draws over those below it, and
//! the layer commands take the configured names.

use std::time::Duration;

use crate::lighting::effects::{EffectLayer, LayerSet};
use crate::lighting::engine::tests::common::{create_test_fixture, on_channel};
use crate::lighting::engine::EffectEngine;
use crate::lighting::parser::{parse_light_shows_in, LightShow};
use crate::lighting::timeline::LightingTimeline;

fn five() -> LayerSet {
    LayerSet::new(
        ["floor", "back", "wash", "accents", "blinders"]
            .map(String::from)
            .to_vec(),
    )
    .unwrap()
}

/// The show, parsed against `layers`.
fn show(layers: &LayerSet, body: &str) -> LightShow {
    parse_light_shows_in(layers, &format!("show \"s\" {{\n{body}\n}}\n"), None)
        .unwrap()
        .shift_remove("s")
        .unwrap()
}

fn engine_with(layers: &LayerSet, show: &LightShow, up_to: Duration) -> EffectEngine {
    let mut engine = EffectEngine::with_layers(layers.clone());
    engine.register_fixture(create_test_fixture("all", 1, 1));
    for cue in show.cues.iter().filter(|c| c.time <= up_to) {
        for effect in &cue.effects {
            engine
                .start_effect(LightingTimeline::create_effect_instance(effect, cue.time))
                .unwrap();
        }
        for command in &cue.layer_commands {
            engine.apply_layer_command(command);
        }
    }
    engine
}

const FIVE_LAYERS: &str = r#"
    @00:00.000
    all: static color: "blue", duration: 10s, layer: floor
    all: static color: "green", duration: 10s, layer: accents
    all: static color: "red", duration: 10s, layer: blinders
    @00:05.000
    clear(layer: blinders)
"#;

#[test]
fn a_show_names_a_fifth_layer_and_it_parses_to_its_position() {
    let layers = five();
    let show = show(&layers, FIVE_LAYERS);
    let on: Vec<_> = show.cues[0].effects.iter().map(|e| e.layer).collect();
    assert_eq!(
        on,
        [
            Some(EffectLayer(0)),
            Some(EffectLayer(3)),
            Some(EffectLayer(4))
        ]
    );
    assert_eq!(show.cues[1].layer_commands[0].layer, Some(EffectLayer(4)));
}

#[test]
fn each_layer_draws_over_the_ones_below_it() {
    let layers = five();
    let show = show(&layers, FIVE_LAYERS);
    let mut engine = engine_with(&layers, &show, Duration::ZERO);
    let commands = engine.update(Duration::from_millis(10), None).unwrap();
    // Red, on the top layer, covers the green and blue beneath it.
    assert_eq!(on_channel(commands, 2).value, 255, "red");
    assert_eq!(on_channel(commands, 3).value, 0, "green");
    assert_eq!(on_channel(commands, 4).value, 0, "blue");
}

#[test]
fn a_clear_on_a_configured_layer_uncovers_the_layer_below() {
    let layers = five();
    let show = show(&layers, FIVE_LAYERS);
    let mut engine = engine_with(&layers, &show, Duration::from_secs(5));
    let commands = engine.update(Duration::from_millis(10), None).unwrap();
    // The blinders cleared, the accents' green shows.
    assert_eq!(on_channel(commands, 2).value, 0, "red");
    assert_eq!(on_channel(commands, 3).value, 255, "green");
    assert_eq!(on_channel(commands, 4).value, 0, "blue");
}

#[test]
fn freeze_unfreeze_and_master_take_a_configured_layer() {
    let layers = five();
    let show = show(
        &layers,
        "@00:00.000\nall: static color: \"red\", duration: 10s, layer: accents\n\
         @00:01.000\nfreeze(layer: accents)\n\
         @00:02.000\nunfreeze(layer: accents)\n\
         @00:03.000\nmaster(layer: accents, intensity: 50%)\n",
    );
    let commands: Vec<_> = show
        .cues
        .iter()
        .flat_map(|c| c.layer_commands.iter())
        .collect();
    assert_eq!(commands.len(), 3);
    assert!(commands.iter().all(|c| c.layer == Some(EffectLayer(3))));
    let mut engine = engine_with(&layers, &show, Duration::from_secs(1));
    assert!(engine.is_layer_frozen(EffectLayer(3)));
    engine.apply_layer_command(commands[1]);
    assert!(!engine.is_layer_frozen(EffectLayer(3)));
}

#[test]
fn the_engine_names_its_layers_from_the_set() {
    let layers = five();
    let engine = EffectEngine::with_layers(layers.clone());
    assert_eq!(engine.layers(), &layers);
    assert_eq!(
        engine.layers().display_name(EffectLayer(4)),
        "blinders".to_string()
    );
    assert_eq!(EffectEngine::new().layers(), LayerSet::standard());
}
