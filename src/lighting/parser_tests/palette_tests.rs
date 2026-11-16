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
//! Palettes, as the player's timeline resolves them.

use std::collections::HashMap;

use crate::lighting::effects::{Color, EffectType};
use crate::lighting::parser::*;

const TEMPO: &str = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}
"#;

const PALETTE: &str = r##"
palette {
    primary: "#C01020"
    secondary: "orange"
    accent: rgb(255, 240, 200)
}
"##;

fn parse(body: &str) -> Result<LightShows, String> {
    parse_light_shows(&format!("{TEMPO}{body}")).map_err(|e| e.to_string())
}

fn rgb(effect: &Effect) -> (u8, u8, u8) {
    match &effect.effect_type {
        EffectType::Static { parameters, .. } => (
            (parameters["red"] * 255.0).round() as u8,
            (parameters["green"] * 255.0).round() as u8,
            (parameters["blue"] * 255.0).round() as u8,
        ),
        other => panic!("expected a static, got {other:?}"),
    }
}

const BLUE: Color = Color {
    r: 0x20,
    g: 0x40,
    b: 0xFF,
    w: None,
};

fn overrides(pairs: &[(&str, Color)]) -> HashMap<String, Color> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

const TWO_USES: &str = r#"
show "S" {
    @1/1
    a: static, color: palette.primary, duration: 1beat
    b: cycle, color: palette.primary, color: palette.secondary, speed: 1.0, duration: 1beat
}
"#;

fn parsed(body: &str) -> Vec<LightShow> {
    parse(&format!("{PALETTE}{body}"))
        .unwrap()
        .into_values()
        .collect()
}

#[test]
fn the_timeline_resolves_with_overrides_and_without() {
    use crate::lighting::timeline::LightingTimeline;
    let first_static = |timeline: &LightingTimeline| {
        let (_, effects) = timeline.cue_effects().next().expect("a cue");
        rgb(&effects[0])
    };
    let plain = LightingTimeline::new(parsed(TWO_USES));
    let blue = LightingTimeline::with_palette(parsed(TWO_USES), &overrides(&[("primary", BLUE)]));
    assert_eq!(first_static(&plain), (0xC0, 0x10, 0x20));
    assert_eq!(first_static(&blue), (0x20, 0x40, 0xFF));
}
