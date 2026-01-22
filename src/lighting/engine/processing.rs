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
use std::time::Duration;

use super::super::effects::*;
use super::super::tempo::TempoMap;

/// Calculate cycle progress (0.0 to 1.0) for a given elapsed time and cycle period
#[inline]
fn cycle_progress(elapsed: Duration, cycle_period: f64) -> f64 {
    (elapsed.as_secs_f64() % cycle_period) / cycle_period
}

/// Calculate phase for periodic effects (0.0 to 2π)
#[inline]
fn phase(elapsed: Duration, frequency: f64) -> f64 {
    elapsed.as_secs_f64() * frequency * 2.0 * std::f64::consts::PI
}

/// Calculate color indices and interpolation progress for color cycle effects
fn calculate_color_indices(
    cycle_progress: f64,
    color_count: usize,
    direction: &CycleDirection,
) -> (usize, usize, f64) {
    match direction {
        CycleDirection::Forward => {
            let color_index_f = cycle_progress * color_count as f64;
            let color_index = color_index_f.floor() as usize;
            let next_index = (color_index + 1) % color_count;
            let segment_progress = color_index_f - color_index as f64;
            (color_index, next_index, segment_progress)
        }
        CycleDirection::Backward => {
            let reversed_progress = 1.0 - cycle_progress;
            let color_index_f = reversed_progress * color_count as f64;
            let color_index = color_index_f.floor() as usize;

            // Handle boundary case: when reversed_progress = 1.0 (cycle start),
            // color_index_f = color_count, which is out of bounds.
            if color_index >= color_count {
                (color_count - 1, color_count - 1, 0.0)
            } else {
                let next_index = if color_index == 0 {
                    color_count - 1
                } else {
                    color_index - 1
                };
                let segment_progress = color_index_f - color_index as f64;
                (color_index, next_index, segment_progress)
            }
        }
        CycleDirection::PingPong => {
            // PingPong: go forward then backward through colors
            // ping_pong_progress goes 0 → 1 → 0 over one cycle
            let ping_pong_progress = if cycle_progress < 0.5 {
                cycle_progress * 2.0
            } else {
                2.0 - cycle_progress * 2.0
            };

            // Map ping_pong_progress (0 to 1) to color indices (0 to len-1)
            let max_index = (color_count - 1) as f64;
            let color_progress = ping_pong_progress * max_index;

            // Calculate current and next color indices for interpolation
            let color_index = color_progress.floor() as usize;
            let seg_progress = color_progress - color_index as f64;

            // Handle edge case when at exactly the last color
            if color_index >= color_count - 1 {
                (color_count - 1, color_count - 1, 0.0)
            } else {
                (color_index, color_index + 1, seg_progress)
            }
        }
    }
}

/// Process a single effect and return fixture states
pub(crate) fn process_effect(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    elapsed: Duration,
    engine_elapsed: Duration,
    tempo_map: Option<&TempoMap>,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    if !effect.enabled {
        return Ok(None);
    }

    // Calculate absolute time for tempo-aware effects
    let absolute_time = engine_elapsed;

    match &effect.effect_type {
        EffectType::Static { parameters, .. } => {
            apply_static_effect(fixture_registry, effect, parameters, elapsed)
        }
        EffectType::ColorCycle {
            colors,
            speed,
            direction,
            transition,
        } => {
            let current_speed = speed.to_cycles_per_second(tempo_map, absolute_time);
            apply_color_cycle(
                fixture_registry,
                effect,
                colors,
                current_speed,
                direction,
                *transition,
                elapsed,
            )
        }
        EffectType::Strobe { frequency, .. } => {
            let current_frequency = frequency.to_hz(tempo_map, absolute_time);
            apply_strobe(fixture_registry, effect, current_frequency, elapsed)
        }
        EffectType::Dimmer {
            start_level,
            end_level,
            duration,
            curve,
        } => apply_dimmer(
            fixture_registry,
            effect,
            *start_level,
            *end_level,
            curve,
            elapsed,
            *duration,
        ),
        EffectType::Chase {
            pattern,
            speed,
            direction,
            transition,
        } => {
            let current_speed = speed.to_cycles_per_second(tempo_map, absolute_time);
            apply_chase(
                fixture_registry,
                effect,
                pattern,
                current_speed,
                direction,
                *transition,
                elapsed,
            )
        }
        EffectType::Rainbow {
            speed,
            saturation,
            brightness,
        } => {
            let current_speed = speed.to_cycles_per_second(tempo_map, absolute_time);
            apply_rainbow(
                fixture_registry,
                effect,
                current_speed,
                *saturation,
                *brightness,
                elapsed,
            )
        }
        EffectType::Pulse {
            base_level,
            pulse_amplitude,
            frequency,
            ..
        } => {
            let current_frequency = frequency.to_hz(tempo_map, absolute_time);
            apply_pulse(
                fixture_registry,
                effect,
                *base_level,
                *pulse_amplitude,
                current_frequency,
                elapsed,
            )
        }
    }
}

