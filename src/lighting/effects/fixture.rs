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
use super::composite::INTENSITY;
use super::state::ChannelState;
use super::types::{BlendMode, EffectLayer};
use crate::lighting::channels::CMY_OF_RGB;
use crate::lighting::engine::{base_channel, is_position_channel};
use crate::lighting::types::{strobe_function, FunctionStep, PhysicalUnit, StrobeCurve};

/// The one internal contribution a level makes, whatever the fixture's
/// hardware: its layer, blend mode and value, composited with the colour
/// at the end of the frame (design 3.4).
fn intensity(
    value: f64,
    layer: EffectLayer,
    blend_mode: BlendMode,
) -> HashMap<String, ChannelState> {
    HashMap::from([(
        INTENSITY.to_string(),
        ChannelState::new(value, layer, blend_mode),
    )])
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
    /// to create colors through additive mixing. A CMY fixture uses it too:
    /// its flags take RGB's complement at output.
    Rgb,
    /// No colour to mix — a dimmer-only fixture, or one whose colour is a
    /// wheel. Colour effects leave it alone rather than record a colour the
    /// fixture cannot show (which the stage views would then draw).
    None,
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
    /// Multiply RGB channels to preserve color (RGB-only fixtures)
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
    fn determine_color_strategy(capabilities: &FixtureCapabilities) -> ColorStrategy {
        if capabilities.contains(FixtureCapabilities::RGB_COLOR) {
            ColorStrategy::Rgb
        } else {
            ColorStrategy::None
        }
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

    /// Apply brightness control. A level is the same ask on every fixture:
    /// how the hardware takes it is decided when the frame is resolved.
    pub fn apply_brightness(
        &self,
        level: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        intensity(level, layer, blend_mode)
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
            ColorStrategy::None => {}
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
            StrobeStrategy::RgbStrobing | StrobeStrategy::BrightnessStrobing => {
                // Software strobing is a level that flashes whatever colour
                // is beneath it.
                if let Some(value) = strobe_value {
                    // When strobe is OFF (0), use Replace blend mode to override background
                    // When strobe is ON (1), use the original blend mode for layering
                    let effective_blend_mode = if value == 0.0 {
                        BlendMode::Replace
                    } else {
                        blend_mode
                    };

                    result = intensity(value, layer, effective_blend_mode);
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
        intensity(pulse_value, layer, blend_mode)
    }

    /// Apply chase control using the fixture's strategy
    ///
    /// A chase is a moving brightness mask, not a colour. In the gating modes it
    /// is a level, composited with the colour the layers beneath provide (white
    /// when there is none), so it modulates that colour instead of painting
    /// over it.
    ///
    /// The blend mode decides what the mask does to the bed. `multiply` (and
    /// `replace`) gate it: the active fixture keeps its colour and every other
    /// fixture goes dark, the classic running light. `screen` and `add` lift it:
    /// the active fixture brightens toward white and the inactive fixtures keep
    /// the bed untouched, so a chase can ride over a wash without blacking out
    /// the stage. On a fixture with a dimmer the lift is a level too; on one
    /// without, the mask goes into the colour channels and the blend does the
    /// work. `overlay` is treated as `screen` there: with a zero mask it would
    /// darken a dim bed, which no chase means.
    pub fn apply_chase(
        &self,
        chase_value: f64,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> HashMap<String, ChannelState> {
        match (self.chase_strategy, blend_mode) {
            (
                ChaseStrategy::RgbChannels,
                BlendMode::Screen | BlendMode::Add | BlendMode::Overlay,
            ) => {
                let lifting_mode = match blend_mode {
                    BlendMode::Overlay => BlendMode::Screen,
                    other => other,
                };
                let mut result = HashMap::new();
                insert_rgb(&mut result, chase_value, layer, lifting_mode);
                result
            }
            _ => intensity(chase_value, layer, blend_mode),
        }
    }
}

/// Whether a fixture mixes colour subtractively
/// ([`crate::lighting::channels::mixes_subtractively`]).
pub fn has_cmy<V>(channels: &HashMap<String, V>) -> bool {
    crate::lighting::channels::mixes_subtractively(|c| channels.contains_key(c))
}

/// Whether a channel map has a dimmer of its own: a fixture's, or a
/// cell's. Without one, the level is carried on the colour channels.
pub fn has_dimmer_channel<V>(channels: &HashMap<String, V>) -> bool {
    channels.contains_key("dimmer")
}

/// The name a pixel fixture's cell registers under beside it: the
/// fixture's, a slash, the cell's.
pub fn cell_fixture_name(fixture: &str, cell: &str) -> String {
    format!("{fixture}/{cell}")
}

/// The cell's own name in a name [`cell_fixture_name`] built.
pub fn cell_name(sub_fixture: &str) -> &str {
    sub_fixture.rsplit('/').next().unwrap_or(sub_fixture)
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
    /// Where the fixture hangs, in stage coordinates (meters, right-handed
    /// Z-up, origin downstage-center, +x stage-left, +y upstage), when the
    /// venue places it. Chases order themselves by it.
    pub position: Option<[f64; 3]>,
    /// Mounting rotation in degrees about X, Y, Z, when the venue states it.
    pub rotation: Option<[f64; 3]>,
    /// The beam angle the venue states for this unit, in degrees, when its
    /// own differs from the type's. A drawing hint for the 3D view; nothing
    /// in the engine reads it.
    pub beam_angle: Option<f64>,
    /// The structured channel definitions — fine bytes, physical ranges,
    /// function tables — that physical values resolve through. Mirrors
    /// `channels` for a fixture type that declares nothing more.
    pub channel_defs: HashMap<String, crate::lighting::types::ChannelDef>,
    /// Movement limits, when the fixture type declares them.
    pub movement: crate::lighting::types::MovementLimits,
    /// How a strobe rate maps onto the strobe channel's DMX range, resolved
    /// (the type's stated curve, or the automatic one).
    pub strobe_curve: crate::lighting::types::StrobeCurve,
    /// The fixture type's rig model in the asset store, when it has one.
    pub rig: Option<String>,
    /// How the rig's joints sit in the mounting frame (design §18.6);
    /// absent for a fixture aimed by the plain convention.
    pub aim: Option<super::AimCalibration>,
    /// The cells of a pixel fixture (design §17.2), in the manufacturer's
    /// order; empty for a one-colour fixture.
    pub cells: Vec<crate::lighting::types::Cell>,
    /// For a cell registered as a sub-fixture (`parent/cell`): the fixture
    /// it belongs to. The state stream and the plot treat sub-fixtures as
    /// part of their parent.
    pub parent: Option<String>,
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
        let channel_defs = channels
            .iter()
            .map(|(name, &offset)| (name.clone(), crate::lighting::types::ChannelDef::at(offset)))
            .collect();
        Self {
            name,
            universe,
            address,
            fixture_type,
            channels,
            max_strobe_frequency,
            min_strobe_frequency: None,
            strobe_dmx_offset: None,
            position: None,
            rotation: None,
            beam_angle: None,
            channel_defs,
            movement: crate::lighting::types::MovementLimits::default(),
            strobe_curve: crate::lighting::types::StrobeCurve::default(),
            rig: None,
            aim: None,
            cells: Vec::new(),
            parent: None,
            cached_capabilities: capabilities,
            cached_profile: profile,
        }
    }

    /// The geometry the fixture is aimed through: its rig's, or the plain
    /// convention.
    pub fn calibration(&self) -> super::AimCalibration {
        self.aim.unwrap_or(super::AimCalibration::IDENTITY)
    }

    /// Replaces the plain channel definitions with the fixture type's
    /// structured ones (fine bytes, ranges, functions).
    pub fn with_channel_defs(
        mut self,
        channel_defs: HashMap<String, crate::lighting::types::ChannelDef>,
    ) -> Self {
        self.channel_defs = channel_defs;
        self
    }

    /// Derive fixture capabilities from available channels
    fn derive_capabilities(channels: &HashMap<String, u16>) -> FixtureCapabilities {
        let mut capabilities = FixtureCapabilities::NONE;

        // Colour: an RGB triple, or a CMY one (subtractive mixing, written
        // as RGB's complement at output — see `FixtureState::to_dmx_commands`).
        if (channels.contains_key("red")
            && channels.contains_key("green")
            && channels.contains_key("blue"))
            || has_cmy(channels)
        {
            capabilities = capabilities.with(FixtureCapabilities::RGB_COLOR);
        }

        // Single-channel capabilities
        if channels.contains_key("white") {
            capabilities = capabilities.with(FixtureCapabilities::WHITE_COLOR);
        }
        if has_dimmer_channel(channels) {
            capabilities = capabilities.with(FixtureCapabilities::DIMMING);
        }
        if channels.contains_key("strobe") {
            capabilities = capabilities.with(FixtureCapabilities::STROBING);
        }
        // A head moves on any channel that carries where it points: `pan`,
        // a GDTF profile's `pan:1`, a fine byte.
        for channel in channels.keys().filter(|c| is_position_channel(c)) {
            capabilities = capabilities.with(match base_channel(channel) {
                "pan" => FixtureCapabilities::PANNING,
                _ => FixtureCapabilities::TILTING,
            });
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

    /// Whether a named channel value means something to this fixture: one of
    /// its own channels, or — on a CMY fixture — red, green or blue, which
    /// reach its flags as their complement at output.
    pub fn takes_channel(&self, name: &str) -> bool {
        self.channels.contains_key(name)
            || (CMY_OF_RGB.iter().any(|(rgb, _)| *rgb == name) && has_cmy(&self.channels))
    }

    /// The DMX channel (1-based, on the fixture's universe) a channel at
    /// `offset` (1-based from the fixture's address) is patched to; `None`
    /// for one no channel number holds (past 65535), which would wrap.
    #[inline]
    pub fn dmx_channel(&self, offset: u16) -> Option<u16> {
        u16::try_from((u32::from(self.address) + u32::from(offset)).checked_sub(1)?).ok()
    }

    /// Get cached capabilities
    #[inline]
    pub fn capabilities(&self) -> FixtureCapabilities {
        self.cached_capabilities
    }

    /// Check if the fixture has a specific capability
    #[inline]
    pub fn has_capability(&self, capability: FixtureCapabilities) -> bool {
        self.cached_capabilities.contains(capability)
    }

    /// Whether the fixture has a dimmer channel of its own. Without one,
    /// its level is carried on its colour channels.
    #[inline]
    pub fn has_dimmer(&self) -> bool {
        self.has_capability(FixtureCapabilities::DIMMING)
    }

    /// Get the cached fixture profile
    #[inline]
    pub fn profile(&self) -> &FixtureProfile {
        &self.cached_profile
    }

    /// How a strobe rate becomes this fixture's strobe byte: the one rule
    /// the engine writes through ([`StrobeMap::normalized`]) and the state
    /// stream reads back through ([`FixtureInfo::strobe_hz`]). Which rule
    /// is the type's stated curve, or the automatic one, over its strobe
    /// function and range.
    pub fn strobe_map(&self) -> StrobeMap<'_> {
        let max = self.max_strobe_frequency.unwrap_or(20.0);
        let min = self.min_strobe_frequency.unwrap_or(0.0);
        let dmx_offset = self.strobe_dmx_offset.unwrap_or(0);
        let offset = f64::from(dmx_offset) / 255.0;
        let linear = matches!(
            self.strobe_curve,
            StrobeCurve::Linear | StrobeCurve::Declared
        );
        // Linear and declared follow the strobe function itself when there
        // is one: its own table (declared), else one step from its
        // endpoints — in its direction, over its DMX range.
        let function = linear
            .then(|| strobe_function(&self.channel_defs))
            .flatten();
        let endpoints = function.and_then(|f| {
            let hz = f.physical.filter(|p| p.unit == PhysicalUnit::Hertz)?;
            Some(FunctionStep {
                dmx_from: f.dmx_from,
                dmx_to: f.dmx_to,
                from: hz.from,
                to: hz.to,
            })
        });
        match function {
            Some(f) if self.strobe_curve == StrobeCurve::Declared && !f.steps.is_empty() => {
                StrobeMap::Declared(&f.steps)
            }
            _ => match endpoints {
                Some(step) => StrobeMap::Endpoints(step),
                None if linear && max > min => StrobeMap::LinearHz { min, max, offset },
                None if max > 0.0 && min > 0.0 && dmx_offset > 0 => {
                    StrobeMap::Period { min, max, offset }
                }
                None => StrobeMap::Ratio { max, offset },
            },
        }
    }

    /// The rate the fixture's strobe channel flashes at with `byte` on it,
    /// read back through [`Self::strobe_map`]; `None` for a fixture with no
    /// strobe channel, below the map's first byte (the function's "open"
    /// range), outside a declared table's steps, or at byte 0 of a channel
    /// no function declares a rate at (the engine's "strobe off"). A
    /// function that starts at byte 0 declares it a rate, and it reads
    /// back as one: the engine writes byte 0 for that rate too.
    pub fn strobe_hz(&self, byte: u8) -> Option<f64> {
        if !self.has_capability(FixtureCapabilities::STROBING) {
            return None;
        }
        self.strobe_map().hz(byte).filter(|hz| *hz > 0.0)
    }
}

/// A fixture's strobe rule (see [`FixtureInfo::strobe_map`]), with the
/// forward mapping the engine drives the channel through and its inverse.
#[derive(Clone, Debug, PartialEq)]
pub enum StrobeMap<'a> {
    /// `declared` with a table: the function's own steps.
    Declared(&'a [FunctionStep]),
    /// `linear` or `declared` with a strobe function but no table: one
    /// step from the function's endpoints.
    Endpoints(FunctionStep),
    /// `linear` without a function: hertz run linearly over
    /// `offset..=1` of the channel.
    LinearHz { min: f64, max: f64, offset: f64 },
    /// `period`: the strobe period (1/Hz) runs linearly over
    /// `offset..=1` of the channel, as many fixtures map it.
    Period { min: f64, max: f64, offset: f64 },
    /// No range to speak of: the rate as a fraction of the maximum, over
    /// `offset..=1` of the channel (a period curve with no minimum, #633).
    Ratio { max: f64, offset: f64 },
}

impl StrobeMap<'_> {
    /// The steps of a tabled map; empty otherwise.
    fn steps(&self) -> &[FunctionStep] {
        match self {
            StrobeMap::Declared(steps) => steps,
            StrobeMap::Endpoints(step) => std::slice::from_ref(step),
            _ => &[],
        }
    }

    /// The normalized (0..1) value the strobe channel is driven to for
    /// `hz`, which [`dmx_byte`](crate::lighting::channels::dmx_byte) rounds to the byte. A
    /// tabled map lands on its byte exactly.
    pub fn normalized(&self, hz: f64) -> f64 {
        match *self {
            StrobeMap::Declared(_) | StrobeMap::Endpoints(_) => {
                // The step the rate falls in, linear inside it, on its byte.
                let dmx = declared_dmx(self.steps(), hz).round();
                (dmx / 255.0).clamp(0.0, 1.0)
            }
            StrobeMap::LinearHz { min, max, offset } => {
                // Linear in hertz across the function's DMX range, as a
                // GDTF declares it:
                // DMX = start + (hz − min)/(max − min) × (255 − start).
                let fraction = (hz.clamp(min, max) - min) / (max - min);
                crate::lighting::channels::lerp(offset, 1.0, fraction).clamp(0.0, 1.0)
            }
            StrobeMap::Period { min, max, offset } => {
                // Interpolate in period-space: many fixtures map DMX
                // linearly to strobe period (1/freq), not frequency directly.
                let max_period = 1.0 / min;
                let min_period = 1.0 / max;
                let desired_period = 1.0 / hz.clamp(min, max);
                let period_range = max_period - min_period;
                let period_fraction = if period_range.abs() < f64::EPSILON {
                    1.0
                } else {
                    (max_period - desired_period) / period_range
                };
                crate::lighting::channels::lerp(offset, 1.0, period_fraction).clamp(0.0, 1.0)
            }
            StrobeMap::Ratio { max, offset } => {
                // Not a number is full rate, as the map had it before it
                // clamped below.
                let fraction = hz / max;
                let fraction = if fraction.is_nan() {
                    1.0
                } else {
                    fraction.clamp(0.0, 1.0)
                };
                crate::lighting::channels::lerp(offset, 1.0, fraction)
            }
        }
    }

    /// The rate `byte` on the channel means, the inverse of
    /// [`Self::normalized`]: for a tabled map the step's value at the byte
    /// itself (`None` outside every step — byte 0 included, unless a step
    /// starts there); for the others the value at the byte itself, the
    /// centre of the levels that round to it (`None` below the map's first
    /// byte, and at byte 0, which no function declares a rate at: it is
    /// what the engine writes for off).
    pub fn hz(&self, byte: u8) -> Option<f64> {
        let byte = f64::from(byte);
        let from_fraction = |offset: f64| {
            let fraction = (byte / 255.0 - offset) / (1.0 - offset);
            (byte > 0.0 && byte >= offset * 255.0).then_some(fraction.clamp(0.0, 1.0))
        };
        match *self {
            StrobeMap::Declared(_) | StrobeMap::Endpoints(_) => {
                let step = self
                    .steps()
                    .iter()
                    .find(|s| f64::from(s.dmx_from) <= byte && byte <= f64::from(s.dmx_to))?;
                let (dmx_from, dmx_to) = (f64::from(step.dmx_from), f64::from(step.dmx_to));
                if (dmx_to - dmx_from).abs() < f64::EPSILON {
                    return Some(step.from);
                }
                let fraction = (byte - dmx_from) / (dmx_to - dmx_from);
                Some(step.from + fraction * (step.to - step.from))
            }
            StrobeMap::LinearHz { min, max, offset } => {
                Some(min + from_fraction(offset)? * (max - min))
            }
            StrobeMap::Period { min, max, offset } => {
                let max_period = 1.0 / min;
                let min_period = 1.0 / max;
                let period = max_period - from_fraction(offset)? * (max_period - min_period);
                Some(1.0 / period)
            }
            StrobeMap::Ratio { max, offset } => Some(from_fraction(offset)? * max),
        }
    }
}

/// The DMX value a function's step table gives `hz`: the first step in
/// DMX order whose physical range contains it, interpolated inside the
/// step; else the nearest end of any step (so a rate past the table lands
/// on its end). `steps` is non-empty.
pub(crate) fn declared_dmx(steps: &[FunctionStep], hz: f64) -> f64 {
    let at = |step: &FunctionStep, hz: f64| -> f64 {
        let (dmx_from, dmx_to) = (f64::from(step.dmx_from), f64::from(step.dmx_to));
        if (step.to - step.from).abs() < f64::EPSILON {
            return dmx_from;
        }
        let fraction = ((hz - step.from) / (step.to - step.from)).clamp(0.0, 1.0);
        dmx_from + fraction * (dmx_to - dmx_from)
    };
    if let Some(step) = steps
        .iter()
        .find(|s| s.from.min(s.to) <= hz && hz <= s.from.max(s.to))
    {
        return at(step, hz);
    }
    let mut best: Option<(f64, f64)> = None;
    for step in steps {
        for end in [step.from, step.to] {
            let distance = (hz - end).abs();
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, at(step, end)));
            }
        }
    }
    best.map_or(0.0, |(_, dmx)| dmx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channels(names: &[&str]) -> HashMap<String, u16> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_string(), i as u16 + 1))
            .collect()
    }

    fn rgb_fixture() -> FixtureInfo {
        FixtureInfo::new(
            "par".to_string(),
            1,
            1,
            "generic_par".to_string(),
            make_channels(&["red", "green", "blue"]),
            None,
        )
    }

    fn rgb_dimmer_fixture() -> FixtureInfo {
        FixtureInfo::new(
            "par_d".to_string(),
            1,
            1,
            "par_with_dimmer".to_string(),
            make_channels(&["red", "green", "blue", "dimmer"]),
            None,
        )
    }

    fn full_fixture() -> FixtureInfo {
        FixtureInfo::new(
            "mover".to_string(),
            1,
            1,
            "moving_head".to_string(),
            make_channels(&[
                "red", "green", "blue", "white", "dimmer", "strobe", "pan", "tilt",
            ]),
            None,
        )
    }

    #[test]
    fn a_channel_past_65535_has_no_dmx_channel() {
        let mut par = rgb_fixture();
        par.address = 65534;
        assert_eq!(par.dmx_channel(1), Some(65534));
        assert_eq!(par.dmx_channel(2), Some(65535));
        // It wrapped onto channel 0 before.
        assert_eq!(par.dmx_channel(3), None);
    }

    // ── FixtureCapabilities ──────────────────────────────────────────

    #[test]
    fn a_cmy_fixture_has_colour_and_takes_rgb() {
        let cmy = FixtureInfo::new(
            "spot".to_string(),
            1,
            1,
            "cmy_spot".to_string(),
            make_channels(&["dimmer", "cyan", "magenta", "yellow"]),
            None,
        );
        assert!(cmy.has_capability(FixtureCapabilities::RGB_COLOR));
        assert!(cmy.takes_channel("red") && cmy.takes_channel("cyan"));
        assert!(!cmy.takes_channel("white"));
        // Two of three flags is not subtractive mixing.
        let partial = FixtureInfo::new(
            "spot".to_string(),
            1,
            1,
            "partial".to_string(),
            make_channels(&["dimmer", "cyan", "magenta"]),
            None,
        );
        assert!(!partial.has_capability(FixtureCapabilities::RGB_COLOR));
        assert!(!partial.takes_channel("red"));
    }

    #[test]
    fn capabilities_none() {
        assert_eq!(FixtureCapabilities::NONE.count(), 0);
    }

    #[test]
    fn capabilities_single() {
        let caps = FixtureCapabilities::NONE.with(FixtureCapabilities::RGB_COLOR);
        assert!(caps.contains(FixtureCapabilities::RGB_COLOR));
        assert!(!caps.contains(FixtureCapabilities::DIMMING));
        assert_eq!(caps.count(), 1);
    }

    #[test]
    fn capabilities_multiple() {
        let caps = FixtureCapabilities::NONE
            .with(FixtureCapabilities::RGB_COLOR)
            .with(FixtureCapabilities::DIMMING)
            .with(FixtureCapabilities::STROBING);
        assert!(caps.contains(FixtureCapabilities::RGB_COLOR));
        assert!(caps.contains(FixtureCapabilities::DIMMING));
        assert!(caps.contains(FixtureCapabilities::STROBING));
        assert!(!caps.contains(FixtureCapabilities::PANNING));
        assert_eq!(caps.count(), 3);
    }

    #[test]
    fn capabilities_idempotent() {
        let caps = FixtureCapabilities::NONE
            .with(FixtureCapabilities::RGB_COLOR)
            .with(FixtureCapabilities::RGB_COLOR);
        assert_eq!(caps.count(), 1);
    }

    // ── derive_capabilities ──────────────────────────────────────────

    #[test]
    fn derive_capabilities_rgb_only() {
        let f = rgb_fixture();
        assert!(f.has_capability(FixtureCapabilities::RGB_COLOR));
        assert!(!f.has_capability(FixtureCapabilities::DIMMING));
        assert!(!f.has_capability(FixtureCapabilities::STROBING));
    }

    #[test]
    fn derive_capabilities_rgb_plus_dimmer() {
        let f = rgb_dimmer_fixture();
        assert!(f.has_capability(FixtureCapabilities::RGB_COLOR));
        assert!(f.has_capability(FixtureCapabilities::DIMMING));
        assert!(!f.has_capability(FixtureCapabilities::STROBING));
    }

    #[test]
    fn derive_capabilities_full() {
        let f = full_fixture();
        assert!(f.has_capability(FixtureCapabilities::RGB_COLOR));
        assert!(f.has_capability(FixtureCapabilities::WHITE_COLOR));
        assert!(f.has_capability(FixtureCapabilities::DIMMING));
        assert!(f.has_capability(FixtureCapabilities::STROBING));
        assert!(f.has_capability(FixtureCapabilities::PANNING));
        assert!(f.has_capability(FixtureCapabilities::TILTING));
    }

    #[test]
    fn derive_capabilities_requires_all_rgb() {
        // Only red+green (no blue) should NOT get RGB_COLOR
        let f = FixtureInfo::new(
            "partial".to_string(),
            1,
            1,
            "type".to_string(),
            make_channels(&["red", "green"]),
            None,
        );
        assert!(!f.has_capability(FixtureCapabilities::RGB_COLOR));
    }

    #[test]
    fn derive_capabilities_color_temp() {
        let f = FixtureInfo::new(
            "ct_fixture".to_string(),
            1,
            1,
            "type".to_string(),
            make_channels(&["ct"]),
            None,
        );
        assert!(f.has_capability(FixtureCapabilities::COLOR_TEMPERATURE));
    }

    #[test]
    fn derive_capabilities_effects_channels() {
        for channel in &["effects", "prism", "frost"] {
            let f = FixtureInfo::new(
                "fx".to_string(),
                1,
                1,
                "type".to_string(),
                make_channels(&[channel]),
                None,
            );
            assert!(
                f.has_capability(FixtureCapabilities::EFFECTS),
                "failed for {}",
                channel
            );
        }
    }

    // ── FixtureProfile strategy selection ─────────────────────────────

    #[test]
    fn profile_rgb_only_strategies() {
        let p = rgb_fixture().profile().clone();
        assert_eq!(p.brightness_strategy, BrightnessStrategy::RgbMultiplication);
        assert_eq!(p.strobe_strategy, StrobeStrategy::RgbStrobing);
        assert_eq!(p.pulse_strategy, PulseStrategy::RgbMultiplication);
        assert_eq!(p.chase_strategy, ChaseStrategy::RgbChannels);
    }

    #[test]
    fn profile_rgb_dimmer_strategies() {
        let p = rgb_dimmer_fixture().profile().clone();
        assert_eq!(p.brightness_strategy, BrightnessStrategy::DedicatedDimmer);
        assert_eq!(p.strobe_strategy, StrobeStrategy::BrightnessStrobing);
        assert_eq!(p.pulse_strategy, PulseStrategy::DedicatedDimmer);
        assert_eq!(p.chase_strategy, ChaseStrategy::DedicatedDimmer);
    }

    #[test]
    fn profile_full_fixture_strategies() {
        let p = full_fixture().profile().clone();
        assert_eq!(p.brightness_strategy, BrightnessStrategy::DedicatedDimmer);
        assert_eq!(p.strobe_strategy, StrobeStrategy::DedicatedChannel);
        assert_eq!(p.pulse_strategy, PulseStrategy::DedicatedDimmer);
        assert_eq!(p.chase_strategy, ChaseStrategy::DedicatedDimmer);
    }

    #[test]
    fn profile_no_capabilities_fallbacks() {
        let p = FixtureProfile::from_capabilities(&FixtureCapabilities::NONE);
        assert_eq!(p.brightness_strategy, BrightnessStrategy::RgbMultiplication);
        assert_eq!(p.strobe_strategy, StrobeStrategy::BrightnessStrobing);
        assert_eq!(p.pulse_strategy, PulseStrategy::RgbMultiplication);
        assert_eq!(p.chase_strategy, ChaseStrategy::BrightnessControl);
    }

    // ── apply_brightness ─────────────────────────────────────────────

    #[test]
    fn apply_brightness_dedicated_dimmer() {
        let p = rgb_dimmer_fixture().profile().clone();
        let result = p.apply_brightness(0.75, EffectLayer::Background, BlendMode::Replace);
        assert!(!result.contains_key("dimmer"), "a level, not a channel");
        assert!(!result.contains_key("red"));
        assert!((result[INTENSITY].value - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_brightness_is_the_same_level_on_an_rgb_only_fixture() {
        let p = rgb_fixture().profile().clone();
        let result = p.apply_brightness(0.5, EffectLayer::Background, BlendMode::Replace);
        assert!(!result.contains_key("dimmer"));
        // Its blend mode is kept, not turned into a multiply.
        assert_eq!(result[INTENSITY].blend_mode, BlendMode::Replace);
        assert_eq!(result[INTENSITY].layer, EffectLayer::Background);
    }

    // ── apply_color ──────────────────────────────────────────────────

    #[test]
    fn apply_color_rgb() {
        let p = rgb_fixture().profile().clone();
        let color = Color::new(255, 128, 0);
        let result = p.apply_color(color, EffectLayer::Background, BlendMode::Replace);
        assert!((result["red"].value - 1.0).abs() < f64::EPSILON);
        assert!((result["green"].value - 128.0 / 255.0).abs() < 0.01);
        assert!((result["blue"].value - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_color_with_white_channel() {
        let p = rgb_fixture().profile().clone();
        let color = Color {
            r: 255,
            g: 255,
            b: 255,
            w: Some(200),
        };
        let result = p.apply_color(color, EffectLayer::Background, BlendMode::Replace);
        assert!(result.contains_key("white"));
        assert!((result["white"].value - 200.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn strobe_ratio_map_clamps_its_fraction() {
        let map = StrobeMap::Ratio {
            max: 20.0,
            offset: 0.2,
        };
        assert_eq!(map.normalized(-5.0), 0.2);
        assert!((map.normalized(10.0) - 0.6).abs() < 1e-9);
        assert_eq!(map.normalized(40.0), 1.0);
        // Not a number is full rate, as before the fraction was clamped below.
        assert_eq!(map.normalized(f64::NAN), 1.0);
    }

    // ── apply_strobe ─────────────────────────────────────────────────

    #[test]
    fn apply_strobe_dedicated_channel() {
        let p = full_fixture().profile().clone();
        let result = p.apply_strobe(0.5, EffectLayer::Foreground, BlendMode::Replace, 1.0, None);
        assert!(result.contains_key("strobe"));
    }

    #[test]
    fn apply_strobe_zero_crossfade_returns_empty() {
        let p = full_fixture().profile().clone();
        let result = p.apply_strobe(0.5, EffectLayer::Foreground, BlendMode::Replace, 0.0, None);
        assert!(result.is_empty());
    }

    #[test]
    fn apply_strobe_rgb_strobing() {
        let p = rgb_fixture().profile().clone();
        let result = p.apply_strobe(
            0.0,
            EffectLayer::Foreground,
            BlendMode::Replace,
            1.0,
            Some(1.0),
        );
        // A software strobe is a level, so it flashes the colour beneath.
        assert!(!result.contains_key("red"));
        assert_eq!(result[INTENSITY].value, 1.0);
    }

    // ── apply_pulse ──────────────────────────────────────────────────

    #[test]
    fn apply_pulse_dedicated_dimmer() {
        let p = rgb_dimmer_fixture().profile().clone();
        let result = p.apply_pulse(0.8, EffectLayer::Midground, BlendMode::Multiply);
        assert!(result.contains_key(INTENSITY));
    }

    #[test]
    fn apply_pulse_rgb_multiplication() {
        let p = rgb_fixture().profile().clone();
        let result = p.apply_pulse(0.8, EffectLayer::Midground, BlendMode::Multiply);
        assert!(result.contains_key(INTENSITY));
    }

    // ── apply_chase ──────────────────────────────────────────────────

    #[test]
    fn apply_chase_dedicated_dimmer() {
        let p = rgb_dimmer_fixture().profile().clone();
        let result = p.apply_chase(0.5, EffectLayer::Foreground, BlendMode::Replace);
        assert!(result.contains_key(INTENSITY));
    }

    #[test]
    fn apply_chase_rgb_is_a_level_not_rgb_channels() {
        let p = rgb_fixture().profile().clone();
        let result = p.apply_chase(0.5, EffectLayer::Foreground, BlendMode::Replace);

        // A chase is a brightness mask. Writing it into RGB would make the chase value
        // the colour (always white) and zero every inactive fixture on the layer.
        assert!(!result.contains_key("red"));
        assert!(!result.contains_key("green"));
        assert!(!result.contains_key("blue"));

        let state = result.get(INTENSITY).expect("chase level");
        assert_eq!(state.value, 0.5);
        assert_eq!(state.blend_mode, BlendMode::Replace);
    }

    #[test]
    fn apply_chase_rgb_screen_lifts_through_rgb_channels() {
        let p = rgb_fixture().profile().clone();

        // A lifting blend cannot be expressed as a multiplier, so the mask goes
        // into the colour channels with the blend intact: the active fixture
        // brightens toward white, an inactive one (mask 0) screens to no change.
        let result = p.apply_chase(1.0, EffectLayer::Foreground, BlendMode::Screen);
        for channel in ["red", "green", "blue"] {
            let state = result.get(channel).expect(channel);
            assert_eq!(state.value, 1.0);
            assert_eq!(state.blend_mode, BlendMode::Screen);
        }
        assert!(!result.contains_key(INTENSITY));

        let bed = ChannelState::new(0.4, EffectLayer::Background, BlendMode::Replace);
        let inactive = p.apply_chase(0.0, EffectLayer::Foreground, BlendMode::Screen);
        assert_eq!(bed.blend_with(*inactive.get("red").unwrap()).value, 0.4);

        // Overlay would darken a dim bed where the mask is zero; it lifts instead.
        let overlay = p.apply_chase(0.0, EffectLayer::Foreground, BlendMode::Overlay);
        assert_eq!(overlay.get("red").unwrap().blend_mode, BlendMode::Screen);
    }

    // ── FixtureInfo accessors ────────────────────────────────────────

    #[test]
    fn fixture_info_basic_accessors() {
        let f = FixtureInfo::new(
            "spot1".to_string(),
            2,
            10,
            "generic_spot".to_string(),
            make_channels(&["red", "green", "blue", "dimmer"]),
            Some(25.0),
        );
        assert_eq!(f.name, "spot1");
        assert_eq!(f.universe, 2);
        assert_eq!(f.address, 10);
        assert_eq!(f.fixture_type, "generic_spot");
        assert_eq!(f.max_strobe_frequency, Some(25.0));
        assert_eq!(f.channels.len(), 4);
    }

    /// A head moves on any channel that carries where it points, by the
    /// one rule the crossfade and `is_dark` go by: a GDTF profile's
    /// `pan:1`, or a fine byte on its own, counts; `pan_speed` does not.
    #[test]
    fn a_head_moves_on_a_pan_section_or_fine_byte() {
        let sectioned = FixtureInfo::new(
            "a".to_string(),
            1,
            1,
            "t".to_string(),
            make_channels(&["dimmer", "pan:1", "tilt_fine"]),
            None,
        );
        assert!(sectioned.has_capability(FixtureCapabilities::PANNING));
        assert!(sectioned.has_capability(FixtureCapabilities::TILTING));
        let speed_only = FixtureInfo::new(
            "b".to_string(),
            1,
            1,
            "t".to_string(),
            make_channels(&["dimmer", "pan_speed"]),
            None,
        );
        assert!(!speed_only.has_capability(FixtureCapabilities::PANNING));
        assert!(!speed_only.has_capability(FixtureCapabilities::TILTING));
    }

    #[test]
    fn has_dimmer_is_a_dimmer_channel_of_its_own() {
        let with = FixtureInfo::new(
            "a".to_string(),
            1,
            1,
            "t".to_string(),
            make_channels(&["dimmer", "red"]),
            None,
        );
        let without = FixtureInfo::new(
            "b".to_string(),
            1,
            1,
            "t".to_string(),
            make_channels(&["red", "green", "blue", "dimmer_fine"]),
            None,
        );
        assert!(with.has_dimmer());
        assert!(!without.has_dimmer());
    }

    #[test]
    fn a_cell_name_is_its_fixture_then_the_cell() {
        let name = cell_fixture_name("bar", "cell3");
        assert_eq!(name, "bar/cell3");
        assert_eq!(cell_name(&name), "cell3");
        assert_eq!(cell_name("bar"), "bar");
    }
}
