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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub w: Option<u8>, // White channel for RGBW fixtures
}

impl Color {
    #[cfg(test)]
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, w: None }
    }

    #[cfg(test)]
    pub fn from_hex(hex: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Err("Invalid hex color format".into());
        }

        let r = u8::from_str_radix(&hex[0..2], 16)?;
        let g = u8::from_str_radix(&hex[2..4], 16)?;
        let b = u8::from_str_radix(&hex[4..6], 16)?;

        Ok(Color { r, g, b, w: None })
    }

    #[cfg(test)]
    pub fn from_name(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        match name.to_lowercase().as_str() {
            "red" => Ok(Color {
                r: 255,
                g: 0,
                b: 0,
                w: None,
            }),
            "green" => Ok(Color {
                r: 0,
                g: 255,
                b: 0,
                w: None,
            }),
            "blue" => Ok(Color {
                r: 0,
                g: 0,
                b: 255,
                w: None,
            }),
            "white" => Ok(Color {
                r: 255,
                g: 255,
                b: 255,
                w: None,
            }),
            "black" => Ok(Color {
                r: 0,
                g: 0,
                b: 0,
                w: None,
            }),
            "yellow" => Ok(Color {
                r: 255,
                g: 255,
                b: 0,
                w: None,
            }),
            "cyan" => Ok(Color {
                r: 0,
                g: 255,
                b: 255,
                w: None,
            }),
            "magenta" => Ok(Color {
                r: 255,
                g: 0,
                b: 255,
                w: None,
            }),
            "orange" => Ok(Color {
                r: 255,
                g: 165,
                b: 0,
                w: None,
            }),
            "purple" => Ok(Color {
                r: 128,
                g: 0,
                b: 128,
                w: None,
            }),
            _ => Err(format!("Unknown color name: {}", name).into()),
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CycleDirection {
    Forward,
    Backward,
    PingPong,
}

/// Chase pattern for spatial effects
#[derive(Debug, Clone)]
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

/// An instance of an effect with timing and targeting information
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub id: String,
    pub effect_type: EffectType,
    pub target_fixtures: Vec<String>, // Fixture names or group names
    pub priority: u8,                 // Higher priority overrides lower
    pub start_time: Option<Instant>,
    pub duration: Option<Duration>,
    pub enabled: bool,
}

impl EffectInstance {
    pub fn new(id: String, effect_type: EffectType, target_fixtures: Vec<String>) -> Self {
        // Extract duration from effect_type if available
        let duration = match &effect_type {
            EffectType::Static { duration, .. } => *duration,
            EffectType::Strobe { duration, .. } => *duration,
            EffectType::Pulse { duration, .. } => *duration,
            _ => None,
        };

        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            start_time: None,
            duration,
            enabled: true,
        }
    }

    #[cfg(test)]
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    #[cfg(test)]
    pub fn with_timing(mut self, start_time: Option<Instant>, duration: Option<Duration>) -> Self {
        self.start_time = start_time;
        self.duration = duration;
        self
    }
}

/// DMX command for sending to fixtures
#[derive(Debug, Clone)]
pub struct DmxCommand {
    pub universe: u16,
    pub channel: u16,
    pub value: u8,
}

/// Error types for the effects system
#[derive(Debug)]
pub enum EffectError {
    Fixture(String),
    Parameter(String),
    Timing(String),
}

impl std::fmt::Display for EffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EffectError::Fixture(msg) => write!(f, "Invalid fixture: {}", msg),
            EffectError::Parameter(msg) => write!(f, "Invalid parameter: {}", msg),
            EffectError::Timing(msg) => write!(f, "Invalid timing: {}", msg),
        }
    }
}

impl std::error::Error for EffectError {}

/// Bitwise flags for fixture capabilities
/// This allows for fast bitwise operations instead of HashSet lookups
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureCapabilities(u32);

impl FixtureCapabilities {
    /// No capabilities
    pub const NONE: FixtureCapabilities = FixtureCapabilities(0);

