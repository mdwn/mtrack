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

use super::fixture::FixtureInfo;
use super::types::{BlendMode, EffectLayer};

/// Check if a channel name is a multiplier channel (dimmer or pulse)
/// These are special internal channels used for RGB-only fixtures
#[inline]
pub fn is_multiplier_channel(channel_name: &str) -> bool {
    channel_name.starts_with("_dimmer_mult") || channel_name.starts_with("_pulse_mult")
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
                // Overlay: multiply if base < 0.5, screen if base >= 0.5
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

impl Default for FixtureState {
    fn default() -> Self {
        Self::new()
    }
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
            if is_multiplier_channel(channel_name) {
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

        // Calculate combined multipliers across all layers
        let dimmer_mult =
            read("_dimmer_mult_bg") * read("_dimmer_mult_mid") * read("_dimmer_mult_fg");
        let pulse_mult = read("_pulse_mult_bg") * read("_pulse_mult_mid") * read("_pulse_mult_fg");
        let combined_multiplier = (dimmer_mult * pulse_mult).clamp(0.0, 1.0);

        // Foreground multiplier (for Replace blend mode handling)
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
