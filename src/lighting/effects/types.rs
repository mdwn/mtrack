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

use super::color::Color;
use super::tempo_aware::{TempoAwareFrequency, TempoAwareSpeed};

/// Core effect types for lighting.
/// All effects have an explicit, finite duration.
#[derive(Debug, Clone)]
pub enum EffectType {
    /// Static effect with fixed parameter values
    Static {
        parameters: HashMap<String, f64>,
        duration: Duration,
    },

    /// Color cycle effect
    ColorCycle {
        colors: Vec<Color>,
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        direction: CycleDirection,
        transition: CycleTransition, // how to transition between colors
        duration: Duration,
    },

    /// Strobe effect
    Strobe {
        frequency: TempoAwareFrequency, // Hz (can be tempo-aware)
        duration: Duration,
    },

    /// Dimmer effect with smooth transitions
    Dimmer {
        start_level: f64,
        end_level: f64,
        duration: Duration,
        curve: DimmerCurve,
    },

    /// Chase effect that moves across fixtures
    Chase {
        pattern: ChasePattern,
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        direction: ChaseDirection,
        transition: CycleTransition, // how to transition between fixtures (fade in/out)
        duration: Duration,
    },

    /// Rainbow effect
    Rainbow {
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        saturation: f64,
        brightness: f64,
        duration: Duration,
    },

    /// Pulse effect
    Pulse {
        base_level: f64,
        pulse_amplitude: f64,
        frequency: TempoAwareFrequency, // Hz (can be tempo-aware)
        duration: Duration,
    },
}

impl EffectType {
    /// Get the duration field from the effect type
    #[cfg(test)]
    pub fn get_duration(&self) -> Duration {
        match self {
            EffectType::Static { duration, .. }
            | EffectType::Strobe { duration, .. }
            | EffectType::Pulse { duration, .. }
            | EffectType::Dimmer { duration, .. }
            | EffectType::ColorCycle { duration, .. }
            | EffectType::Chase { duration, .. }
            | EffectType::Rainbow { duration, .. } => *duration,
        }
    }
}

/// Cycle direction for color cycling effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleDirection {
    Forward,
    Backward,
    PingPong,
}

/// Transition type for color cycling effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleTransition {
    /// Snap instantly between colors
    Snap,
    /// Fade smoothly between colors
    Fade,
}

/// Chase pattern for spatial effects
#[derive(Debug, Clone, PartialEq)]
pub enum ChasePattern {
    Linear,
    Snake,
    Random,
}

/// Chase direction for spatial effects
#[derive(Debug, Clone, Copy)]
pub enum ChaseDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
    Clockwise,
    CounterClockwise,
}

/// Dimmer curve types
#[derive(Debug, Clone)]
pub enum DimmerCurve {
    Linear,
    Exponential,
    Logarithmic,
    Sine,
    Cosine,
}

/// Effect layer for layering system
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectLayer {
    Background = 0, // Base layer (e.g., static colors)
    Midground = 1,  // Middle layer (e.g., dimmer effects)
    Foreground = 2, // Top layer (e.g., strobe effects)
}

/// Blend mode for combining effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlendMode {
    /// Replace - higher layer completely replaces lower layer
    Replace,
    /// Multiply - multiply values together (good for dimming)
    Multiply,
    /// Add - add values together (good for color mixing)
    Add,
    /// Overlay - overlay effect (good for strobes)
    Overlay,
    /// Screen - screen blend mode
    Screen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_type_static_duration() {
        let effect = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::from_secs(5),
        };
        assert_eq!(effect.get_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_type_strobe_duration() {
        let effect = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::from_millis(500),
        };
        assert_eq!(effect.get_duration(), Duration::from_millis(500));
    }

    #[test]
    fn effect_type_pulse_duration() {
        let effect = EffectType::Pulse {
            base_level: 0.0,
            pulse_amplitude: 1.0,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Duration::from_secs(3),
        };
        assert_eq!(effect.get_duration(), Duration::from_secs(3));
    }

    #[test]
    fn effect_type_dimmer_duration() {
        let effect = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        };
        assert_eq!(effect.get_duration(), Duration::from_secs(2));
    }

    #[test]
    fn effect_type_color_cycle_duration() {
        let effect = EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
            duration: Duration::from_secs(10),
        };
        assert_eq!(effect.get_duration(), Duration::from_secs(10));
    }

    #[test]
    fn effect_type_chase_duration() {
        let effect = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::from_secs(5),
        };
        assert_eq!(effect.get_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_type_rainbow_duration() {
        let effect = EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(0.5),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::from_secs(8),
        };
        assert_eq!(effect.get_duration(), Duration::from_secs(8));
    }

    #[test]
    fn effect_layer_ordering() {
        assert!(EffectLayer::Background < EffectLayer::Midground);
        assert!(EffectLayer::Midground < EffectLayer::Foreground);
    }

    #[test]
    fn cycle_direction_equality() {
        assert_eq!(CycleDirection::Forward, CycleDirection::Forward);
        assert_ne!(CycleDirection::Forward, CycleDirection::Backward);
        assert_ne!(CycleDirection::Backward, CycleDirection::PingPong);
    }

    #[test]
    fn cycle_transition_equality() {
        assert_eq!(CycleTransition::Snap, CycleTransition::Snap);
        assert_ne!(CycleTransition::Snap, CycleTransition::Fade);
    }

    #[test]
    fn blend_mode_equality() {
        assert_eq!(BlendMode::Replace, BlendMode::Replace);
        assert_ne!(BlendMode::Replace, BlendMode::Multiply);
        assert_ne!(BlendMode::Add, BlendMode::Screen);
    }
}