    /// RGB color mixing capability
    pub const RGB_COLOR: FixtureCapabilities = FixtureCapabilities(1 << 0);
    /// White color capability
    pub const WHITE_COLOR: FixtureCapabilities = FixtureCapabilities(1 << 1);
    /// Dimming capability
    pub const DIMMING: FixtureCapabilities = FixtureCapabilities(1 << 2);
    /// Strobing capability
    pub const STROBING: FixtureCapabilities = FixtureCapabilities(1 << 3);
    /// Panning capability
    pub const PANNING: FixtureCapabilities = FixtureCapabilities(1 << 4);
    /// Tilting capability
    pub const TILTING: FixtureCapabilities = FixtureCapabilities(1 << 5);
    /// Zooming capability
    pub const ZOOMING: FixtureCapabilities = FixtureCapabilities(1 << 6);
    /// Focusing capability
    pub const FOCUSING: FixtureCapabilities = FixtureCapabilities(1 << 7);
    /// Gobo capability
    pub const GOBO: FixtureCapabilities = FixtureCapabilities(1 << 8);
    /// Color temperature capability
    pub const COLOR_TEMPERATURE: FixtureCapabilities = FixtureCapabilities(1 << 9);
    /// Effects capability
    pub const EFFECTS: FixtureCapabilities = FixtureCapabilities(1 << 10);

    /// Check if this set contains a specific capability
    #[inline]
    pub fn contains(&self, capability: FixtureCapabilities) -> bool {
        (self.0 & capability.0) != 0
    }

    /// Add a capability to this set
    #[inline]
    pub fn with(&self, capability: FixtureCapabilities) -> FixtureCapabilities {
        FixtureCapabilities(self.0 | capability.0)
    }
}

/// Information about a fixture for the effects engine
#[derive(Debug, Clone)]
pub struct FixtureInfo {
    pub name: String,
    pub universe: u16,
    pub address: u16,
    pub fixture_type: String,
    pub channels: HashMap<String, u16>,
}

impl FixtureInfo {
    /// Derive fixture capabilities from available channels
    pub fn capabilities(&self) -> FixtureCapabilities {
        let mut capabilities = FixtureCapabilities::NONE;

        // Check for RGB color capability
        if self.channels.contains_key("red")
            && self.channels.contains_key("green")
            && self.channels.contains_key("blue")
        {
            capabilities = capabilities.with(FixtureCapabilities::RGB_COLOR);
        }

        // Check for white color capability
        if self.channels.contains_key("white") {
            capabilities = capabilities.with(FixtureCapabilities::WHITE_COLOR);
        }

        // Check for dimming capability
        if self.channels.contains_key("dimmer") {
            capabilities = capabilities.with(FixtureCapabilities::DIMMING);
        }

        // Check for strobing capability
        if self.channels.contains_key("strobe") {
            capabilities = capabilities.with(FixtureCapabilities::STROBING);
        }

        // Check for panning capability
        if self.channels.contains_key("pan") {
            capabilities = capabilities.with(FixtureCapabilities::PANNING);
        }

        // Check for tilting capability
        if self.channels.contains_key("tilt") {
            capabilities = capabilities.with(FixtureCapabilities::TILTING);
        }

        // Check for zoom capability
        if self.channels.contains_key("zoom") {
            capabilities = capabilities.with(FixtureCapabilities::ZOOMING);
        }

        // Check for focus capability
        if self.channels.contains_key("focus") {
            capabilities = capabilities.with(FixtureCapabilities::FOCUSING);
        }

        // Check for gobo capability
        if self.channels.contains_key("gobo") {
            capabilities = capabilities.with(FixtureCapabilities::GOBO);
        }

        // Check for color temperature capability
        if self.channels.contains_key("ct") || self.channels.contains_key("color_temp") {
            capabilities = capabilities.with(FixtureCapabilities::COLOR_TEMPERATURE);
        }

        // Check for effects capability
        if self.channels.contains_key("effects")
            || self.channels.contains_key("prism")
            || self.channels.contains_key("frost")
        {
            capabilities = capabilities.with(FixtureCapabilities::EFFECTS);
        }

        capabilities
    }

