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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// An instance of an effect with timing and targeting information
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub id: String,
    pub effect_type: EffectType,
    pub target_fixtures: Vec<String>, // Fixture names or group names
    pub priority: u8,                 // Higher priority overrides lower
    pub layer: EffectLayer,           // Layer for layering system
    pub blend_mode: BlendMode,        // How to blend with other effects
    pub start_time: Option<Instant>,
    pub up_time: Option<Duration>,   // Fade in duration (0% to 100%)
    pub hold_time: Option<Duration>, // Time at full intensity (100%)
    pub down_time: Option<Duration>, // Fade out duration (100% to 0%)
    pub enabled: bool,
}

impl EffectInstance {
    pub fn new(id: String, effect_type: EffectType, target_fixtures: Vec<String>) -> Self {
        // Extract duration from effect_type if available
        let duration = match &effect_type {
            EffectType::Static { duration, .. } => *duration,
            EffectType::Dimmer { duration, .. } => Some(*duration), // Dimmer duration becomes up_time
            EffectType::ColorCycle { .. } => Some(Duration::from_secs(60)), // Default 60s for cycling effects
            EffectType::Strobe { duration, .. } => *duration,
            EffectType::Chase { .. } => Some(Duration::from_secs(60)), // Default 60s for chase effects
            EffectType::Rainbow { .. } => Some(Duration::from_secs(60)), // Default 60s for rainbow effects
            EffectType::Pulse { duration, .. } => *duration,
        };

        // Determine timing based on effect type
        let (up_time, hold_time) = match &effect_type {
            EffectType::Dimmer { .. } => (duration, Some(Duration::from_secs(60))), // Dimmer duration becomes up_time, add long hold_time
            EffectType::Static { duration: None, .. } => (None, Some(Duration::from_secs(60))), // Static effects with no duration get long hold_time
            _ => (None, duration.or(Some(Duration::from_secs(1)))), // Default to 1 second if no duration
        };

        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            layer: EffectLayer::Background,
            blend_mode: BlendMode::Replace,
            start_time: None,
            up_time,
            hold_time,
            down_time: None,
            enabled: true,
        }
    }

    #[cfg(test)]
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    #[cfg(test)]
    pub fn with_timing(mut self, start_time: Option<Instant>, hold_time: Option<Duration>) -> Self {
        self.start_time = start_time;
        self.hold_time = hold_time;
        self
    }

    /// Calculate the crossfade multiplier for this effect at the given elapsed time
    pub fn calculate_crossfade_multiplier(&self, elapsed: Duration) -> f64 {
        // elapsed is the time since the effect started

        let up_time = self.up_time.unwrap_or(Duration::from_secs(0));
        let hold_time = self.hold_time.unwrap_or(Duration::from_secs(0));
        let down_time = self.down_time.unwrap_or(Duration::from_secs(0));

        let up_end = up_time;
        let hold_end = up_time + hold_time;
        let total_end = up_time + hold_time + down_time;

        if elapsed < up_end {
            // Fade in phase (0% to 100%)
            if up_time.is_zero() {
                1.0
            } else {
                elapsed.as_secs_f64() / up_time.as_secs_f64()
            }
        } else if elapsed < hold_end {
            // Hold phase (100%)
            1.0
        } else if elapsed < total_end {
            // Fade out phase (100% to 0%)
            if down_time.is_zero() {
                0.0
            } else {
                let fade_out_elapsed = elapsed - hold_end;
                1.0 - (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64())
            }
        } else {
            // Effect has ended
            0.0
        }
    }

    /// Get the total duration of this effect (up_time + hold_time + down_time)
    pub fn total_duration(&self) -> Duration {
        self.up_time.unwrap_or(Duration::from_secs(0))
            + self.hold_time.unwrap_or(Duration::from_secs(0))
            + self.down_time.unwrap_or(Duration::from_secs(0))
    }
}

/// DMX command for sending to fixtures
#[derive(Debug, Clone)]
pub struct DmxCommand {
    pub universe: u16,
    pub channel: u16,
    pub value: u8,
}

/// Represents the current state of a fixture channel
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelState {
    pub value: f64, // 0.0 to 1.0
    pub layer: EffectLayer,
    pub blend_mode: BlendMode,
}

impl ChannelState {
    pub fn new(value: f64, layer: EffectLayer, blend_mode: BlendMode) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            layer,
            blend_mode,
        }
    }

    /// Blend this channel state with another
    pub fn blend_with(&self, other: ChannelState) -> ChannelState {
        let blended_value = match other.blend_mode {
            BlendMode::Replace => other.value,
            BlendMode::Multiply => self.value * other.value,
            BlendMode::Add => (self.value + other.value).min(1.0),
            BlendMode::Overlay => {
                if self.value < 0.5 {
                    2.0 * self.value * other.value
                } else {
                    1.0 - 2.0 * (1.0 - self.value) * (1.0 - other.value)
                }
            }
            BlendMode::Screen => 1.0 - (1.0 - self.value) * (1.0 - other.value),
        };

        // Use the higher layer's blend mode for the result
        let result_layer = self.layer.max(other.layer);
        let result_blend_mode = if other.layer >= self.layer {
            other.blend_mode
        } else {
            self.blend_mode
        };

        ChannelState {
            value: blended_value.clamp(0.0, 1.0),
            layer: result_layer,
            blend_mode: result_blend_mode,
        }
    }
}

