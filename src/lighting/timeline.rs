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

use crate::config::{LightingCue, LightingEffect};
use crate::lighting::{
    effects::{ChaseDirection, ChasePattern, CycleDirection, DimmerCurve},
    Color, EffectInstance, EffectType,
};
use std::collections::HashMap;
use std::time::Duration;

/// Timeline processor for lighting cues during song playback
pub struct LightingTimeline {
    /// Cues sorted by time
    cues: Vec<LightingCue>,
    /// Current position in the timeline
    current_time: Duration,
    /// Index of the next cue to process
    next_cue_index: usize,
    /// Whether the timeline is currently playing
    is_playing: bool,
}

impl LightingTimeline {
    /// Creates a new lighting timeline from cues
    pub fn new(cues: Vec<LightingCue>) -> Self {
        let mut timeline = Self {
            cues,
            current_time: Duration::ZERO,
            next_cue_index: 0,
            is_playing: false,
        };
        timeline.sort_cues();
        timeline
    }

    /// Sorts cues by time
    fn sort_cues(&mut self) {
        self.cues.sort_by(|a, b| {
            let time_a = Self::parse_time_string(a.time());
            let time_b = Self::parse_time_string(b.time());
            time_a.cmp(&time_b)
        });
    }

    /// Parses a time string in format "MM:SS.mmm" to Duration
    fn parse_time_string(time_str: &str) -> Duration {
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() != 2 {
            return Duration::ZERO;
        }

        let minutes: u64 = parts[0].parse().unwrap_or(0);
        let seconds_part = parts[1];

        let seconds_parts: Vec<&str> = seconds_part.split('.').collect();
        let seconds: u64 = seconds_parts[0].parse().unwrap_or(0);
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            // Pad or truncate to 3 digits
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse().unwrap_or(0)
        } else {
            0
        };

