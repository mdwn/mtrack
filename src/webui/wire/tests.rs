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

//! What the UI reads of each payload where serde's attributes decide it:
//! the tags, renames, flattened parts and fields left out.

use serde_json::json;

use super::messages::*;

#[test]
fn messages_are_tagged_by_type() {
    assert_eq!(
        json!(SocketMessage::ConfigChanged),
        json!({"type": "config_changed"})
    );
    assert_eq!(
        json!(SocketMessage::Reload(ShowReload::Error {
            error: "bad".to_string()
        })),
        json!({"type": "reload", "status": "error", "error": "bad"})
    );
    assert_eq!(
        json!(LoopState::Looping {
            section_name: "verse".to_string()
        }),
        json!({"state": "looping", "section_name": "verse"})
    );
}

#[test]
fn the_stage_frame_is_flat_in_the_state_message() {
    let state = json!(SocketMessage::State(StageState {
        frame: StageValues::default(),
        active_effects: vec![],
        under_test: vec![],
    }));
    for key in ["fixtures", "poses", "cells", "strobe_rates", "under_test"] {
        assert!(state.get(key).is_some(), "{key}: {state}");
    }
}

#[test]
fn next_up_leaves_out_what_its_case_does_not_decide() {
    assert_eq!(json!(NextUp::default()), json!({"song": null}));
    let next = json!(NextUp {
        breaks_loop: Some(false),
        fade_out: Some(None),
        auto_play_ms: Some(Some(2000)),
        ..Default::default()
    });
    assert_eq!(next["fade_out"], json!(null));
    assert_eq!(next["auto_play_ms"], json!(2000));
    assert!(next.get("ends_loop").is_none() && next.get("fade_in").is_none());
}

#[test]
fn fixture_metadata_names_its_type_type() {
    let fixture = json!(FixtureMetadata {
        tags: vec![],
        fixture_type: "par".to_string(),
        position: None,
        rotation: None,
        beam_angle: None,
        rig: None,
        capabilities: vec![],
        cells: vec![],
    });
    assert_eq!(fixture["type"], "par");
}

#[test]
fn a_fixture_test_warning_is_tagged_by_kind() {
    use super::fixture_test::TestWarning;
    assert_eq!(
        json!(TestWarning::UnsupportedControl {
            control: "pan".to_string(),
            message: "m".to_string()
        }),
        json!({"kind": "unsupported_control", "control": "pan", "message": "m"})
    );
}

#[test]
fn a_probe_leaves_out_what_its_case_does_not_have() {
    use super::devices::{AudioProbe, ProbeKind};
    let in_use = json!(AudioProbe {
        outcome: ProbeKind::InUse,
        ok: true,
        reason: None,
        callbacks: None,
        health: Some(None),
    });
    assert_eq!(in_use["outcome"], "in_use");
    assert!(in_use.get("reason").is_none() && in_use["health"].is_null());
}

#[test]
fn a_fixture_type_entry_leaves_out_what_its_answer_does_not_carry() {
    use super::fixture_types::FixtureTypeEntry;
    let entry = json!(FixtureTypeEntry {
        referential: true,
        footprint: Some(None),
        rich: false,
        fixture_type: crate::lighting::types::FixtureType::new(
            "par".to_string(),
            Default::default()
        ),
        file: None,
        extension: None,
        gdtf: None,
        dsl: None,
    });
    assert!(entry["footprint"].is_null() && entry["file"].is_null());
    assert!(entry.get("gdtf").is_none() && entry.get("dsl").is_none());
}

#[test]
fn a_refused_gdtf_mode_says_only_why() {
    use super::fixture_types::GdtfMode;
    let mode = json!(GdtfMode {
        name: "bad".to_string(),
        footprint: 1,
        capabilities: None,
        cells: None,
        strobe_range: None,
        channels: None,
        refused: Some("no".to_string()),
    });
    assert_eq!(
        mode,
        json!({"name": "bad", "footprint": 1, "refused": "no"})
    );
}
