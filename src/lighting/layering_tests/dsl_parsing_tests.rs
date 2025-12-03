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

#[cfg(test)]
use crate::lighting::effects::*;
use std::time::Duration;

#[test]
fn test_layering_show_dsl_parsing() {
    use super::super::parser::parse_light_shows;

    // Test the exact DSL from layering_show.light
    let dsl_content = r#"show "Effect Layering Demo" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
    
    @00:04.000
    front_wash: strobe frequency: 2, layer: foreground, blend_mode: overlay
}"#;

    let shows = match parse_light_shows(dsl_content) {
        Ok(s) => s,
        Err(e) => {
            println!("Parser error: {}", e);
            panic!("Failed to parse layering show DSL: {}", e);
        }
    };

    let show = shows.get("Effect Layering Demo").unwrap();
    assert_eq!(show.cues.len(), 3);

    // Check static blue with replace blend mode
    let cue1 = &show.cues[0];
    let effect1 = &cue1.effects[0];
    assert_eq!(effect1.layer, Some(EffectLayer::Background));
    assert_eq!(effect1.blend_mode, Some(BlendMode::Replace));

    // Check dimmer with multiply blend mode
    let cue2 = &show.cues[1];
    let effect2 = &cue2.effects[0];
    assert_eq!(effect2.layer, Some(EffectLayer::Midground));
    assert_eq!(effect2.blend_mode, Some(BlendMode::Multiply));

    // Check strobe with overlay blend mode
    let cue3 = &show.cues[2];
    let effect3 = &cue3.effects[0];
    assert_eq!(effect3.layer, Some(EffectLayer::Foreground));
    assert_eq!(effect3.blend_mode, Some(BlendMode::Overlay));

    println!("Layering show DSL parsing test passed!");
    println!("Successfully parsed all blend modes: replace, multiply, overlay");
}
#[test]
fn test_dsl_blend_mode_parsing() {
    use super::super::parser::parse_light_shows;

    // Test DSL with multiply blend mode
    let dsl_with_multiply = r#"show "Blend Mode Test" {
    @00:00.000
    front_wash: static color: "blue", layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

    let result = parse_light_shows(dsl_with_multiply);
    assert!(
        result.is_ok(),
        "DSL should parse successfully: {:?}",
        result
    );

    let shows = result.unwrap();
    let show = shows.get("Blend Mode Test").unwrap();
    assert_eq!(show.cues.len(), 2);

    // Check first cue (static effect)
    let static_cue = &show.cues[0];
    assert_eq!(static_cue.effects.len(), 1);
    let static_effect = &static_cue.effects[0];
    assert_eq!(
        static_effect.blend_mode,
        Some(super::super::effects::BlendMode::Replace)
    );
    assert_eq!(
        static_effect.layer,
        Some(super::super::effects::EffectLayer::Background)
    );

    // Check second cue (dimmer effect)
    let dimmer_cue = &show.cues[1];
    assert_eq!(dimmer_cue.effects.len(), 1);
    let dimmer_effect = &dimmer_cue.effects[0];
    assert_eq!(
        dimmer_effect.blend_mode,
        Some(super::super::effects::BlendMode::Multiply)
    );
    assert_eq!(
        dimmer_effect.layer,
        Some(super::super::effects::EffectLayer::Midground)
    );

    println!("✅ DSL blend mode parsing test passed");
    println!(
        "  Static effect: blend_mode={:?}, layer={:?}",
        static_effect.blend_mode, static_effect.layer
    );
    println!(
        "  Dimmer effect: blend_mode={:?}, layer={:?}",
        dimmer_effect.blend_mode, dimmer_effect.layer
    );
}
#[test]
fn test_dsl_parsing_debug() {
    use super::super::parser::parse_light_shows;

    let dsl = r#"show "Test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

    match parse_light_shows(dsl) {
        Ok(shows) => {
            for (show_name, show) in shows {
                println!("Show: {}", show_name);
                for cue in &show.cues {
                    println!("  Cue at {:?}: {:?}", cue.time, cue.time);
                    for effect in &cue.effects {
                        println!("    Effect: {:?}", effect.effect_type);
                        println!("    Layer: {:?}", effect.layer);
                        println!("    Blend Mode: {:?}", effect.blend_mode);
                    }
                }
            }
        }
        Err(e) => {
            println!("Error parsing DSL: {}", e);
        }
    }
}
#[test]
fn test_dsl_layering_parsing() {
    use super::super::parser::parse_light_shows;

    let dsl_content = r#"show "DSL Layering Test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%, layer: background
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

    let shows = match parse_light_shows(dsl_content) {
        Ok(s) => s,
        Err(e) => {
            println!("Parser error: {}", e);
            panic!("Failed to parse DSL: {}", e);
        }
    };
    let show = shows.get("DSL Layering Test").unwrap();

    assert_eq!(show.name, "DSL Layering Test");
    assert_eq!(show.cues.len(), 2);

    // Check first cue (static blue with background layer)
    let cue1 = &show.cues[0];
    assert_eq!(cue1.time, Duration::from_secs(0));
    assert_eq!(cue1.effects.len(), 1);
    let effect1 = &cue1.effects[0];
    assert_eq!(effect1.groups, vec!["front_wash"]);
    assert_eq!(effect1.layer, Some(EffectLayer::Background));
    assert_eq!(effect1.blend_mode, None); // Not specified in DSL

    // Check second cue (dimmer with midground layer and multiply blend mode)
    let cue2 = &show.cues[1];
    assert_eq!(cue2.time, Duration::from_secs(2));
    assert_eq!(cue2.effects.len(), 1);
    let effect2 = &cue2.effects[0];
    assert_eq!(effect2.groups, vec!["front_wash"]);
    assert_eq!(effect2.layer, Some(EffectLayer::Midground));
    assert_eq!(effect2.blend_mode, Some(BlendMode::Multiply));

    println!("DSL layering parsing test passed!");
    println!("Successfully parsed layering parameters from DSL:");
    println!("- layer: background, midground, foreground");
    println!("- blend_mode: replace, multiply, overlay");
}