        Duration::from_secs(minutes * 60 + seconds) + Duration::from_millis(milliseconds)
    }

    /// Starts the timeline
    pub fn start(&mut self) {
        self.is_playing = true;
        self.current_time = Duration::ZERO;
        self.next_cue_index = 0;
    }

    /// Stops the timeline
    pub fn stop(&mut self) {
        self.is_playing = false;
    }

    /// Updates the timeline with the current song time
    pub fn update(&mut self, song_time: Duration) -> Vec<EffectInstance> {
        if !self.is_playing {
            return Vec::new();
        }

        self.current_time = song_time;
        let mut triggered_effects = Vec::new();

        // Process all cues that should trigger at or before the current time
        while self.next_cue_index < self.cues.len() {
            let cue = &self.cues[self.next_cue_index];
            let cue_time = Self::parse_time_string(cue.time());

            if cue_time <= song_time {
                // This cue should trigger
                for effect in cue.effects() {
                    if let Some(effect_instance) = Self::create_effect_instance(effect) {
                        triggered_effects.push(effect_instance);
                    }
                }
                self.next_cue_index += 1;
            } else {
                // No more cues to process at this time
                break;
            }
        }

        triggered_effects
    }

    /// Creates an EffectInstance from a LightingEffect
    fn create_effect_instance(effect: &LightingEffect) -> Option<EffectInstance> {
        let effect_type = match effect.effect_type() {
            "static" => Self::create_static_effect(effect),
            "color_cycle" => Self::create_color_cycle_effect(effect),
            "strobe" => Self::create_strobe_effect(effect),
            "dimmer" => Self::create_dimmer_effect(effect),
            "chase" => Self::create_chase_effect(effect),
            "chaser" => Self::create_chaser_effect(effect),
            "rainbow" => Self::create_rainbow_effect(effect),
            "pulse" => Self::create_pulse_effect(effect),
            _ => return None,
        };

        // Create base effect instance
        let mut effect_instance = EffectInstance::new(
            format!(
                "song_effect_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            effect_type,
            effect.groups().clone(),
        );

        // Apply builder methods based on parameters
        if let Some(priority) = effect.parameters().get("priority").and_then(|v| v.as_f64()) {
            effect_instance = effect_instance.with_priority(priority as u8);
        }

        if let Some(fade_in) = effect.parameters().get("fade_in").and_then(|v| v.as_f64()) {
            if let Some(fade_out) = effect.parameters().get("fade_out").and_then(|v| v.as_f64()) {
                effect_instance = effect_instance.with_fades(
                    Some(Duration::from_secs_f64(fade_in)),
                    Some(Duration::from_secs_f64(fade_out)),
                );
            }
        }

        if let Some(duration) = effect.parameters().get("duration").and_then(|v| v.as_f64()) {
            effect_instance = effect_instance.with_timing(
                Some(std::time::Instant::now()),
                Some(Duration::from_secs_f64(duration)),
            );
        }

        Some(effect_instance)
    }

    /// Creates a static effect
    pub fn create_static_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "color" => {
                    // Handle color parameter by converting to RGB channels
                    if let Some(color_str) = value.as_str() {
                        if let Some(color) = Self::parse_color_string(color_str) {
                            parameters.insert("red".to_string(), color.r as f64 / 255.0);
                            parameters.insert("green".to_string(), color.g as f64 / 255.0);
                            parameters.insert("blue".to_string(), color.b as f64 / 255.0);
                        }
                    }
                }
                "red" | "green" | "blue" | "white" => {
                    // Handle individual color channel parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
                "dimmer" => {
                    // Handle dimmer parameter (can be percentage string or number)
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    } else if let Some(dimmer_str) = value.as_str() {
                        if dimmer_str.ends_with('%') {
                            if let Ok(percentage) = dimmer_str.trim_end_matches('%').parse::<f64>()
                            {
                                parameters.insert(key.clone(), percentage / 100.0);
                            }
                        }
                    }
                }
                _ => {
                    // Handle other numeric parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
            }
        }

        EffectType::Static {
            parameters,
            duration: None,
        }
    }

    /// Creates a color cycle effect
    fn create_color_cycle_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();
        let mut colors = Vec::new();

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "colors" => {
                    if let Some(color_list) = value.as_sequence() {
                        for color_val in color_list {
                            if let Some(color_str) = color_val.as_str() {
                                if let Some(color) = Self::parse_color_string(color_str) {
                                    colors.push(color);
                                }
                            }
                        }
                    }
                }
                "speed" => {
                    if let Some(speed) = value.as_f64() {
                        parameters.insert("speed".to_string(), speed);
                    }
                }
                "dimmer" => {
                    if let Some(dimmer) = value.as_f64() {
                        parameters.insert("dimmer".to_string(), dimmer);
                    }
                }
                _ => {}
            }
        }

        let direction = if let Some(direction_str) = effect
            .parameters()
            .get("direction")
            .and_then(|v| v.as_str())
        {
            match direction_str {
                "forward" => CycleDirection::Forward,
                "backward" => CycleDirection::Backward,
                "ping_pong" => CycleDirection::PingPong,
                _ => CycleDirection::Forward,
            }
        } else {
            CycleDirection::Forward
        };

        EffectType::ColorCycle {
            colors,
            speed: parameters.get("speed").copied().unwrap_or(1.0),
            direction,
        }
    }

    /// Creates a strobe effect
    pub fn create_strobe_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "color" => {
                    // Handle color parameter by converting to RGB channels
                    if let Some(color_str) = value.as_str() {
                        if let Some(color) = Self::parse_color_string(color_str) {
                            parameters.insert("red".to_string(), color.r as f64 / 255.0);
                            parameters.insert("green".to_string(), color.g as f64 / 255.0);
                            parameters.insert("blue".to_string(), color.b as f64 / 255.0);
                        }
                    }
                }
                "red" | "green" | "blue" | "white" => {
                    // Handle individual color channel parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
                "dimmer" => {
                    // Handle dimmer parameter (can be percentage string or number)
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    } else if let Some(dimmer_str) = value.as_str() {
                        if dimmer_str.ends_with('%') {
                            if let Ok(percentage) = dimmer_str.trim_end_matches('%').parse::<f64>()
                            {
                                parameters.insert(key.clone(), percentage / 100.0);
                            }
                        }
                    }
                }
                _ => {
                    // Handle other numeric parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
            }
        }

        EffectType::Strobe {
            frequency: parameters.get("frequency").copied().unwrap_or(10.0),
            intensity: parameters.get("intensity").copied().unwrap_or(1.0),
            duration: None,
        }
    }

    /// Creates a dimmer effect
    fn create_dimmer_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();
        let mut curve = DimmerCurve::Linear;

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "start_level" | "end_level" | "duration" => {
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
                "curve" => {
                    if let Some(curve_str) = value.as_str() {
                        curve = match curve_str {
                            "linear" => DimmerCurve::Linear,
                            "exponential" => DimmerCurve::Exponential,
                            "logarithmic" => DimmerCurve::Logarithmic,
                            "sine" => DimmerCurve::Sine,
                            "cosine" => DimmerCurve::Cosine,
                            _ => DimmerCurve::Linear,
                        };
                    }
                }
                "custom_curve" => {
                    if let Some(curve_list) = value.as_sequence() {
                        let mut custom_points = Vec::new();
                        for curve_val in curve_list {
                            if let Some(point) = curve_val.as_f64() {
                                custom_points.push(point);
                            }
                        }
                        if !custom_points.is_empty() {
                            curve = DimmerCurve::Custom(custom_points);
                        }
                    }
                }
                _ => {
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
            }
        }

        EffectType::Dimmer {
            start_level: parameters.get("start_level").copied().unwrap_or(0.0),
            end_level: parameters.get("end_level").copied().unwrap_or(1.0),
            duration: Duration::from_secs_f64(parameters.get("duration").copied().unwrap_or(1.0)),
            curve,
        }
    }

    /// Creates a chase effect
    pub fn create_chase_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();
        let mut pattern = ChasePattern::Linear;
        let mut direction = ChaseDirection::LeftToRight;

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "speed" => {
                    if let Some(speed) = value.as_f64() {
                        parameters.insert("speed".to_string(), speed);
                    }
                }
                "pattern" => {
                    if let Some(pattern_str) = value.as_str() {
                        pattern = match pattern_str {
                            "linear" => ChasePattern::Linear,
                            "snake" => ChasePattern::Snake,
                            "random" => ChasePattern::Random,
                            _ => ChasePattern::Linear,
                        };
                    }
                }
                "custom_pattern" => {
                    if let Some(pattern_list) = value.as_sequence() {
                        let mut custom_order = Vec::new();
                        for pattern_val in pattern_list {
                            if let Some(index) = pattern_val.as_u64() {
                                custom_order.push(index as usize);
                            }
                        }
                        if !custom_order.is_empty() {
                            pattern = ChasePattern::Linear; // Use Linear as fallback
                        }
                    }
                }
                "direction" => {
                    if let Some(direction_str) = value.as_str() {
                        direction = match direction_str {
                            "left_to_right" => ChaseDirection::LeftToRight,
                            "right_to_left" => ChaseDirection::RightToLeft,
                            "top_to_bottom" => ChaseDirection::TopToBottom,
                            "bottom_to_top" => ChaseDirection::BottomToTop,
                            "clockwise" => ChaseDirection::Clockwise,
                            "counter_clockwise" => ChaseDirection::CounterClockwise,
                            _ => ChaseDirection::LeftToRight,
                        };
                    }
                }
                "color" => {
                    // Handle color parameter by converting to RGB channels
                    if let Some(color_str) = value.as_str() {
                        if let Some(color) = Self::parse_color_string(color_str) {
                            parameters.insert("red".to_string(), color.r as f64 / 255.0);
                            parameters.insert("green".to_string(), color.g as f64 / 255.0);
                            parameters.insert("blue".to_string(), color.b as f64 / 255.0);
                        }
                    }
                }
                "red" | "green" | "blue" | "white" => {
                    // Handle individual color channel parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
                "dimmer" => {
                    // Handle dimmer parameter (can be percentage string or number)
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    } else if let Some(dimmer_str) = value.as_str() {
                        if dimmer_str.ends_with('%') {
                            if let Ok(percentage) = dimmer_str.trim_end_matches('%').parse::<f64>()
                            {
                                parameters.insert(key.clone(), percentage / 100.0);
                            }
                        }
                    }
                }
                _ => {
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
            }
        }

        EffectType::Chase {
            pattern,
            speed: parameters.get("speed").copied().unwrap_or(1.0),
            direction,
        }
    }

    /// Creates a rainbow effect
    fn create_rainbow_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();

        for (key, value) in effect.parameters() {
            if let Some(num) = value.as_f64() {
                parameters.insert(key.clone(), num);
            }
        }

        EffectType::Rainbow {
            speed: parameters.get("speed").copied().unwrap_or(1.0),
            saturation: parameters.get("saturation").copied().unwrap_or(1.0),
            brightness: parameters.get("brightness").copied().unwrap_or(1.0),
        }
    }

    /// Creates a pulse effect
    pub fn create_pulse_effect(effect: &LightingEffect) -> EffectType {
        let mut parameters = HashMap::new();

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "color" => {
                    // Handle color parameter by converting to RGB channels
                    if let Some(color_str) = value.as_str() {
                        if let Some(color) = Self::parse_color_string(color_str) {
                            parameters.insert("red".to_string(), color.r as f64 / 255.0);
                            parameters.insert("green".to_string(), color.g as f64 / 255.0);
                            parameters.insert("blue".to_string(), color.b as f64 / 255.0);
                        }
                    }
                }
                "red" | "green" | "blue" | "white" => {
                    // Handle individual color channel parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
                "dimmer" => {
                    // Handle dimmer parameter (can be percentage string or number)
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    } else if let Some(dimmer_str) = value.as_str() {
                        if dimmer_str.ends_with('%') {
                            if let Ok(percentage) = dimmer_str.trim_end_matches('%').parse::<f64>()
                            {
                                parameters.insert(key.clone(), percentage / 100.0);
                            }
                        }
                    }
                }
                _ => {
                    // Handle other numeric parameters
                    if let Some(num) = value.as_f64() {
                        parameters.insert(key.clone(), num);
                    }
                }
            }
        }

        EffectType::Pulse {
            base_level: parameters.get("base_level").copied().unwrap_or(0.5),
            pulse_amplitude: parameters.get("pulse_amplitude").copied().unwrap_or(0.5),
            frequency: parameters.get("frequency").copied().unwrap_or(1.0),
            duration: None,
        }
    }

    /// Creates a chaser effect - returns a special chaser effect type
    fn create_chaser_effect(effect: &LightingEffect) -> EffectType {
        let mut steps = Vec::new();
        let mut loop_mode = crate::lighting::effects::LoopMode::Loop;
        let mut direction = crate::lighting::effects::ChaserDirection::Forward;

        for (key, value) in effect.parameters() {
            match key.as_str() {
                "steps" => {
                    if let Some(steps_list) = value.as_sequence() {
                        for step_val in steps_list {
                            if let Some(step_map) = step_val.as_mapping() {
                                let mut step_parameters = HashMap::new();
                                let mut hold_time = Duration::from_secs(1);
                                let mut transition_time = Duration::from_millis(100);
                                let mut transition_type =
                                    crate::lighting::effects::TransitionType::Snap;

                                for (step_key, step_value) in step_map {
                                    match step_key.as_str().unwrap_or("") {
                                        "hold_time" => {
                                            if let Some(time) = step_value.as_f64() {
                                                hold_time = Duration::from_secs_f64(time);
                                            }
                                        }
                                        "transition_time" => {
                                            if let Some(time) = step_value.as_f64() {
                                                transition_time = Duration::from_secs_f64(time);
                                            }
                                        }
                                        "transition_type" => {
                                            if let Some(trans_str) = step_value.as_str() {
                                                transition_type = match trans_str {
                                                    "snap" => crate::lighting::effects::TransitionType::Snap,
                                                    "fade" => crate::lighting::effects::TransitionType::Fade,
                                                    "crossfade" => crate::lighting::effects::TransitionType::Crossfade,
                                                    "wipe" => crate::lighting::effects::TransitionType::Wipe,
                                                    _ => crate::lighting::effects::TransitionType::Snap,
                                                };
                                            }
                                        }
                                        _ => {
                                            if let Some(num) = step_value.as_f64() {
                                                step_parameters.insert(
                                                    step_key.as_str().unwrap_or("").to_string(),
                                                    num,
                                                );
                                            }
                                        }
                                    }
                                }

                                let step_effect = EffectInstance::new(
                                    format!("chaser_step_{}", steps.len()),
                                    EffectType::Static {
                                        parameters: step_parameters,
                                        duration: None,
                                    },
                                    effect.groups().clone(),
                                );

                                let chaser_step = crate::lighting::effects::ChaserStep {
                                    effect: step_effect,
                                    hold_time,
                                    transition_time,
                                    transition_type,
                                };

                                steps.push(chaser_step);
                            }
                        }
                    }
                }
                "loop_mode" => {
                    if let Some(loop_str) = value.as_str() {
                        loop_mode = match loop_str {
                            "once" => crate::lighting::effects::LoopMode::Once,
                            "loop" => crate::lighting::effects::LoopMode::Loop,
                            "ping_pong" => crate::lighting::effects::LoopMode::PingPong,
                            "random" => crate::lighting::effects::LoopMode::Random,
                            _ => crate::lighting::effects::LoopMode::Loop,
                        };
                    }
                }
                "direction" => {
                    if let Some(direction_str) = value.as_str() {
                        direction = match direction_str {
                            "forward" => crate::lighting::effects::ChaserDirection::Forward,
                            "backward" => crate::lighting::effects::ChaserDirection::Backward,
                            "random" => crate::lighting::effects::ChaserDirection::Random,
                            _ => crate::lighting::effects::ChaserDirection::Forward,
                        };
                    }
                }
                "speed_multiplier" => if let Some(_speed) = value.as_f64() {},
                _ => {}
            }
        }

        // Create a chaser effect type that contains the chaser definition
        EffectType::Chaser {
            chaser: crate::lighting::effects::Chaser {
                id: format!(
                    "chaser_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ),
                steps,
                loop_mode,
                direction,
            },
        }
    }

    /// Parses a color string (e.g., "red", "blue", "#FF0000") to a Color
    pub fn parse_color_string(color_str: &str) -> Option<Color> {
        // Try named colors first
        if let Ok(color) = Color::from_name(color_str) {
            return Some(color);
        }

        // Try hex colors
        if color_str.starts_with('#') {
            if let Ok(color) = Color::from_hex(color_str) {
                return Some(color);
            }
        }

        // Try RGB format
        if color_str.starts_with("rgb(") && color_str.ends_with(')') {
            let rgb_str = &color_str[4..color_str.len() - 1];
            let parts: Vec<&str> = rgb_str.split(',').collect();
            if parts.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    parts[0].trim().parse::<u8>(),
                    parts[1].trim().parse::<u8>(),
                    parts[2].trim().parse::<u8>(),
                ) {
                    return Some(Color::new(r, g, b));
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time_string() {
        assert_eq!(
            LightingTimeline::parse_time_string("0:00.000"),
            Duration::ZERO
        );
        assert_eq!(
            LightingTimeline::parse_time_string("0:05.000"),
            Duration::from_secs(5)
        );
        assert_eq!(
            LightingTimeline::parse_time_string("1:30.500"),
            Duration::from_secs(90) + Duration::from_millis(500)
        );
        assert_eq!(
            LightingTimeline::parse_time_string("2:15.250"),
            Duration::from_secs(135) + Duration::from_millis(250)
        );
    }

    #[test]
    fn test_timeline_creation() {
        let cues = vec![];
        let timeline = LightingTimeline::new(cues);
        assert_eq!(timeline.cues.len(), 0);
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_timeline_start_stop() {
        let cues = vec![];
        let mut timeline = LightingTimeline::new(cues);

        assert!(!timeline.is_playing);
        timeline.start();
        assert!(timeline.is_playing);
        timeline.stop();
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_color_parsing() {
        assert_eq!(
            LightingTimeline::parse_color_string("red"),
            Some(Color::new(255, 0, 0))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("green"),
            Some(Color::new(0, 255, 0))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("blue"),
            Some(Color::new(0, 0, 255))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("#FF0000"),
            Some(Color::new(255, 0, 0))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("#00FF00"),
            Some(Color::new(0, 255, 0))
        );
        assert_eq!(LightingTimeline::parse_color_string("unknown"), None);
    }

    #[test]
    fn test_timeline_with_dsl_cues() {
        use crate::config::{LightingCue, LightingEffect};
        use std::collections::HashMap;

        // Create cues similar to what would come from DSL parsing
        let mut parameters1 = HashMap::new();
        parameters1.insert(
            "color".to_string(),
            serde_yml::Value::String("blue".to_string()),
        );
        parameters1.insert(
            "dimmer".to_string(),
            serde_yml::Value::String("60%".to_string()),
        );

        let mut parameters2 = HashMap::new();
        parameters2.insert(
            "color".to_string(),
            serde_yml::Value::String("red".to_string()),
        );
        parameters2.insert(
            "dimmer".to_string(),
            serde_yml::Value::String("80%".to_string()),
        );

        let cues = vec![
            LightingCue::new(
                "00:00.000".to_string(),
                Some("Opening cue".to_string()),
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["front_wash".to_string()],
                    parameters1,
                )],
            ),
            LightingCue::new(
                "00:05.000".to_string(),
                Some("Second cue".to_string()),
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["back_wash".to_string()],
                    parameters2,
                )],
            ),
        ];

        let mut timeline = LightingTimeline::new(cues);
        timeline.start();

        // Test that the first cue triggers at the right time
        let effects = timeline.update(Duration::from_millis(0));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target_fixtures, vec!["front_wash"]);

        // Test that the second cue triggers at the right time
        let effects = timeline.update(Duration::from_millis(5000));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target_fixtures, vec!["back_wash"]);

        // Test that no new effects are triggered after all cues
        let effects = timeline.update(Duration::from_millis(10000));
        assert_eq!(effects.len(), 0);
    }

    #[test]
    fn test_timeline_cue_ordering() {
        use crate::config::{LightingCue, LightingEffect};
        use std::collections::HashMap;

        // Create cues in non-chronological order
        let mut parameters = HashMap::new();
        parameters.insert(
            "color".to_string(),
            serde_yml::Value::String("blue".to_string()),
        );

        let cues = vec![
            LightingCue::new(
                "00:10.000".to_string(),
                None,
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["late_cue".to_string()],
                    parameters.clone(),
                )],
            ),
            LightingCue::new(
                "00:05.000".to_string(),
                None,
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["middle_cue".to_string()],
                    parameters.clone(),
                )],
            ),
            LightingCue::new(
                "00:00.000".to_string(),
                None,
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["early_cue".to_string()],
                    parameters,
                )],
            ),
        ];

        let mut timeline = LightingTimeline::new(cues);
        timeline.start();

        // Verify cues are processed in chronological order
        let effects = timeline.update(Duration::from_millis(0));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target_fixtures, vec!["early_cue"]);

        let effects = timeline.update(Duration::from_millis(5000));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target_fixtures, vec!["middle_cue"]);

        let effects = timeline.update(Duration::from_millis(10000));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target_fixtures, vec!["late_cue"]);
    }

    #[test]
    fn test_timeline_edge_cases() {
        use crate::config::{LightingCue, LightingEffect};
        use std::collections::HashMap;

        // Test with empty timeline
        let timeline = LightingTimeline::new(vec![]);
        assert_eq!(timeline.cues.len(), 0);

        // Test with cues at the same time
        let mut parameters = HashMap::new();
        parameters.insert(
            "color".to_string(),
            serde_yml::Value::String("blue".to_string()),
        );

        let cues = vec![
            LightingCue::new(
                "00:05.000".to_string(),
                None,
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["cue1".to_string()],
                    parameters.clone(),
                )],
            ),
            LightingCue::new(
                "00:05.000".to_string(),
                None,
                vec![LightingEffect::new(
                    "static".to_string(),
                    vec!["cue2".to_string()],
                    parameters,
                )],
            ),
        ];

        let mut timeline = LightingTimeline::new(cues);
        timeline.start();

        // Both cues should trigger at the same time
        let effects = timeline.update(Duration::from_millis(5000));
        assert_eq!(effects.len(), 2);
    }

    #[test]
    fn test_parse_color_string() {
        // Test named colors
        assert_eq!(
            LightingTimeline::parse_color_string("red"),
            Some(Color::new(255, 0, 0))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("blue"),
            Some(Color::new(0, 0, 255))
        );

        // Test hex colors
        assert_eq!(
            LightingTimeline::parse_color_string("#ff0000"),
            Some(Color::new(255, 0, 0))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("#00ff00"),
            Some(Color::new(0, 255, 0))
        );

        // Test RGB colors
        assert_eq!(
            LightingTimeline::parse_color_string("rgb(255, 0, 0)"),
            Some(Color::new(255, 0, 0))
        );
        assert_eq!(
            LightingTimeline::parse_color_string("rgb(0, 255, 0)"),
            Some(Color::new(0, 255, 0))
        );

        // Test invalid colors
        assert_eq!(LightingTimeline::parse_color_string("invalid"), None);
        assert_eq!(LightingTimeline::parse_color_string("#gggggg"), None);
    }

    #[test]
    fn test_static_effect_color_handling() {
        use crate::config::LightingEffect;
        use serde_yml::Value;
        use std::collections::HashMap;

        // Create a lighting effect with color parameter
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), Value::String("#ff0000".to_string()));
        parameters.insert("dimmer".to_string(), Value::String("60%".to_string()));

        let effect = LightingEffect::new(
            "static".to_string(),
            vec!["front_wash".to_string()],
            parameters,
        );

        // Test that the static effect correctly processes color
        let effect_type = LightingTimeline::create_static_effect(&effect);

        if let EffectType::Static { parameters, .. } = effect_type {
            // Verify that color was converted to RGB channels
            assert!(parameters.contains_key("red"));
            assert!(parameters.contains_key("green"));
            assert!(parameters.contains_key("blue"));
            assert!(parameters.contains_key("dimmer"));

            // Verify RGB values (red should be 1.0, green and blue should be 0.0)
            assert_eq!(parameters.get("red"), Some(&1.0));
            assert_eq!(parameters.get("green"), Some(&0.0));
            assert_eq!(parameters.get("blue"), Some(&0.0));
        } else {
            panic!("Expected Static effect type");
        }
    }

    #[test]
    fn test_multiple_effect_types_color_handling() {
        use crate::config::LightingEffect;
        use serde_yml::Value;
        use std::collections::HashMap;

        // Test strobe effect with color
        let mut strobe_params = HashMap::new();
        strobe_params.insert("color".to_string(), Value::String("blue".to_string()));
        strobe_params.insert("frequency".to_string(), Value::String("8".to_string()));

        let strobe_effect = LightingEffect::new(
            "strobe".to_string(),
            vec!["strobe_lights".to_string()],
            strobe_params,
        );

        let strobe_type = LightingTimeline::create_strobe_effect(&strobe_effect);
        if let EffectType::Strobe { .. } = strobe_type {
            // Strobe effect should have been created successfully
        } else {
            panic!("Expected Strobe effect type");
        }

        // Test pulse effect with color
        let mut pulse_params = HashMap::new();
        pulse_params.insert(
            "color".to_string(),
            Value::String("rgb(0, 255, 0)".to_string()),
        );
        pulse_params.insert("frequency".to_string(), Value::String("2".to_string()));

        let pulse_effect = LightingEffect::new(
            "pulse".to_string(),
            vec!["pulse_lights".to_string()],
            pulse_params,
        );

        let pulse_type = LightingTimeline::create_pulse_effect(&pulse_effect);
        if let EffectType::Pulse { .. } = pulse_type {
            // Pulse effect should have been created successfully
        } else {
            panic!("Expected Pulse effect type");
        }

        // Test chase effect with color
        let mut chase_params = HashMap::new();
        chase_params.insert("color".to_string(), Value::String("#ffff00".to_string()));
        chase_params.insert("speed".to_string(), Value::String("1.5".to_string()));

        let chase_effect = LightingEffect::new(
            "chase".to_string(),
            vec!["chase_lights".to_string()],
            chase_params,
        );

        let chase_type = LightingTimeline::create_chase_effect(&chase_effect);
        if let EffectType::Chase { .. } = chase_type {
            // Chase effect should have been created successfully
        } else {
            panic!("Expected Chase effect type");
        }
    }
}
