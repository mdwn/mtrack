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

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::super::effects::{BlendMode, EffectInstance, EffectLayer, EffectType};

/// Stop effects that conflict with the new effect
pub(crate) fn stop_conflicting_effects(
    active_effects: &mut HashMap<String, EffectInstance>,
    new_effect: &EffectInstance,
    fixture_registry: &HashMap<String, super::super::effects::FixtureInfo>,
) {
    let mut to_remove = Vec::new();

    for (effect_id, effect) in active_effects.iter() {
        // Skip if effect is already disabled
        if !effect.enabled {
            continue;
        }

        // Check if effects should conflict based on sophisticated rules
        if should_effects_conflict(effect, new_effect, fixture_registry) {
            to_remove.push(effect_id.clone());
        }
    }

    for effect_id in to_remove {
        active_effects.remove(&effect_id);
    }
}

/// Determine if two effects should conflict based on sophisticated rules
fn should_effects_conflict(
    existing: &EffectInstance,
    new: &EffectInstance,
    _fixture_registry: &HashMap<String, super::super::effects::FixtureInfo>,
) -> bool {
    // 1. Layer-based conflict resolution
    // Effects in different layers generally don't conflict unless they have channel conflicts
    if existing.layer != new.layer {
        return have_channel_conflicts(existing, new);
    }

    // 2. Priority-based conflict resolution within the same layer
    if existing.priority < new.priority {
        return have_fixture_overlap(existing, new);
    }

    // 3. Effect type specific conflict rules
    effects_conflict_by_type(existing, new)
}

/// Check if effects have overlapping target fixtures
fn have_fixture_overlap(existing: &EffectInstance, new: &EffectInstance) -> bool {
    existing
        .target_fixtures
        .iter()
        .any(|fixture| new.target_fixtures.contains(fixture))
}

/// Check if effects have channel-level conflicts
fn have_channel_conflicts(_existing: &EffectInstance, _new: &EffectInstance) -> bool {
    // Effects in different layers should generally not conflict
    // The layering system is designed to allow effects in different layers
    // to coexist and blend together
    false
}

/// Determine conflicts based on effect types and blend modes
fn effects_conflict_by_type(existing: &EffectInstance, new: &EffectInstance) -> bool {
    use EffectType::{Chase, ColorCycle, Dimmer, Pulse, Rainbow, Static, Strobe};

    // If effects don't overlap fixtures, they don't conflict
    if !have_fixture_overlap(existing, new) {
        return false;
    }

    // Check blend mode compatibility
    if blend_modes_are_compatible(existing.blend_mode, new.blend_mode) {
        return false;
    }

    // Effect type specific conflict rules
    match (&existing.effect_type, &new.effect_type) {
        // Static effects conflict with other static effects
        (Static { .. }, Static { .. }) => true,

        // Static effects conflict with color cycle effects
        (Static { .. }, ColorCycle { .. }) => true,
        (ColorCycle { .. }, Static { .. }) => true,

        // Color cycle effects conflict with other color cycle effects
        (ColorCycle { .. }, ColorCycle { .. }) => true,

        // Strobe effects conflict with other strobe effects
        (Strobe { .. }, Strobe { .. }) => true,

        // Chase effects conflict with other chase effects
        (Chase { .. }, Chase { .. }) => true,

        // Rainbow effects conflict with static and color cycle effects
        (Rainbow { .. }, Static { .. }) => true,
        (Static { .. }, Rainbow { .. }) => true,
        (Rainbow { .. }, ColorCycle { .. }) => true,
        (ColorCycle { .. }, Rainbow { .. }) => true,
        (Rainbow { .. }, Rainbow { .. }) => true,

        // Dimmer and pulse effects are generally compatible (they layer)
        (Dimmer { .. }, _) => false,
        (_, Dimmer { .. }) => false,
        (Pulse { .. }, _) => false,
        (_, Pulse { .. }) => false,

        // Default: effects of different types don't conflict
        _ => false,
    }
}

/// Check if two blend modes are compatible (can layer together)
pub(crate) fn blend_modes_are_compatible(existing: BlendMode, new: BlendMode) -> bool {
    match (existing, new) {
        // Replace mode conflicts with everything
        (BlendMode::Replace, _) => false,
        (_, BlendMode::Replace) => false,

        // Multiply, Add, Overlay, and Screen can generally layer together
        (BlendMode::Multiply, BlendMode::Multiply) => true,
        (BlendMode::Add, BlendMode::Add) => true,
        (BlendMode::Overlay, BlendMode::Overlay) => true,
        (BlendMode::Screen, BlendMode::Screen) => true,

        // Different blend modes can layer if they're not Replace
        (BlendMode::Multiply, BlendMode::Add) => true,
        (BlendMode::Multiply, BlendMode::Overlay) => true,
        (BlendMode::Multiply, BlendMode::Screen) => true,
        (BlendMode::Add, BlendMode::Multiply) => true,
        (BlendMode::Add, BlendMode::Overlay) => true,
        (BlendMode::Add, BlendMode::Screen) => true,
        (BlendMode::Overlay, BlendMode::Multiply) => true,
        (BlendMode::Overlay, BlendMode::Add) => true,
        (BlendMode::Overlay, BlendMode::Screen) => true,
        (BlendMode::Screen, BlendMode::Multiply) => true,
        (BlendMode::Screen, BlendMode::Add) => true,
        (BlendMode::Screen, BlendMode::Overlay) => true,
    }
}

/// Clear a layer - immediately stops all effects on the specified layer
pub(crate) fn clear_layer(
    active_effects: &mut HashMap<String, EffectInstance>,
    releasing_effects: &mut HashMap<String, (Duration, Instant)>,
    frozen_layers: &mut HashMap<EffectLayer, Instant>,
    layer: EffectLayer,
) {
    // Collect effect IDs on this layer BEFORE removing them
    let effects_on_layer: Vec<String> = active_effects
        .iter()
        .filter(|(_, effect)| effect.layer == layer)
        .map(|(id, _)| id.clone())
        .collect();

    // Remove all effects on this layer
    active_effects.retain(|_, effect| effect.layer != layer);

    // Also remove any releasing effects that were on this layer
    for id in effects_on_layer {
        releasing_effects.remove(&id);
    }

    // Unfreeze the layer if it was frozen
    frozen_layers.remove(&layer);
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