/// Represents the current state of a fixture
#[derive(Debug, Clone)]
pub struct FixtureState {
    pub fixture_name: String,
    pub channels: HashMap<String, ChannelState>,
}

impl FixtureState {
    pub fn new(fixture_name: String) -> Self {
        Self {
            fixture_name,
            channels: HashMap::new(),
        }
    }

    /// Set a channel value
    pub fn set_channel(&mut self, channel_name: String, state: ChannelState) {
        self.channels.insert(channel_name, state);
    }

    /// Get a channel value, returning default if not set
    #[cfg(test)]
    pub fn get_channel(&self, channel_name: &str) -> Option<&ChannelState> {
        self.channels.get(channel_name)
    }

    /// Blend this fixture state with another
    pub fn blend_with(&mut self, other: &FixtureState) {
        // Check if the other state has a dimmer multiplier
        if let Some(dimmer_multiplier) = other.channels.get("_dimmer_multiplier") {
            // Apply the dimmer multiplier to all existing RGB channels
            for channel_name in &["red", "green", "blue"] {
                if let Some(self_state) = self.channels.get(*channel_name) {
                    let dimmed_value = self_state.value * dimmer_multiplier.value;

                    // Apply the dimmer multiplier to this channel
                    let dimmed_state =
                        ChannelState::new(dimmed_value, self_state.layer, self_state.blend_mode);
                    self.channels.insert(channel_name.to_string(), dimmed_state);
                }
            }
        }

        // Check if the other state has a pulse multiplier
        if let Some(pulse_multiplier) = other.channels.get("_pulse_multiplier") {
            // Apply the pulse multiplier to all existing RGB channels
            for channel_name in &["red", "green", "blue"] {
                if let Some(self_state) = self.channels.get(*channel_name) {
                    let pulsed_value = self_state.value * pulse_multiplier.value;

                    // Apply the pulse multiplier to this channel
                    let pulsed_state =
                        ChannelState::new(pulsed_value, self_state.layer, self_state.blend_mode);
                    self.channels.insert(channel_name.to_string(), pulsed_state);
                }
            }
        }

        // Blend other channels normally
        for (channel_name, other_state) in &other.channels {
            if channel_name == "_dimmer_multiplier" || channel_name == "_pulse_multiplier" {
                // Skip the multiplier channels - we already handled them above
                continue;
            }

            if let Some(self_state) = self.channels.get(channel_name) {
                // Blend existing channel
                let original_value = self_state.value;
                let blended_state = self_state.blend_with(*other_state);
                let blended_value = blended_state.value;

                self.channels.insert(channel_name.clone(), blended_state);

                tracing::debug!(
                    fixture_name = %self.fixture_name,
                    channel = channel_name,
                    original_value = original_value,
                    other_value = other_state.value,
                    blended_value = blended_value,
                    original_dmx = (original_value * 255.0) as u8,
                    other_dmx = (other_state.value * 255.0) as u8,
                    blended_dmx = (blended_value * 255.0) as u8,
                    "Blended existing channel"
                );
            } else {
                // Add new channel
                self.channels.insert(channel_name.clone(), *other_state);

                tracing::debug!(
                    fixture_name = %self.fixture_name,
                    channel = channel_name,
                    value = other_state.value,
                    dmx_value = (other_state.value * 255.0) as u8,
                    "Added new channel"
                );
            }
        }

        // Debug: Print final state
        tracing::debug!(
            fixture_name = %self.fixture_name,
            final_channels = ?self.channels.iter().map(|(k, v)| (k, v.value)).collect::<Vec<_>>(),
            "Final fixture state after blending"
        );
    }

    /// Convert to DMX commands
    pub fn to_dmx_commands(&self, fixture_info: &FixtureInfo) -> Vec<DmxCommand> {
        let mut commands = Vec::new();

        // Converting fixture state to DMX commands

        for (channel_name, state) in &self.channels {
            if let Some(&channel_offset) = fixture_info.channels.get(channel_name) {
                let dmx_channel = fixture_info.address + channel_offset - 1;
                let dmx_value = (state.value * 255.0) as u8;

                commands.push(DmxCommand {
                    universe: fixture_info.universe,
                    channel: dmx_channel,
                    value: dmx_value,
                });

                // DMX channel calculation: fixture_addr + channel_offset - 1
            }
        }

        // Generated DMX commands for fixture

        commands
    }
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
    pub max_strobe_frequency: Option<f64>, // Maximum strobe frequency in Hz
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
            max_strobe_frequency: None, // RGB_Par doesn't have strobe
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
            max_strobe_frequency: Some(20.0), // Test strobe fixture max frequency
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
            max_strobe_frequency: Some(15.0), // Moving head max strobe frequency
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
