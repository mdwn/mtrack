// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use crate::lighting::effects::{ChasePattern, EffectType};
use crate::lighting::parser::*;

#[test]
fn test_parse_chase_random_pattern() {
    let content = r#"tempo {
    start: 1.5s
    bpm: 160
    time_signature: 4/4
    changes: [
    @68/1 {
    bpm: 180
    }
    ]
}

show "Test" {
    @70/1
    all_wash: chase, speed: 1beats, pattern: random, hold_time: 4measures
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse show: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    assert_eq!(show.cues.len(), 1);
    let cue = &show.cues[0];
    assert_eq!(cue.effects.len(), 1);

    let effect = &cue.effects[0];
    match &effect.effect_type {
        EffectType::Chase {
            pattern,
            speed: _,
            direction: _,
            transition: _,
        } => {
            println!("Parsed pattern: {:?}", pattern);
            println!("Pattern debug: pattern={:?}", pattern);
            match pattern {
                ChasePattern::Random => {
                    println!("✓ Pattern is correctly set to Random");
                }
                ChasePattern::Linear => {
                    println!("✗ Pattern is Linear, but should be Random!");
                    panic!("Pattern was parsed as Linear instead of Random");
                }
                ChasePattern::Snake => {
                    println!("✗ Pattern is Snake, but should be Random!");
                    panic!("Pattern was parsed as Snake instead of Random");
                }
            }
        }
        other => {
            println!("Effect type: {:?}", other);
            panic!("Effect is not a Chase effect! Got: {:?}", other);
        }
    }
}
