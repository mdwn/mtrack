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

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::super::effects::{EffectInstance, EffectLayer};

/// Clear a layer - immediately stops all effects on the specified layer
pub(crate) fn clear_layer(
    active_effects: &mut HashMap<String, EffectInstance>,
    releasing_effects: &mut HashMap<String, (Duration, Instant)>,
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
    layer: EffectLayer,
) {
    // Remove matching effects from releasing_effects before removing from active_effects,
    // since we need active_effects to know which IDs are on this layer
    releasing_effects.retain(|id, _| {
        active_effects
            .get(id)
            .is_none_or(|effect| effect.layer != layer)
    });
    active_effects.retain(|_, effect| effect.layer != layer);
    frozen_layers.remove(&layer);
}

/// Clear all layers - immediately stops all effects on all layers
pub(crate) fn clear_all_layers(
    active_effects: &mut HashMap<String, EffectInstance>,
    releasing_effects: &mut HashMap<String, (Duration, Instant)>,
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
) {
    active_effects.clear();
    releasing_effects.clear();
    frozen_layers.clear();
}

/// Release a layer with a custom fade time
pub(crate) fn release_layer_with_time(
    active_effects: &mut HashMap<String, EffectInstance>,
    releasing_effects: &mut HashMap<String, (Duration, Instant)>,
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
    layer: EffectLayer,
    fade_time: Option<Duration>,
    current_time: Instant,
) {
    let default_fade = Duration::from_secs(1);

    for (effect_id, effect) in active_effects.iter() {
        if effect.layer == layer && !releasing_effects.contains_key(effect_id) {
            let release_time =
                fade_time.unwrap_or_else(|| effect.down_time.unwrap_or(default_fade));
            releasing_effects.insert(effect_id.clone(), (release_time, current_time));
        }
    }
    // Unfreeze the layer if it was frozen (properly adjusts effect start times
    // to maintain smooth animation continuity during the fade-out)
    unfreeze_layer(frozen_layers, active_effects, layer, current_time);
}

/// Freeze a layer - pauses all effects on the layer at their current state
pub(crate) fn freeze_layer(
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
    _active_effects: &mut HashMap<String, EffectInstance>,
    layer: EffectLayer,
    current_time: Instant,
) {
    // Record the time when the layer was frozen
    // Don't overwrite if already frozen
    frozen_layers.entry(layer).or_insert(current_time);
}

/// Unfreeze a layer - resumes effects on the layer from where they left off
pub(crate) fn unfreeze_layer(
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
    active_effects: &mut HashMap<String, EffectInstance>,
    layer: EffectLayer,
    current_time: Instant,
) {
    // When unfreezing, we need to adjust effect start times to account for frozen duration
    if let Some(frozen_at) = frozen_layers.remove(&layer) {
        let frozen_duration = current_time.duration_since(frozen_at);

        // Adjust start times for all effects on this layer
        for effect in active_effects.values_mut() {
            if effect.layer == layer {
                if let Some(start_time) = effect.start_time {
                    // Push the start time forward by the frozen duration
                    // This makes it appear as if no time passed while frozen
                    effect.start_time = Some(start_time + frozen_duration);
                }
            }
        }
    }
}

/// Set the intensity master for a layer (0.0 to 1.0)
pub(crate) fn set_layer_intensity_master(
    layer_intensity_masters: &mut HashMap<EffectLayer, f64>,
    layer: EffectLayer,
    intensity: f64,
) {
    let clamped = intensity.clamp(0.0, 1.0);
    if (clamped - 1.0).abs() < f64::EPSILON {
        // 1.0 is the default, remove from map to save memory
        layer_intensity_masters.remove(&layer);
    } else {
        layer_intensity_masters.insert(layer, clamped);
    }
}