    /// Check if the fixture has a specific capability
    #[inline]
    pub fn has_capability(&self, capability: FixtureCapabilities) -> bool {
        self.capabilities().contains(capability)
    }
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
    fn test_fixture_capabilities() {
        // Test RGB fixture
        let mut rgb_channels = HashMap::new();
        rgb_channels.insert("red".to_string(), 1);
        rgb_channels.insert("green".to_string(), 2);
        rgb_channels.insert("blue".to_string(), 3);
        rgb_channels.insert("dimmer".to_string(), 4);

        let rgb_fixture = FixtureInfo {
            name: "RGB Fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGB_Par".to_string(),
            channels: rgb_channels,
        };

        assert!(rgb_fixture.has_capability(FixtureCapabilities::RGB_COLOR));
        assert!(rgb_fixture.has_capability(FixtureCapabilities::DIMMING));
        assert!(!rgb_fixture.has_capability(FixtureCapabilities::STROBING));

        // Test strobe fixture
        let mut strobe_channels = HashMap::new();
        strobe_channels.insert("strobe".to_string(), 1);
        strobe_channels.insert("dimmer".to_string(), 2);

        let strobe_fixture = FixtureInfo {
            name: "Strobe Fixture".to_string(),
            universe: 1,
            address: 5,
            fixture_type: "Strobe".to_string(),
            channels: strobe_channels,
        };

        assert!(strobe_fixture.has_capability(FixtureCapabilities::STROBING));
        assert!(strobe_fixture.has_capability(FixtureCapabilities::DIMMING));
        assert!(!strobe_fixture.has_capability(FixtureCapabilities::RGB_COLOR));

        // Test multiple capabilities
        assert!(
            rgb_fixture.has_capability(FixtureCapabilities::RGB_COLOR)
                && rgb_fixture.has_capability(FixtureCapabilities::DIMMING)
        );
        assert!(
            !(strobe_fixture.has_capability(FixtureCapabilities::RGB_COLOR)
                && strobe_fixture.has_capability(FixtureCapabilities::DIMMING))
        );

        // Test bitwise operations
        let capabilities = FixtureCapabilities::RGB_COLOR.with(FixtureCapabilities::DIMMING);
        assert!(capabilities.contains(FixtureCapabilities::RGB_COLOR));
        assert!(capabilities.contains(FixtureCapabilities::DIMMING));
        assert!(!capabilities.contains(FixtureCapabilities::STROBING));
        assert_eq!(capabilities.0.count_ones(), 2);
    }

    #[test]
    fn test_capabilities_performance() {
        // Create a fixture with multiple capabilities
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        channels.insert("dimmer".to_string(), 4);
        channels.insert("strobe".to_string(), 5);
        channels.insert("pan".to_string(), 6);
        channels.insert("tilt".to_string(), 7);

        let fixture = FixtureInfo {
            name: "Multi-Capability Fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "Moving_Head".to_string(),
            channels,
        };

        let capabilities = fixture.capabilities();

        // Test individual capability checks (very fast with bitwise operations)
        assert!(capabilities.contains(FixtureCapabilities::RGB_COLOR));
        assert!(capabilities.contains(FixtureCapabilities::DIMMING));
        assert!(capabilities.contains(FixtureCapabilities::STROBING));
        assert!(capabilities.contains(FixtureCapabilities::PANNING));
        assert!(capabilities.contains(FixtureCapabilities::TILTING));
        assert!(!capabilities.contains(FixtureCapabilities::ZOOMING));

        // Test multiple capability checks (single bitwise operation)
        let _required = FixtureCapabilities::RGB_COLOR
            .with(FixtureCapabilities::DIMMING)
            .with(FixtureCapabilities::STROBING);
        assert!(
            capabilities.contains(FixtureCapabilities::RGB_COLOR)
                && capabilities.contains(FixtureCapabilities::DIMMING)
                && capabilities.contains(FixtureCapabilities::STROBING)
        );

        // Test capability counting
        assert_eq!(capabilities.0.count_ones(), 5);
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
}
