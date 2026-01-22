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

use super::super::effects::*;
use std::collections::HashMap;
use std::time::Duration;

pub fn create_test_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4); // Add strobe channel

    FixtureInfo {
        name: name.to_string(),
        universe,
        address,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    }
}

// Helper function to create EffectInstance with layering
pub fn create_effect_with_layering(
    id: String,
    effect_type: EffectType,
    target_fixtures: Vec<String>,
    layer: EffectLayer,
    blend_mode: BlendMode,
) -> EffectInstance {
    let mut effect = EffectInstance::new(id, effect_type, target_fixtures, None, None, None);
    effect.layer = layer;
    effect.blend_mode = blend_mode;
    // Ensure effects persist long enough for tests
    if effect.hold_time.is_none() {
        effect.hold_time = Some(Duration::from_secs(10));
    }
    effect
}

// Helper function to create EffectInstance with timing
pub fn create_effect_with_timing(
    id: String,
    effect_type: EffectType,
    target_fixtures: Vec<String>,
    layer: EffectLayer,
    blend_mode: BlendMode,
    up_time: Option<Duration>,
    down_time: Option<Duration>,
) -> EffectInstance {
    let mut effect =
        EffectInstance::new(id, effect_type, target_fixtures, up_time, None, down_time);
    effect.layer = layer;
    effect.blend_mode = blend_mode;
    effect.up_time = up_time;
    effect.down_time = down_time;
    effect
}