/// Apply a static effect and return fixture states
fn apply_static_effect(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    parameters: &HashMap<String, f64>,
    elapsed: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    // Calculate crossfade multiplier
    let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

    let mut fixture_states = HashMap::new();

    for fixture_name in &effect.target_fixtures {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let mut fixture_state = FixtureState::new();

            // For static effects, we apply parameters directly
            // The fixture profile system is more useful for dynamic effects

            for (param_name, value) in parameters {
                // Apply crossfade multiplier to the value
                let faded_value = *value * crossfade_multiplier;

                // For static effects, apply parameters directly if the channel exists
                // The fixture profile system is more useful for dynamic effects that need
                // to adapt their behavior based on fixture capabilities
                if fixture.channels.contains_key(param_name) {
                    let channel_state =
                        ChannelState::new(faded_value, effect.layer, effect.blend_mode);
                    fixture_state.set_channel(param_name.clone(), channel_state);
                }
            }

            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}

/// Apply a color cycle effect and return fixture states
fn apply_color_cycle(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    colors: &[Color],
    speed: f64,
    direction: &CycleDirection,
    transition: CycleTransition,
    elapsed: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    if colors.is_empty() {
        return Ok(None);
    }

    // Calculate crossfade multiplier
    let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

    // Guard against zero/negative speed - treat as "stopped" at first color
    if speed <= 0.0 {
        let color = colors[0];
        let mut fixture_states = HashMap::new();
        for fixture_name in &effect.target_fixtures {
            if let Some(fixture) = fixture_registry.get(fixture_name) {
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands = profile.apply_color(color, effect.layer, effect.blend_mode);

                let mut fixture_state = FixtureState::new();
                for (channel_name, mut channel_state) in channel_commands {
                    channel_state.value *= crossfade_multiplier;
                    fixture_state.set_channel(channel_name, channel_state);
                }
                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }
        return Ok(Some(fixture_states));
    }

    let cycle_time = 1.0 / speed;
    let cycle_progress_val = cycle_progress(elapsed, cycle_time);

    // Calculate color indices and interpolation factor for smooth transitions
    let (color_index, next_index, segment_progress) =
        calculate_color_indices(cycle_progress_val, colors.len(), direction);

    // Apply transition based on transition type
    let color = match transition {
        CycleTransition::Fade => {
            // Interpolate between current and next color for smooth transitions
            let current_color = colors[color_index % colors.len()];
            let next_color = colors[next_index % colors.len()];
            current_color.lerp(&next_color, segment_progress)
        }
        CycleTransition::Snap => {
            // Snap to current color (original behavior)
            colors[color_index % colors.len()]
        }
    };
    let mut fixture_states = HashMap::new();

    for fixture_name in &effect.target_fixtures {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let mut fixture_state = FixtureState::new();

            // Use fixture profile to determine how to apply color
            let profile = FixtureProfile::for_fixture(fixture);
            let channel_commands = profile.apply_color(color, effect.layer, effect.blend_mode);

            // Apply the channel commands from the profile with crossfade multiplier
            for (channel_name, mut channel_state) in channel_commands {
                // Apply crossfade multiplier to the color value
                channel_state.value *= crossfade_multiplier;
                fixture_state.set_channel(channel_name, channel_state);
            }

            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}

/// Apply a strobe effect and return fixture states
fn apply_strobe(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    frequency: f64,
    elapsed: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    // Calculate crossfade multiplier
    let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

    let mut fixture_states = HashMap::new();

    for fixture_name in &effect.target_fixtures {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let mut fixture_state = FixtureState::new();

            if frequency == 0.0 {
                // Frequency 0 means strobe is disabled
                if fixture.has_capability(FixtureCapabilities::STROBING) {
                    // Hardware strobe: just disable the strobe channel
                    fixture_state.set_channel(
                        "strobe".to_string(),
                        ChannelState::new(0.0, effect.layer, effect.blend_mode),
                    );
                }
                // Software strobe: when frequency=0, don't set any channels
                // This allows parent layers/effects to take over control
            } else {
                // Use fixture profile to determine how to apply strobe control
                let profile = FixtureProfile::for_fixture(fixture);

                // Calculate strobe parameters based on strategy
                let (normalized_frequency, strobe_value) =
                    if profile.strobe_strategy == StrobeStrategy::DedicatedChannel {
                        // Hardware strobe: normalize frequency to 0-1 range
                        let max_freq = fixture.max_strobe_frequency.unwrap_or(20.0);
                        let normalized = (frequency / max_freq).min(1.0);
                        (normalized, None)
                    } else {
                        // Software strobe: calculate on/off value
                        let strobe_period = 1.0 / frequency;
                        let strobe_phase = cycle_progress(elapsed, strobe_period);
                        let is_strobe_on = strobe_phase < 0.5; // 50% duty cycle
                        (frequency, Some(if is_strobe_on { 1.0 } else { 0.0 }))
                    };

                let channel_commands = profile.apply_strobe(
                    normalized_frequency,
                    effect.layer,
                    effect.blend_mode,
                    crossfade_multiplier,
                    strobe_value,
                );

                // Apply the channel commands from the profile
                for (channel_name, channel_state) in channel_commands {
                    fixture_state.set_channel(channel_name, channel_state);
                }
            }

            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}

/// Apply a dimmer effect and return fixture states
fn apply_dimmer(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    start_level: f64,
    end_level: f64,
    curve: &DimmerCurve,
    elapsed: Duration,
    duration: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    // Calculate dimmer value based on elapsed time and duration with curve applied
    let dimmer_value = if duration.is_zero() {
        end_level // Instant transition
    } else {
        let linear_progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);

        // Apply curve to the progress value
        let curved_progress = match curve {
            DimmerCurve::Linear => linear_progress,
            DimmerCurve::Exponential => linear_progress * linear_progress,
            DimmerCurve::Logarithmic => {
                if linear_progress <= 0.0 {
                    0.0
                } else {
                    // Map [0,1] to [0,1] using log curve
                    // log(1 + 9*x) / log(10) gives nice log curve from 0 to 1
                    (1.0 + 9.0 * linear_progress).log10()
                }
            }
            DimmerCurve::Sine => {
                // Smooth ease-in-out using sine
                (1.0 - ((linear_progress * std::f64::consts::PI).cos())) / 2.0
            }
            DimmerCurve::Cosine => {
                // Smooth ease-in using cosine
                1.0 - (1.0 - linear_progress).powi(2)
            }
        };

        start_level + (end_level - start_level) * curved_progress
    };

    // Apply dimmer to all fixtures
    let mut fixture_states = HashMap::new();
    for fixture_name in &effect.target_fixtures {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let profile = FixtureProfile::for_fixture(fixture);
            let channel_commands =
                profile.apply_brightness(dimmer_value, effect.layer, effect.blend_mode);

            let mut fixture_state = FixtureState::new();
            for (channel_name, channel_state) in channel_commands {
                fixture_state.set_channel(channel_name, channel_state);
            }
            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}

/// Apply a chase effect and return fixture states
fn apply_chase(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    pattern: &ChasePattern,
    speed: f64,
    direction: &ChaseDirection,
    transition: CycleTransition,
    elapsed: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    // Calculate crossfade multiplier
    let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

    // Guard against zero/negative speed - treat as "stopped" with first fixture active
    if speed <= 0.0 {
        let mut fixture_states = HashMap::new();
        for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
            if let Some(fixture) = fixture_registry.get(fixture_name) {
                let mut fixture_state = FixtureState::new();
                let chase_value = if i == 0 { crossfade_multiplier } else { 0.0 };
                let profile = FixtureProfile::for_fixture(fixture);
                let channel_commands =
                    profile.apply_chase(chase_value, effect.layer, effect.blend_mode);
                for (channel_name, channel_state) in channel_commands {
                    fixture_state.set_channel(channel_name, channel_state);
                }
                fixture_states.insert(fixture_name.clone(), fixture_state);
            }
        }
        return Ok(Some(fixture_states));
    }

    let chase_period = if speed > 0.0 {
        1.0 / speed
    } else {
        f64::INFINITY
    };

    let mut fixture_states = HashMap::new();
    let fixture_count = effect.target_fixtures.len();

    // Guard against empty fixture list - nothing to chase
    if fixture_count == 0 {
        return Ok(Some(fixture_states));
    }

    // Calculate fixture order based on pattern and direction
    // For Random pattern, use cue_time (song time) as seed for deterministic randomness
    // This ensures random effects are consistent when seeking to the same time
    let fixture_order = calculate_fixture_order(fixture_count, pattern, direction, effect.cue_time);

    // Calculate the pattern cycle length
    let pattern_length = fixture_order.len();

    // Use consistent timing for all patterns
    // Each position in the pattern should last the same time as a linear chase position
    let position_duration = chase_period / fixture_count as f64;
    let pattern_cycle_period = position_duration * pattern_length as f64;
    let pattern_progress = cycle_progress(elapsed, pattern_cycle_period);
    let current_pattern_index_f = pattern_progress * pattern_length as f64;
    let current_pattern_index = current_pattern_index_f.floor() as usize;
    let position_progress = current_pattern_index_f - current_pattern_index as f64;

    for (i, fixture_name) in effect.target_fixtures.iter().enumerate() {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let mut fixture_state = FixtureState::new();

            let chase_value = match transition {
                CycleTransition::Snap => {
                    // Binary on/off: fixture is either fully on or fully off
                    let is_fixture_active = if current_pattern_index < pattern_length {
                        fixture_order[current_pattern_index] == i
                    } else {
                        false
                    };
                    if is_fixture_active {
                        1.0
                    } else {
                        0.0
                    }
                }
                CycleTransition::Fade => {
                    // Fade in/out: calculate smooth transitions
                    // Each fixture fades in over the first 50% of its position, stays at 1.0 for the middle,
                    // and fades out over the last 50% of its position
                    let fade_ratio = 0.5; // 50% of position_duration for fade in, 50% for fade out

                    // Check if this fixture is the current active fixture
                    let is_current = if current_pattern_index < pattern_length {
                        fixture_order[current_pattern_index] == i
                    } else {
                        false
                    };

                    // Check if this fixture is the previous fixture (fading out)
                    let prev_index = if current_pattern_index > 0 {
                        current_pattern_index - 1
                    } else {
                        pattern_length - 1
                    };
                    let is_previous = if prev_index < pattern_length {
                        fixture_order[prev_index] == i
                    } else {
                        false
                    };

                    if is_current {
                        // Current fixture: fade in if at start of position, otherwise full on
                        if position_progress < fade_ratio {
                            // Fading in: 0.0 to 1.0 over first 50% of position
                            position_progress / fade_ratio
                        } else {
                            // Fully on for the rest of the position
                            1.0
                        }
                    } else if is_previous {
                        // Previous fixture: fade out if at start of next position
                        if position_progress < fade_ratio {
                            // Fading out: 1.0 to 0.0 over first 50% of next position
                            1.0 - (position_progress / fade_ratio)
                        } else {
                            // Fully off
                            0.0
                        }
                    } else {
                        // Not active
                        0.0
                    }
                }
            } * crossfade_multiplier;

            // Use fixture profile to determine how to apply chase control
            let profile = FixtureProfile::for_fixture(fixture);
            let channel_commands =
                profile.apply_chase(chase_value, effect.layer, effect.blend_mode);

            // Apply the channel commands from the profile
            for (channel_name, channel_state) in channel_commands {
                fixture_state.set_channel(channel_name, channel_state);
            }

            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}

/// Calculate fixture order for chase effects based on pattern and direction
fn calculate_fixture_order(
    fixture_count: usize,
    pattern: &ChasePattern,
    direction: &ChaseDirection,
    cue_time: Option<Duration>,
) -> Vec<usize> {
    match pattern {
        ChasePattern::Random => {
            use rand::rngs::StdRng;
            use rand::{Rng, SeedableRng};
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut order: Vec<usize> = (0..fixture_count).collect();

            // Generate a seed from the cue_time (song time when effect was supposed to start)
            // This ensures random effects are deterministic based on their position in the timeline
            // When seeking, effects use their intended cue time, ensuring consistency
            let seed = if let Some(cue) = cue_time {
                let mut hasher = DefaultHasher::new();
                cue.as_nanos().hash(&mut hasher);
                fixture_count.hash(&mut hasher); // Include fixture count for uniqueness
                hasher.finish()
            } else {
                // Fallback if cue_time not set (shouldn't happen in normal operation)
                fixture_count as u64 * 7 + 13
            };

            // Create a seeded RNG for deterministic shuffling
            let mut rng = StdRng::seed_from_u64(seed);

            // Fisher-Yates shuffle with proper random number generator
            for i in (1..fixture_count).rev() {
                let j = rng.gen_range(0..=i);
                order.swap(i, j);
            }

            order
        }
        ChasePattern::Linear => {
            // Linear pattern - fixtures in order
            let mut order: Vec<usize> = (0..fixture_count).collect();
            // Direction determines if we reverse the order
            match direction {
                ChaseDirection::LeftToRight
                | ChaseDirection::TopToBottom
                | ChaseDirection::Clockwise => {
                    // Forward direction - keep original order
                    order
                }
                ChaseDirection::RightToLeft
                | ChaseDirection::BottomToTop
                | ChaseDirection::CounterClockwise => {
                    // Reverse direction - reverse the order
                    order.reverse();
                    order
                }
            }
        }
        ChasePattern::Snake => {
            // Snake pattern - forward then reverse
            // Create a snake pattern: 0, 1, 2, 3, 2, 1, 0, 1, 2, 3, ...
            let mut snake_order = Vec::new();

            // Forward pass: 0, 1, 2, 3
            for i in 0..fixture_count {
                snake_order.push(i);
            }

            // Reverse pass: 2, 1 (skip the last element to avoid duplication)
            for i in (1..fixture_count - 1).rev() {
                snake_order.push(i);
            }

            // Apply direction
            match direction {
                ChaseDirection::LeftToRight
                | ChaseDirection::TopToBottom
                | ChaseDirection::Clockwise => {
                    // Forward direction - use snake order as is
                    snake_order
                }
                ChaseDirection::RightToLeft
                | ChaseDirection::BottomToTop
                | ChaseDirection::CounterClockwise => {
                    // Reverse direction - reverse the snake order
                    snake_order.reverse();
                    snake_order
                }
            }
        }
    }
}

/// Apply a rainbow effect and return fixture states
fn apply_rainbow(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    speed: f64,
    saturation: f64,
    brightness: f64,
    elapsed: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    // Calculate crossfade multiplier
    let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

    // Calculate hue: cycles through 360 degrees based on speed
    let hue = (elapsed.as_secs_f64() * speed * 360.0) % 360.0;
    let color = Color::from_hsv(hue, saturation, brightness);

    let mut fixture_states = HashMap::new();

    for fixture_name in &effect.target_fixtures {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let mut fixture_state = FixtureState::new();

            // Use fixture profile to determine how to apply color
            let profile = FixtureProfile::for_fixture(fixture);
            let channel_commands = profile.apply_color(color, effect.layer, effect.blend_mode);

            // Apply the channel commands from the profile with crossfade multiplier
            for (channel_name, mut channel_state) in channel_commands {
                // Apply crossfade multiplier to the color value
                channel_state.value *= crossfade_multiplier;
                fixture_state.set_channel(channel_name, channel_state);
            }

            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}

/// Apply a pulse effect and return fixture states
fn apply_pulse(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
    base_level: f64,
    pulse_amplitude: f64,
    frequency: f64,
    elapsed: Duration,
) -> Result<Option<HashMap<String, FixtureState>>, EffectError> {
    // Calculate crossfade multiplier
    let crossfade_multiplier = effect.calculate_crossfade_multiplier(elapsed);

    let pulse_phase = phase(elapsed, frequency);
    // Convert sine wave (-1 to 1) to 0-1 range, then scale by amplitude
    let pulse_value =
        (base_level + pulse_amplitude * (pulse_phase.sin() * 0.5 + 0.5)) * crossfade_multiplier;

    // Apply pulse to all fixtures
    let mut fixture_states = HashMap::new();
    for fixture_name in &effect.target_fixtures {
        if let Some(fixture) = fixture_registry.get(fixture_name) {
            let profile = FixtureProfile::for_fixture(fixture);
            let channel_commands =
                profile.apply_pulse(pulse_value, effect.layer, effect.blend_mode);

            let mut fixture_state = FixtureState::new();
            for (channel_name, channel_state) in channel_commands {
                fixture_state.set_channel(channel_name, channel_state);
            }
            fixture_states.insert(fixture_name.clone(), fixture_state);
        }
    }

    Ok(Some(fixture_states))
}
