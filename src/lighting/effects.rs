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
    /// Determine if this effect is permanent (state-changing) or temporary (show effect)
    pub fn is_permanent(&self) -> bool {
        match &self.effect_type {
            EffectType::Static { duration, .. } => {
                // Static effects are permanent only if they have no duration AND no timing parameters
                duration.is_none()
                    && self.up_time.is_none()
                    && self.hold_time.is_none()
                    && self.down_time.is_none()
            }
            // Dimmer effects are always permanent so their resulting brightness persists
            EffectType::Dimmer { .. } => true,
            EffectType::ColorCycle { .. } => false, // Cycles complete and end
            EffectType::Strobe { .. } => false,     // Strobe completes and end
            EffectType::Chase { .. } => false,      // Chases complete and end
            EffectType::Rainbow { .. } => false,    // Rainbow cycles complete and end
            EffectType::Pulse { .. } => false,      // Pulse cycles complete and end
        }
    }

    pub fn new(
        id: String,
        effect_type: EffectType,
        target_fixtures: Vec<String>,
        up_time: Option<Duration>,
        hold_time: Option<Duration>,
        down_time: Option<Duration>,
    ) -> Self {
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

        // Determine timing based on effect type, but allow override from parameters
        let (default_up_time, default_hold_time, default_down_time) = match &effect_type {
            EffectType::Dimmer { .. } => (None, None, None), // Dimmer uses its duration field
            EffectType::Static { duration: None, .. } => {
                // If timing parameters are provided, treat as timed effect
                if up_time.is_some() || hold_time.is_some() || down_time.is_some() {
                    // Use provided timing parameters for timed static effect
                    (up_time, hold_time, down_time)
                } else {
                    (None, None, None) // Truly indefinite static effect
                }
            }
            EffectType::Static {
                duration: Some(_), ..
            } => (None, duration, None), // Static effects with duration just hold for that duration
            _ => (None, duration.or(Some(Duration::from_secs(1))), None), // Default to 1 second if no duration
        };

        // Use provided timing or fall back to defaults
        let final_up_time = up_time.or(default_up_time);
        let final_hold_time = hold_time.or(default_hold_time);
        let final_down_time = down_time.or(default_down_time);

        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            layer: EffectLayer::Background,
            blend_mode: BlendMode::Replace,
            start_time: None,
            up_time: final_up_time,
            hold_time: final_hold_time,
            down_time: final_down_time,
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

        // Small epsilon to make boundary checks inclusive and avoid flapping
        let eps = Duration::from_micros(1);

        // Check if this is an indefinite effect (no hold_time and no down_time)
        let is_indefinite = hold_time.is_zero() && down_time.is_zero();

        if up_time.is_zero() {
            // No fade in phase - go directly to hold or fade out
            if is_indefinite {
                // Indefinite effect (like static effects) - always at full intensity
                1.0
            } else if elapsed <= hold_end + eps {
                // Hold phase (100%)
                1.0
            } else if elapsed < total_end + eps {
                // Fade out phase (100% to 0%)
                if down_time.is_zero() {
                    0.0
                } else {
                    let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                    let t = if down_time.is_zero() {
                        1.0
                    } else {
                        (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0)
                    };
                    1.0 - t
                }
            } else {
                // Effect has ended
                0.0
            }
        } else if elapsed < up_end + eps {
            // Fade in phase (0% to 100%)
            (elapsed.as_secs_f64() / up_time.as_secs_f64()).clamp(0.0, 1.0)
        } else if is_indefinite {
            // Indefinite effect after fade-in - always at full intensity
            1.0
        } else if elapsed <= hold_end + eps {
            // Hold phase (100%)
            1.0
        } else if elapsed < total_end + eps {
            // Fade out phase (100% to 0%)
            if down_time.is_zero() {
                0.0
            } else {
                let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                let t = (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0);
                1.0 - t
            }
        } else {
            // Effect has ended
            0.0
        }
    }

    /// Get the total duration of this effect (up_time + hold_time + down_time)
    /// Returns None for indefinite effects (like static effects with no duration)
    pub fn total_duration(&self) -> Option<Duration> {
        // Static effects with no duration are indefinite only if they also have no timing parameters
        if matches!(self.effect_type, EffectType::Static { duration: None, .. })
            && self.up_time.is_none()
            && self.hold_time.is_none()
            && self.down_time.is_none()
        {
            return None; // Truly indefinite static effect
        }

        // For dimmers, use duration field (timing params not used)
        if let EffectType::Dimmer { duration, .. } = &self.effect_type {
            return Some(*duration);
        }

        let duration = self.up_time.unwrap_or(Duration::from_secs(0))
            + self.hold_time.unwrap_or(Duration::from_secs(0))
            + self.down_time.unwrap_or(Duration::from_secs(0));

        Some(duration)
    }

    /// Determine if the effect has reached its intended terminal state for the given elapsed time
    /// This prefers value-based completion when applicable (e.g., dimmer hitting end level).
    pub fn has_reached_terminal_state(&self, elapsed: Duration) -> bool {
        let eps = Duration::from_micros(1);
        let value_eps = 0.0; // require exact target value for termination
        match &self.effect_type {
            EffectType::Dimmer { duration, start_level, end_level, .. } => {
                // Dimmer effect completes when end_level is reached
                if duration.is_zero() {
                    return true; // Instant transition
                }

                // Terminal when we've reached end_level
                let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
                let value = start_level + (end_level - start_level) * progress;
                (value - *end_level).abs() <= value_eps
            }
            EffectType::Static { duration, .. } => {
                if let Some(d) = duration {
                    return elapsed + eps >= *d;
                }
                false
            }
            EffectType::Strobe { duration, .. } => duration.map(|d| elapsed + eps >= d).unwrap_or(false)
            ,
            EffectType::Pulse { duration, .. } => duration.map(|d| elapsed + eps >= d).unwrap_or(false)
            ,
            // Cycle-like effects terminate at configured duration
            EffectType::ColorCycle { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
            EffectType::Chase { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
            EffectType::Rainbow { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
        }
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
    pub channels: HashMap<String, ChannelState>,
}

impl FixtureState {
    pub fn new() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }

    /// Set a channel value
    pub fn set_channel(&mut self, channel_name: String, state: ChannelState) {
        self.channels.insert(channel_name, state);
    }

    /// Blend this fixture state with another
    pub fn blend_with(&mut self, other: &FixtureState) {
        // Blend other channels normally
        for (channel_name, other_state) in &other.channels {
            // For per-layer multiplier channels, overwrite (last-writer-wins) to avoid compounding across frames
            if channel_name.starts_with("_dimmer_mult") || channel_name.starts_with("_pulse_mult") {
                self.channels.insert(channel_name.clone(), *other_state);
                continue;
            }

            if let Some(self_state) = self.channels.get(channel_name) {
                // Blend existing channel
                let blended_state = self_state.blend_with(*other_state);

                self.channels.insert(channel_name.clone(), blended_state);
            } else {
                // Add new channel
                self.channels.insert(channel_name.clone(), *other_state);
            }
        }
    }

    /// Convert to DMX commands
    pub fn to_dmx_commands(&self, fixture_info: &FixtureInfo) -> Vec<DmxCommand> {
        let mut commands = Vec::new();

        // Apply per-layer multipliers for RGB-only fixtures at emission time
        // Combine multipliers from all layers
        let read = |k: &str| self.channels.get(k).map(|c| c.value).unwrap_or(1.0);
        let dimmer_multiplier =
            read("_dimmer_mult_bg") * read("_dimmer_mult_mid") * read("_dimmer_mult_fg");
        let pulse_multiplier =
            read("_pulse_mult_bg") * read("_pulse_mult_mid") * read("_pulse_mult_fg");
        let combined_multiplier = (dimmer_multiplier * pulse_multiplier).clamp(0.0, 1.0);
        let fg_multiplier = (read("_dimmer_mult_fg") * read("_pulse_mult_fg")).clamp(0.0, 1.0);
        let has_dedicated_dimmer = fixture_info.channels.contains_key("dimmer");

        for (channel_name, state) in &self.channels {
            if let Some(&channel_offset) = fixture_info.channels.get(channel_name) {
                let dmx_channel = fixture_info.address + channel_offset - 1;
                let mut value = state.value;
                // If this is an RGB-only fixture, multiply RGB outputs by persisted multipliers
                // except when this channel is a Replace (e.g., foreground Replace should not be dim-scaled)
                if !has_dedicated_dimmer
                    && (channel_name == "red" || channel_name == "green" || channel_name == "blue")
                {
                    let effective_multiplier = if state.layer == EffectLayer::Foreground
                        && state.blend_mode == BlendMode::Replace
                    {
                        fg_multiplier
                    } else {
                        combined_multiplier
                    };
                    if effective_multiplier != 1.0 {
                        value = (value * effective_multiplier).clamp(0.0, 1.0);
                    }
                }
                let dmx_value = (value * 255.0) as u8;

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

/// Strategies for handling brightness control on different fixture types
///
/// These strategies ensure that dimming operations preserve color information
/// and produce visually consistent results across different fixture types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BrightnessStrategy {
    /// Use dedicated dimmer channel (RGB+dimmer fixtures)
    ///
    /// This strategy is used for fixtures that have a dedicated dimmer channel.
    /// The dimmer channel controls overall brightness while RGB channels maintain
    /// their color information, ensuring color is preserved during dimming.
    DedicatedDimmer,
    /// Multiply RGB channels to preserve color (RGB-only fixtures)
    ///
    /// This strategy is used for RGB-only fixtures without a dedicated dimmer.
    /// Instead of directly setting RGB values, a multiplier is applied during
    /// the blending process to preserve the existing color while controlling brightness.
    RgbMultiplication,
}

/// Strategies for handling color control
///
/// These strategies define how color information is applied to different fixture types,
/// supporting various color spaces and mixing methods.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorStrategy {
    /// Use RGB channels for color mixing
    ///
    /// This is the most common strategy, using red, green, and blue channels
    /// to create colors through additive mixing.
    Rgb,
}

/// Strategies for handling strobe effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrobeStrategy {
    /// Use dedicated strobe channel
    DedicatedChannel,
    /// Strobe RGB channels (software strobing)
    RgbStrobing,
    /// Strobe brightness control
    BrightnessStrobing,
}

/// Strategies for handling pulse effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PulseStrategy {
    /// Use dedicated dimmer channel (RGB+dimmer fixtures)
    DedicatedDimmer,
    /// Multiply RGB channels to preserve color (RGB-only fixtures)
    RgbMultiplication,
}

/// Strategies for handling chase effects
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChaseStrategy {
    /// Use dedicated dimmer channel (RGB+dimmer fixtures)
    DedicatedDimmer,
    /// Use RGB channels directly (RGB-only fixtures)
    RgbChannels,
    /// Use brightness control (fallback)
    BrightnessControl,
}

/// Fixture profile that defines how to achieve common lighting operations
///
/// This struct encapsulates the strategies used by different fixture types to perform
/// common lighting operations like brightness control, color mixing, strobing, etc.
/// The profile is automatically determined based on the fixture's capabilities,
/// ensuring that the same lighting show produces visually consistent results
/// across different fixture types.
#[derive(Debug, Clone)]
pub struct FixtureProfile {
    /// Strategy for controlling brightness/dimming
    pub brightness_strategy: BrightnessStrategy,
    /// Strategy for controlling color mixing
    pub color_strategy: ColorStrategy,
    /// Strategy for controlling strobing effects
    pub strobe_strategy: StrobeStrategy,
    /// Strategy for controlling pulsing effects
    pub pulse_strategy: PulseStrategy,
    /// Strategy for controlling chase effects
    pub chase_strategy: ChaseStrategy,
}

impl FixtureProfile {
    /// Create a fixture profile based on fixture capabilities
    ///
    /// This method analyzes the fixture's capabilities and selects the most appropriate
    /// strategies for each type of lighting operation. The selection prioritizes:
    /// 1. Dedicated channels when available (better performance and control)
    /// 2. Color-preserving methods for RGB operations
    /// 3. Fallback strategies for basic functionality
    pub fn for_fixture(fixture: &FixtureInfo) -> Self {
        let capabilities = fixture.capabilities();

        // Determine strategies based on fixture capabilities
        let brightness_strategy = Self::determine_brightness_strategy(&capabilities);
        let color_strategy = Self::determine_color_strategy(&capabilities);
        let strobe_strategy = Self::determine_strobe_strategy(&capabilities);
        let pulse_strategy = Self::determine_pulse_strategy(&capabilities);
        let chase_strategy = Self::determine_chase_strategy(&capabilities);

        FixtureProfile {
            brightness_strategy,
            color_strategy,
            strobe_strategy,
            pulse_strategy,
            chase_strategy,
        }
    }

    /// Determine the best brightness strategy for the given capabilities
    fn determine_brightness_strategy(capabilities: &FixtureCapabilities) -> BrightnessStrategy {
        if capabilities.contains(FixtureCapabilities::DIMMING) {
            BrightnessStrategy::DedicatedDimmer
        } else {
            // Always use multiplication for RGB-only fixtures to preserve color
            BrightnessStrategy::RgbMultiplication
        }
    }

    /// Determine the best color strategy for the given capabilities
    fn determine_color_strategy(_capabilities: &FixtureCapabilities) -> ColorStrategy {
        // Currently only RGB is supported, but this is where HSV/CMY detection would go
        ColorStrategy::Rgb
    }

    /// Determine the best strobe strategy for the given capabilities
    fn determine_strobe_strategy(capabilities: &FixtureCapabilities) -> StrobeStrategy {
        if capabilities.contains(FixtureCapabilities::STROBING) {
            StrobeStrategy::DedicatedChannel
        } else if capabilities.contains(FixtureCapabilities::DIMMING) {
            // Use dimmer channel for strobing when available
            StrobeStrategy::BrightnessStrobing
        } else if capabilities.contains(FixtureCapabilities::RGB_COLOR) {
            // Use RGB channels for software strobing
            StrobeStrategy::RgbStrobing
        } else {
            // Fallback to brightness strobing
            StrobeStrategy::BrightnessStrobing
        }
    }

    /// Determine the best pulse strategy for the given capabilities
    fn determine_pulse_strategy(capabilities: &FixtureCapabilities) -> PulseStrategy {
        if capabilities.contains(FixtureCapabilities::DIMMING) {
            PulseStrategy::DedicatedDimmer
        } else {
            // Always use multiplication for RGB-only fixtures to preserve color
            PulseStrategy::RgbMultiplication
        }
    }

    /// Determine the best chase strategy for the given capabilities
    fn determine_chase_strategy(capabilities: &FixtureCapabilities) -> ChaseStrategy {
        if capabilities.contains(FixtureCapabilities::DIMMING) {
            ChaseStrategy::DedicatedDimmer
        } else if capabilities.contains(FixtureCapabilities::RGB_COLOR) {
            ChaseStrategy::RgbChannels
        } else {
            ChaseStrategy::BrightnessControl
        }
    }

    /// Apply brightness control using the fixture's strategy
    pub fn apply_brightness(
        &self,
        level: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        let mut result = HashMap::new();

        // The conceptual dimmer effect should behave identically regardless of fixture type
        // The fixture type only determines the implementation strategy, not the behavior
        match self.brightness_strategy {
            BrightnessStrategy::DedicatedDimmer => {
                // For fixtures with dedicated dimmer channels, always use the dimmer channel
                // The blend mode controls how it interacts with other effects
                result.insert(
                    "dimmer".to_string(),
                    ChannelState::new(level, layer, blend_mode),
                );
            }
            BrightnessStrategy::RgbMultiplication => {
                // For RGB-only fixtures, always use RGB multiplication
                // This ensures consistent behavior regardless of blend mode
                let key = match layer {
                    EffectLayer::Background => "_dimmer_mult_bg",
                    EffectLayer::Midground => "_dimmer_mult_mid",
                    EffectLayer::Foreground => "_dimmer_mult_fg",
                };
                result.insert(
                    key.to_string(),
                    ChannelState::new(level, layer, BlendMode::Multiply),
                );
            }
        }

        result
    }

    /// Apply color control using the fixture's strategy
    pub fn apply_color(
        &self,
        color: Color,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        let mut result = HashMap::new();

        match self.color_strategy {
            ColorStrategy::Rgb => {
                result.insert(
                    "red".to_string(),
                    ChannelState::new(color.r as f64 / 255.0, layer, blend_mode),
                );
                result.insert(
                    "green".to_string(),
                    ChannelState::new(color.g as f64 / 255.0, layer, blend_mode),
                );
                result.insert(
                    "blue".to_string(),
                    ChannelState::new(color.b as f64 / 255.0, layer, blend_mode),
                );

                // Add white channel if present in color
                if let Some(w) = color.w {
                    result.insert(
                        "white".to_string(),
                        ChannelState::new(w as f64 / 255.0, layer, blend_mode),
                    );
                }
            }
        }

        result
    }

    /// Apply strobe control using the fixture's strategy
    pub fn apply_strobe(
        &self,
        frequency: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
        crossfade_multiplier: f64,
        strobe_value: Option<f64>, // For software strobing
    ) -> HashMap<String, ChannelState> {
        let mut result = HashMap::new();

        // Only apply strobe if crossfade multiplier is > 0 (effect is active)
        if crossfade_multiplier <= 0.0 {
            return result;
        }

        match self.strobe_strategy {
            StrobeStrategy::DedicatedChannel => {
                // Hardware strobe: send normalized speed value to dedicated strobe channel
                // Note: frequency normalization should be done by the caller
                result.insert(
                    "strobe".to_string(),
                    ChannelState::new(frequency, layer, blend_mode),
                );
            }
            StrobeStrategy::RgbStrobing => {
                // Software strobing: use the provided strobe value
                if let Some(value) = strobe_value {
                    // When strobe is OFF (0), use Replace blend mode to override background
                    // When strobe is ON (1), use the original blend mode for layering
                    let effective_blend_mode = if value == 0.0 {
                        BlendMode::Replace
                    } else {
                        blend_mode
                    };

                    let channel_state = ChannelState::new(value, layer, effective_blend_mode);
                    result.insert("red".to_string(), channel_state);
                    result.insert("green".to_string(), channel_state);
                    result.insert("blue".to_string(), channel_state);
                }
            }
            StrobeStrategy::BrightnessStrobing => {
                // Strobe brightness control
                if let Some(value) = strobe_value {
                    let effective_blend_mode = if value == 0.0 {
                        BlendMode::Replace
                    } else {
                        blend_mode
                    };

                    let channel_state = ChannelState::new(value, layer, effective_blend_mode);
                    result.insert("dimmer".to_string(), channel_state);
                }
            }
        }

        result
    }

    /// Apply pulse control using the fixture's strategy
    pub fn apply_pulse(
        &self,
        pulse_value: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        let mut result = HashMap::new();

        match self.pulse_strategy {
            PulseStrategy::DedicatedDimmer => {
                // Use dedicated dimmer channel
                result.insert(
                    "dimmer".to_string(),
                    ChannelState::new(pulse_value, layer, blend_mode),
                );
            }
            PulseStrategy::RgbMultiplication => {
                // Use RGB multiplication (preserves color)
                // Store as multiplier for blending system to apply to existing channels
                let key = match layer {
                    EffectLayer::Background => "_pulse_mult_bg",
                    EffectLayer::Midground => "_pulse_mult_mid",
                    EffectLayer::Foreground => "_pulse_mult_fg",
                };
                result.insert(
                    key.to_string(),
                    ChannelState::new(pulse_value, layer, BlendMode::Multiply),
                );
            }
        }

        result
    }

    /// Apply chase control using the fixture's strategy
    pub fn apply_chase(
        &self,
        chase_value: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        let mut result = HashMap::new();

        match self.chase_strategy {
            ChaseStrategy::DedicatedDimmer => {
                // Use dedicated dimmer channel
                result.insert(
                    "dimmer".to_string(),
                    ChannelState::new(chase_value, layer, blend_mode),
                );
            }
            ChaseStrategy::RgbChannels => {
                // Use RGB channels directly - set all to same value for white chase
                let channel_state = ChannelState::new(chase_value, layer, blend_mode);
                result.insert("red".to_string(), channel_state);
                result.insert("green".to_string(), channel_state);
                result.insert("blue".to_string(), channel_state);
            }
            ChaseStrategy::BrightnessControl => {
                // Use brightness control (fallback)
                result.insert(
                    "dimmer".to_string(),
                    ChannelState::new(chase_value, layer, blend_mode),
                );
            }
        }

        result
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
            None,
            None,
            None,
        );

        assert_eq!(effect.id, "test_effect");
        assert_eq!(effect.target_fixtures.len(), 2);
        assert!(effect.enabled);
    }
}
