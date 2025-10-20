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

/// Core effect types for lighting
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum EffectType {
    /// Static effect with fixed parameter values
    Static {
        parameters: HashMap<String, f64>,
        duration: Option<Duration>,
    },

    /// Color cycle effect
    ColorCycle {
        colors: Vec<Color>,
        speed: f64, // cycles per second
        direction: CycleDirection,
    },

    /// Strobe effect
    Strobe {
        frequency: f64, // Hz
        intensity: f64, // 0.0 to 1.0
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
        speed: f64,
        direction: ChaseDirection,
    },

    /// Rainbow effect
    Rainbow {
        speed: f64,
        saturation: f64,
        brightness: f64,
    },

    /// Pulse effect
    Pulse {
        base_level: f64,
        pulse_amplitude: f64,
        frequency: f64,
        duration: Option<Duration>,
    },
}

/// Color representation
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub w: Option<u8>, // White channel for RGBW fixtures
}

#[allow(dead_code)]
impl Color {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, w: None }
    }

    pub fn new_rgbw(r: u8, g: u8, b: u8, w: u8) -> Self {
        Self {
            r,
            g,
            b,
            w: Some(w),
        }
    }

    pub fn from_hsv(h: f64, s: f64, v: f64) -> Self {
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;

        let (r, g, b) = if h < 60.0 {
            (c, x, 0.0)
        } else if h < 120.0 {
            (x, c, 0.0)
        } else if h < 180.0 {
            (0.0, c, x)
        } else if h < 240.0 {
            (0.0, x, c)
        } else if h < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        Self {
            r: ((r + m) * 255.0) as u8,
            g: ((g + m) * 255.0) as u8,
            b: ((b + m) * 255.0) as u8,
            w: None,
        }
    }
}

/// Cycle direction for color cycling effects
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum CycleDirection {
    Forward,
    Backward,
    PingPong,
}

/// Chase pattern for spatial effects
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ChasePattern {
    Linear,
    Snake,
    Random,
    Custom(Vec<usize>), // Custom fixture order
}

/// Chase direction for spatial effects
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
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
#[allow(dead_code)]
pub enum DimmerCurve {
    Linear,
    Exponential,
    Logarithmic,
    Sine,
    Cosine,
    Custom(Vec<f64>), // Custom curve points
}

/// An instance of an effect with timing and targeting information
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EffectInstance {
    pub id: String,
    pub effect_type: EffectType,
    pub target_fixtures: Vec<String>, // Fixture names or group names
    pub priority: u8,                 // Higher priority overrides lower
    pub start_time: Option<Instant>,
    pub duration: Option<Duration>,
    pub fade_in: Option<Duration>,
    pub fade_out: Option<Duration>,
    pub enabled: bool,
}

#[allow(dead_code)]
impl EffectInstance {
    pub fn new(id: String, effect_type: EffectType, target_fixtures: Vec<String>) -> Self {
        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            start_time: None,
            duration: None,
            fade_in: None,
            fade_out: None,
            enabled: true,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_timing(mut self, start_time: Option<Instant>, duration: Option<Duration>) -> Self {
        self.start_time = start_time;
        self.duration = duration;
        self
    }

    pub fn with_fades(mut self, fade_in: Option<Duration>, fade_out: Option<Duration>) -> Self {
        self.fade_in = fade_in;
        self.fade_out = fade_out;
        self
    }
}

/// A step in a chaser sequence
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChaserStep {
    pub effect: EffectInstance,
    pub hold_time: Duration,
    pub transition_time: Duration,
    pub transition_type: TransitionType,
}

/// Transition types between chaser steps
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum TransitionType {
    Snap,      // Instant change
    Fade,      // Smooth transition
    Crossfade, // Overlap with previous step
    Wipe,      // Sequential transition
}

/// A chaser sequence
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Chaser {
    pub id: String,
    pub name: String,
    pub steps: Vec<ChaserStep>,
    pub loop_mode: LoopMode,
    pub direction: ChaserDirection,
    pub speed_multiplier: f64,
    pub enabled: bool,
}

/// Loop modes for chasers
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum LoopMode {
    Once,     // Play once and stop
    Loop,     // Repeat indefinitely
    PingPong, // Forward then backward
    Random,   // Random step order
}

/// Chaser direction
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum ChaserDirection {
    Forward,
    Backward,
    Random,
}

#[allow(dead_code)]
impl Chaser {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            steps: Vec::new(),
            loop_mode: LoopMode::Loop,
            direction: ChaserDirection::Forward,
            speed_multiplier: 1.0,
            enabled: true,
        }
    }

    pub fn add_step(mut self, step: ChaserStep) -> Self {
        self.steps.push(step);
        self
    }

    pub fn with_loop_mode(mut self, loop_mode: LoopMode) -> Self {
        self.loop_mode = loop_mode;
        self
    }

    pub fn with_speed(mut self, speed_multiplier: f64) -> Self {
        self.speed_multiplier = speed_multiplier;
        self
    }
}

/// A running instance of a chaser
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChaserInstance {
    pub chaser: Chaser,
    pub current_step: usize,
    pub step_start_time: Instant,
    pub is_running: bool,
    pub direction: ChaserDirection,
}

/// DMX command for sending to fixtures
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DmxCommand {
    pub universe: u16,
    pub channel: u16,
    pub value: u8,
}

/// Error types for the effects system
#[derive(Debug)]
#[allow(dead_code)]
pub enum EffectError {
    InvalidFixture(String),
    InvalidParameter(String),
    InvalidTiming(String),
    EngineError(String),
}

#[allow(dead_code)]
impl std::fmt::Display for EffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EffectError::InvalidFixture(msg) => write!(f, "Invalid fixture: {}", msg),
            EffectError::InvalidParameter(msg) => write!(f, "Invalid parameter: {}", msg),
            EffectError::InvalidTiming(msg) => write!(f, "Invalid timing: {}", msg),
            EffectError::EngineError(msg) => write!(f, "Engine error: {}", msg),
        }
    }
}

impl std::error::Error for EffectError {}

/// Information about a fixture for the effects engine
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FixtureInfo {
    pub name: String,
    pub universe: u16,
    pub address: u16,
    pub fixture_type: String,
    pub channels: HashMap<String, u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_from_hsv() {
        let red = Color::from_hsv(0.0, 1.0, 1.0);
        assert_eq!(red.r, 255);
        assert_eq!(red.g, 0);
        assert_eq!(red.b, 0);

        let green = Color::from_hsv(120.0, 1.0, 1.0);
        assert_eq!(green.r, 0);
        assert_eq!(green.g, 255);
        assert_eq!(green.b, 0);

        let blue = Color::from_hsv(240.0, 1.0, 1.0);
        assert_eq!(blue.r, 0);
        assert_eq!(blue.g, 0);
        assert_eq!(blue.b, 255);
    }

    #[test]
    fn test_effect_instance_creation() {
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Some(Duration::from_secs(5)),
            },
            vec!["fixture1".to_string(), "fixture2".to_string()],
        );

        assert_eq!(effect.id, "test_effect");
        assert_eq!(effect.target_fixtures.len(), 2);
        assert!(effect.enabled);
    }

    #[test]
    fn test_chaser_creation() {
        let chaser = Chaser::new("test_chaser".to_string(), "Test Chaser".to_string())
            .with_loop_mode(LoopMode::Once)
            .with_speed(1.5);

        assert_eq!(chaser.id, "test_chaser");
        assert_eq!(chaser.name, "Test Chaser");
        assert_eq!(chaser.speed_multiplier, 1.5);
        assert!(matches!(chaser.loop_mode, LoopMode::Once));
    }
}
