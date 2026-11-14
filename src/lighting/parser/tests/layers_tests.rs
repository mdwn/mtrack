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

//! `layer:` names a layer of the player's configured set: the grammar takes
//! any identifier, and the parser checks it against the set.

use crate::lighting::effects::{EffectLayer, LayerSet};
use crate::lighting::parser::document::document;
use crate::lighting::parser::writer::write;
use crate::lighting::parser::{parse_light_shows, parse_light_shows_in};

fn set(names: &[&str]) -> LayerSet {
    LayerSet::new(names.iter().map(|n| n.to_string()).collect()).unwrap()
}

fn show_with(line: &str) -> String {
    format!("show \"s\" {{\n    @00:00.000\n    {line}\n}}\n")
}

#[test]
fn an_effect_names_any_layer_of_the_set() {
    let layers = set(&["floor", "back", "accents"]);
    let source = show_with("all: static color: \"red\", duration: 1s, layer: accents");
    let shows = parse_light_shows_in(&layers, &source, None).unwrap();
    assert_eq!(
        shows["s"].cues[0].effects[0].layer,
        Some(EffectLayer(2)),
        "the layer is its position in the set"
    );
}

#[test]
fn a_layer_name_may_begin_with_a_keyword_or_hold_dashes() {
    // `add` and `snap` are blend modes and transitions; a layer named for
    // them is read whole.
    let layers = set(&["addons", "snap-lights", "screen2"]);
    for (name, index) in [("addons", 0), ("snap-lights", 1), ("screen2", 2)] {
        let source = show_with(&format!(
            "all: static color: \"red\", duration: 1s, layer: {name}"
        ));
        let shows = parse_light_shows_in(&layers, &source, None).unwrap();
        assert_eq!(
            shows["s"].cues[0].effects[0].layer,
            Some(EffectLayer(index)),
            "{name}"
        );
    }
}

#[test]
fn a_layer_last_in_the_parameters_or_followed_by_more_parses() {
    let layers = set(&["lo", "hi"]);
    let source = show_with("all: static color: \"red\", layer: hi, duration: 1s, blend_mode: add");
    let effect = &parse_light_shows_in(&layers, &source, None).unwrap()["s"].cues[0].effects[0];
    assert_eq!(effect.layer, Some(EffectLayer(1)));
    assert_eq!(
        effect.blend_mode,
        Some(crate::lighting::effects::BlendMode::Add)
    );
}

#[test]
fn an_unknown_layer_fails_the_load_with_its_name_position_and_the_players_layers() {
    let source = show_with("all: static color: \"red\", duration: 1s, layer: accents");
    let error = parse_light_shows(&source).unwrap_err().to_string();
    assert!(error.contains("unknown layer 'accents'"), "{error}");
    assert!(error.contains("line 3"), "{error}");
    assert!(error.contains("column"), "{error}");
    assert!(
        error.contains("background, midground, foreground"),
        "{error}"
    );
}

#[test]
fn an_unknown_layer_in_a_layer_command_is_refused_the_same_way() {
    for command in ["clear", "freeze", "unfreeze"] {
        let source = show_with(&format!("{command}(layer: blinders)"));
        let error = parse_light_shows(&source).unwrap_err().to_string();
        assert!(error.contains("unknown layer 'blinders'"), "{error}");
        assert!(error.contains("line 3"), "{error}");
        assert!(error.contains("midground"), "{error}");
    }
    let master = parse_light_shows(&show_with("master(layer: blinders, intensity: 50%)"))
        .unwrap_err()
        .to_string();
    assert!(master.contains("unknown layer 'blinders'"), "{master}");
}

#[test]
fn an_unknown_layer_inside_a_sequence_is_refused_where_it_is_defined() {
    let source = "sequence \"s\" {\n    @00:00.000\n    all: static color: \"red\", duration: 1s, layer: accents\n}\n\
                  show \"x\" {\n    @00:00.000\n    sequence \"s\"\n}\n";
    let error = parse_light_shows(source).unwrap_err().to_string();
    assert!(error.contains("unknown layer 'accents'"), "{error}");
    assert!(
        parse_light_shows_in(&set(&["a", "accents"]), source, None).is_ok(),
        "the set that has it takes it"
    );
}

#[test]
fn a_layer_is_not_a_quoted_string_or_a_number() {
    let layers = set(&["a"]);
    for value in ["\"a\"", "1", "#ffffff"] {
        let source = show_with(&format!(
            "all: static color: \"red\", duration: 1s, layer: {value}"
        ));
        assert!(
            parse_light_shows_in(&layers, &source, None).is_err(),
            "layer: {value}"
        );
    }
}

#[test]
fn the_writer_keeps_a_layer_by_its_name_in_the_set_edited_against() {
    let source = "show \"s\" {\n    @00:00.000\n    all: static color: \"red\", duration: 1s, layer: accents\n    freeze(layer: blinders)\n}\n";
    let result = document(source, None);
    let doc = result.document.expect("a document, whatever the layers");
    // The document holds names; written back, they are the names.
    assert_eq!(write(&doc), source);
    // And what was written parses against the set that has them.
    let layers = set(&["floor", "accents", "blinders"]);
    let shows = parse_light_shows_in(&layers, &write(&doc), None).unwrap();
    assert_eq!(shows["s"].cues[0].effects[0].layer, Some(EffectLayer(1)));
    assert_eq!(
        shows["s"].cues[0].layer_commands[0].layer,
        Some(EffectLayer(2))
    );
}

#[test]
fn a_new_effect_and_the_vocabulary_name_the_sets_layers() {
    let layers = set(&["floor", "ceiling"]);
    let vocabulary = crate::lighting::parser::edit::vocabulary(&layers);
    assert_eq!(vocabulary.layers, ["floor", "ceiling"]);
    let effect = crate::lighting::parser::edit::new_effect(Some("ceiling"), false);
    let layer = effect
        .effect
        .params
        .iter()
        .find(|p| p.name == "layer")
        .map(|p| p.value.as_str());
    assert_eq!(layer, Some("ceiling"));
}