/// Set the speed master for a layer (0.0+ where 1.0 = normal speed)
pub(crate) fn set_layer_speed_master(
    layer_speed_masters: &mut HashMap<EffectLayer, f64>,
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
    active_effects: &mut HashMap<String, EffectInstance>,
    layer: EffectLayer,
    speed: f64,
    current_time: Instant,
) {
    let clamped = speed.max(0.0); // Speed can be > 1.0 but not negative

    // Speed 0.0 means freeze - use the freeze_layer mechanism
    if clamped == 0.0 {
        freeze_layer(frozen_layers, active_effects, layer, current_time);
    } else {
        // Non-zero speed means unfreeze (if was frozen by speed=0)
        // Note: this only unfreezes if we're changing FROM 0.0
        let was_frozen_by_speed = layer_speed_masters.get(&layer) == Some(&0.0);
        if was_frozen_by_speed {
            unfreeze_layer(frozen_layers, active_effects, layer, current_time);
        }
    }

    if (clamped - 1.0).abs() < f64::EPSILON {
        // 1.0 is the default, remove from map to save memory
        layer_speed_masters.remove(&layer);
    } else {
        layer_speed_masters.insert(layer, clamped);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::super::super::effects::EffectType;
    use super::*;

    fn static_effect() -> EffectType {
        EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::from_secs(5),
        }
    }

    fn make_effect(id: &str, fixtures: Vec<&str>, layer: EffectLayer) -> EffectInstance {
        let mut inst = EffectInstance::new(
            id.to_string(),
            static_effect(),
            fixtures.into_iter().map(|s| s.to_string()).collect(),
            None,
            None,
            None,
        );
        inst.layer = layer;
        inst
    }

    // ── clear_layer ────────────────────────────────────────────────

    #[test]
    fn clear_layer_removes_matching() {
        let mut active = HashMap::new();
        active.insert(
            "a".to_string(),
            make_effect("a", vec!["f1"], EffectLayer::Background),
        );
        active.insert(
            "b".to_string(),
            make_effect("b", vec!["f1"], EffectLayer::Foreground),
        );
        let mut releasing = HashMap::new();
        let mut frozen = HashMap::new();
        clear_layer(
            &mut active,
            &mut releasing,
            &mut frozen,
            EffectLayer::Background,
        );
        assert!(!active.contains_key("a"));
        assert!(active.contains_key("b"));
    }

    // ── clear_all_layers ───────────────────────────────────────────

    #[test]
    fn clear_all_empties_everything() {
        let mut active = HashMap::new();
        active.insert(
            "a".to_string(),
            make_effect("a", vec!["f1"], EffectLayer::Background),
        );
        let mut releasing = HashMap::new();
        releasing.insert("a".to_string(), (Duration::from_secs(1), Instant::now()));
        let mut frozen = HashMap::new();
        frozen.insert(EffectLayer::Background, Instant::now());
        clear_all_layers(&mut active, &mut releasing, &mut frozen);
        assert!(active.is_empty());
        assert!(releasing.is_empty());
        assert!(frozen.is_empty());
    }

    // ── set_layer_intensity_master ─────────────────────────────────

    #[test]
    fn intensity_master_set_value() {
        let mut masters = HashMap::new();
        set_layer_intensity_master(&mut masters, EffectLayer::Background, 0.5);
        assert_eq!(masters.get(&EffectLayer::Background), Some(&0.5));
    }

    #[test]
    fn intensity_master_one_removes() {
        let mut masters = HashMap::new();
        masters.insert(EffectLayer::Background, 0.5);
        set_layer_intensity_master(&mut masters, EffectLayer::Background, 1.0);
        assert!(!masters.contains_key(&EffectLayer::Background));
    }

    #[test]
    fn intensity_master_clamps() {
        let mut masters = HashMap::new();
        set_layer_intensity_master(&mut masters, EffectLayer::Background, 1.5);
        // 1.5 clamps to 1.0, which is default, so removed
        assert!(!masters.contains_key(&EffectLayer::Background));
    }

    #[test]
    fn intensity_master_clamps_negative() {
        let mut masters = HashMap::new();
        set_layer_intensity_master(&mut masters, EffectLayer::Background, -0.5);
        assert_eq!(masters.get(&EffectLayer::Background), Some(&0.0));
    }

    // ── freeze/unfreeze ────────────────────────────────────────────

    #[test]
    fn freeze_records_time() {
        let mut frozen = HashMap::new();
        let mut active = HashMap::new();
        let now = Instant::now();
        freeze_layer(&mut frozen, &mut active, EffectLayer::Background, now);
        assert_eq!(frozen.get(&EffectLayer::Background), Some(&now));
    }

    #[test]
    fn freeze_does_not_overwrite() {
        let mut frozen = HashMap::new();
        let mut active = HashMap::new();
        let t1 = Instant::now();
        freeze_layer(&mut frozen, &mut active, EffectLayer::Background, t1);
        let t2 = Instant::now();
        freeze_layer(&mut frozen, &mut active, EffectLayer::Background, t2);
        assert_eq!(frozen.get(&EffectLayer::Background), Some(&t1));
    }

    #[test]
    fn unfreeze_adjusts_start_times() {
        let mut frozen = HashMap::new();
        let mut active = HashMap::new();
        let base = Instant::now();
        let mut effect = make_effect("a", vec!["f1"], EffectLayer::Background);
        effect.start_time = Some(base);
        active.insert("a".to_string(), effect);

        frozen.insert(EffectLayer::Background, base);
        // Simulate 2 seconds frozen
        let unfreeze_time = base + Duration::from_secs(2);
        unfreeze_layer(
            &mut frozen,
            &mut active,
            EffectLayer::Background,
            unfreeze_time,
        );

        assert!(!frozen.contains_key(&EffectLayer::Background));
        // Start time should be pushed forward by 2 seconds
        let new_start = active.get("a").unwrap().start_time.unwrap();
        assert_eq!(new_start, base + Duration::from_secs(2));
    }
}
