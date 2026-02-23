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

use super::color::Color;
use super::state::ChannelState;
use super::types::{BlendMode, EffectLayer};

/// Get the layer suffix for multiplier channel names
#[inline]
fn layer_suffix(layer: EffectLayer) -> &'static str {
    match layer {
        EffectLayer::Background => "_bg",
        EffectLayer::Midground => "_mid",
        EffectLayer::Foreground => "_fg",
    }
}

/// Build a multiplier channel key (e.g. "_dimmer_mult_bg", "_pulse_mult_fg")
#[inline]
pub fn multiplier_key(prefix: &str, layer: EffectLayer) -> String {
    format!("_{}_mult{}", prefix, layer_suffix(layer))
}

/// Insert RGB channel states into a result map
fn insert_rgb(
    result: &mut HashMap<String, ChannelState>,
    value: f64,
    layer: EffectLayer,
    blend_mode: BlendMode,
) {
    let state = ChannelState::new(value, layer, blend_mode);
    result.insert("red".to_string(), state);
    result.insert("green".to_string(), state);
    result.insert("blue".to_string(), state);
}

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

    /// Get the number of capabilities in this set
    #[cfg(test)]
    #[inline]
    pub fn count(&self) -> u32 {
        self.0.count_ones()
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
    pub fn for_fixture(fixture: &FixtureInfo) -> &FixtureProfile {
        fixture.profile()
    }

    /// Create a fixture profile from pre-computed capabilities
    pub(super) fn from_capabilities(capabilities: &FixtureCapabilities) -> Self {
        let brightness_strategy = Self::determine_brightness_strategy(capabilities);
        let color_strategy = Self::determine_color_strategy(capabilities);
        let strobe_strategy = Self::determine_strobe_strategy(capabilities);
        let pulse_strategy = Self::determine_pulse_strategy(capabilities);
        let chase_strategy = Self::determine_chase_strategy(capabilities);

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

    /// Apply a dimmer-or-multiplier value, using the given strategy.
    /// Shared logic for brightness, pulse, and similar effects that either use a
    /// dedicated dimmer channel or an RGB multiplier channel.
    fn apply_dimmer_or_multiplier(
        strategy_uses_dimmer: bool,
        multiplier_prefix: &str,
        value: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        let mut result = HashMap::new();
        if strategy_uses_dimmer {
            result.insert(
                "dimmer".to_string(),
                ChannelState::new(value, layer, blend_mode),
            );
        } else {
            result.insert(
                multiplier_key(multiplier_prefix, layer),
                ChannelState::new(value, layer, BlendMode::Multiply),
            );
        }
        result
    }

    /// Apply brightness control using the fixture's strategy
    pub fn apply_brightness(
        &self,
        level: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        Self::apply_dimmer_or_multiplier(
            self.brightness_strategy == BrightnessStrategy::DedicatedDimmer,
            "dimmer",
            level,
            layer,
            blend_mode,
        )
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
                // Helper to convert u8 color value to normalized f64
                let normalize = |v: u8| v as f64 / 255.0;

                result.insert(
                    "red".to_string(),
                    ChannelState::new(normalize(color.r), layer, blend_mode),
                );
                result.insert(
                    "green".to_string(),
                    ChannelState::new(normalize(color.g), layer, blend_mode),
                );
                result.insert(
                    "blue".to_string(),
                    ChannelState::new(normalize(color.b), layer, blend_mode),
                );

                // Add white channel if present in color
                if let Some(w) = color.w {
                    result.insert(
                        "white".to_string(),
                        ChannelState::new(normalize(w), layer, blend_mode),
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

                    insert_rgb(&mut result, value, layer, effective_blend_mode);
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
        Self::apply_dimmer_or_multiplier(
            self.pulse_strategy == PulseStrategy::DedicatedDimmer,
            "pulse",
            pulse_value,
            layer,
            blend_mode,
        )
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
            ChaseStrategy::DedicatedDimmer | ChaseStrategy::BrightnessControl => {
                result.insert(
                    "dimmer".to_string(),
                    ChannelState::new(chase_value, layer, blend_mode),
                );
            }
            ChaseStrategy::RgbChannels => {
                insert_rgb(&mut result, chase_value, layer, blend_mode);
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
    pub max_strobe_frequency: Option<f64>,
    pub min_strobe_frequency: Option<f64>,
    pub strobe_dmx_offset: Option<u8>,
    /// Cached capabilities derived from channels (computed once at construction)
    cached_capabilities: FixtureCapabilities,
    /// Cached fixture profile (computed once at construction)
    cached_profile: FixtureProfile,
}

impl FixtureInfo {
    /// Create a new FixtureInfo, computing and caching capabilities and profile.
    pub fn new(
        name: String,
        universe: u16,
        address: u16,
        fixture_type: String,
        channels: HashMap<String, u16>,
        max_strobe_frequency: Option<f64>,
    ) -> Self {
        let capabilities = Self::derive_capabilities(&channels);
        let profile = FixtureProfile::from_capabilities(&capabilities);
        Self {
            name,
            universe,
            address,
            fixture_type,
            channels,
            max_strobe_frequency,
            min_strobe_frequency: None,
            strobe_dmx_offset: None,
            cached_capabilities: capabilities,
            cached_profile: profile,
        }
    }

    /// Derive fixture capabilities from available channels
    fn derive_capabilities(channels: &HashMap<String, u16>) -> FixtureCapabilities {
        let mut capabilities = FixtureCapabilities::NONE;

        // Check for RGB color capability (requires all three)
        if channels.contains_key("red")
            && channels.contains_key("green")
            && channels.contains_key("blue")
        {
            capabilities = capabilities.with(FixtureCapabilities::RGB_COLOR);
        }

        // Single-channel capabilities
        if channels.contains_key("white") {
            capabilities = capabilities.with(FixtureCapabilities::WHITE_COLOR);
        }
        if channels.contains_key("dimmer") {
            capabilities = capabilities.with(FixtureCapabilities::DIMMING);
        }
        if channels.contains_key("strobe") {
            capabilities = capabilities.with(FixtureCapabilities::STROBING);
        }
        if channels.contains_key("pan") {
            capabilities = capabilities.with(FixtureCapabilities::PANNING);
        }
        if channels.contains_key("tilt") {
            capabilities = capabilities.with(FixtureCapabilities::TILTING);
        }
        if channels.contains_key("zoom") {
            capabilities = capabilities.with(FixtureCapabilities::ZOOMING);
        }
        if channels.contains_key("focus") {
            capabilities = capabilities.with(FixtureCapabilities::FOCUSING);
        }
        if channels.contains_key("gobo") {
            capabilities = capabilities.with(FixtureCapabilities::GOBO);
        }

        // Multi-channel capabilities
        if channels.contains_key("ct") || channels.contains_key("color_temp") {
            capabilities = capabilities.with(FixtureCapabilities::COLOR_TEMPERATURE);
        }

        if channels.contains_key("effects")
            || channels.contains_key("prism")
            || channels.contains_key("frost")
        {
            capabilities = capabilities.with(FixtureCapabilities::EFFECTS);
        }

        capabilities
    }

    /// Get cached capabilities
    #[cfg(test)]
    #[inline]
    pub fn capabilities(&self) -> FixtureCapabilities {
        self.cached_capabilities
    }

    /// Check if the fixture has a specific capability
    #[inline]
    pub fn has_capability(&self, capability: FixtureCapabilities) -> bool {
        self.cached_capabilities.contains(capability)
    }

    /// Get the cached fixture profile
    #[inline]
    pub fn profile(&self) -> &FixtureProfile {
        &self.cached_profile
    }
}
