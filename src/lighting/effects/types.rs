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
use std::time::Duration;

use super::color::Color;
use super::tempo_aware::{TempoAwareFrequency, TempoAwareSpeed};

/// Core effect types for lighting
#[derive(Debug, Clone)]
pub enum EffectType {
    /// Static effect with fixed parameter values
    Static {
        parameters: HashMap<String, f64>,
        duration: Option<Duration>,
    },

    /// Color cycle effect
    ColorCycle {
        colors: Vec<Color>,
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        direction: CycleDirection,
        transition: CycleTransition, // how to transition between colors
    },

    /// Strobe effect
    Strobe {
        frequency: TempoAwareFrequency, // Hz (can be tempo-aware)
        duration: Option<Duration>,
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
    },

    /// Rainbow effect
    Rainbow {
        speed: TempoAwareSpeed, // cycles per second (can be tempo-aware)
        saturation: f64,
        brightness: f64,
    },

    /// Pulse effect
    Pulse {
        base_level: f64,
        pulse_amplitude: f64,
        frequency: TempoAwareFrequency, // Hz (can be tempo-aware)
        duration: Option<Duration>,
    },
}

impl EffectType {
    /// Get the duration field from the effect type if it exists
    /// Used in tests to verify duration parsing
    #[cfg(test)]
    pub fn get_duration(&self) -> Option<Duration> {
        match self {
            EffectType::Static { duration, .. } => *duration,
            EffectType::Strobe { duration, .. } => *duration,
            EffectType::Pulse { duration, .. } => *duration,
            EffectType::Dimmer { duration, .. } => Some(*duration),
            EffectType::ColorCycle { .. } => None,
            EffectType::Chase { .. } => None,
            EffectType::Rainbow { .. } => None,
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
