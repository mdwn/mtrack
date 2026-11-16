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
//! A show's offset, as the player's timeline reads the tempo map through it.

use std::time::Duration;

use crate::lighting::parser::*;

/// #525: a `beats` speed in an offset show reads the tempo map shifted by
/// the offset, as the cue times are. The tempo changes at bar 5; with
/// `offset 1 measures` the music (and the `@5/1` cue) changes a bar later,
/// so a 1-beat chase must keep its 120 BPM rate through the bar the
/// unshifted map would already call 240.
#[test]
fn test_offset_shifts_the_tempo_map_tempo_aware_rates_read() {
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
        @5/1 { bpm: 240 }
    ]
}

show "Offset Chase" {
    @1/1
    offset 1 measures

    @2/1
    front_wash: chase, speed: 1beats, duration: 8measures

    @5/1
    front_wash: static, color: "red", duration: 1beat
}"#;

    let shows = parse_light_shows(content).expect("show parses");
    let show = shows.get("Offset Chase").unwrap();
    let tempo_map = show.tempo_map.as_ref().expect("tempo map");

    // At 120 BPM a bar is 2 s: bar 5 is 8 s in score time, 10 s with the
    // one-bar offset -- where the `@5/1` cue fires.
    let change_cue = show.cues.iter().find(|c| {
        c.effects.iter().any(|e| {
            matches!(
                e.effect_type,
                crate::lighting::effects::EffectType::Static { .. }
            )
        })
    });
    assert_eq!(
        change_cue.map(|c| c.time),
        Some(Duration::from_secs(10)),
        "the @5/1 cue should be shifted by the offset"
    );

    let chase = show
        .cues
        .iter()
        .flat_map(|c| c.effects.iter())
        .find(|e| {
            matches!(
                e.effect_type,
                crate::lighting::effects::EffectType::Chase { .. }
            )
        })
        .expect("chase");
    assert!(
        (chase.tempo_offset_secs - 2.0).abs() < 1e-9,
        "the chase should carry the show's offset, got {}",
        chase.tempo_offset_secs
    );
    let instance =
        crate::lighting::timeline::LightingTimeline::create_effect_instance(chase, Duration::ZERO);
    let crate::lighting::effects::EffectType::Chase { speed, .. } = &instance.effect_type else {
        unreachable!()
    };
    let rate = |secs: f64| {
        speed.to_rate_with_offset(
            Some(tempo_map),
            Duration::from_secs_f64(secs),
            instance.tempo_offset_secs,
        )
    };

    // Between the unshifted change (8 s) and the shifted one (10 s) the music
    // is still at 120: one beat is 0.5 s, so 2 cycles a second.
    assert!((rate(9.0) - 2.0).abs() < 1e-9, "at 9 s: {}", rate(9.0));
    // After the shifted change, 240: 4 cycles a second.
    assert!((rate(10.5) - 4.0).abs() < 1e-9, "at 10.5 s: {}", rate(10.5));
}
