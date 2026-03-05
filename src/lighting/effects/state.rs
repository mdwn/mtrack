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

    #[cfg(test)]
    pub fn get_channel(&self, name: &str) -> Option<&ChannelState> {
        self.channels.get(name)
    }

    /// Create a FixtureState from an iterator of channel name/state pairs.
    pub fn from_channels(channels: impl IntoIterator<Item = (String, ChannelState)>) -> Self {
        Self {
            channels: channels.into_iter().collect(),
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

    /// Compute the effective value for a channel, applying RGB multipliers
    /// for fixtures without a dedicated dimmer.
    pub fn effective_channel_value(
        &self,
        channel_name: &str,
        state: &ChannelState,
        has_dedicated_dimmer: bool,
    ) -> f64 {
        let mut value = state.value;
        if !has_dedicated_dimmer
            && (channel_name == "red" || channel_name == "green" || channel_name == "blue")
        {
            let read = |k: &str| self.channels.get(k).map(|c| c.value).unwrap_or(1.0);
            let dimmer_mult =
                read("_dimmer_mult_bg") * read("_dimmer_mult_mid") * read("_dimmer_mult_fg");
            let pulse_mult =
                read("_pulse_mult_bg") * read("_pulse_mult_mid") * read("_pulse_mult_fg");
            let combined_multiplier = (dimmer_mult * pulse_mult).clamp(0.0, 1.0);
            let fg_multiplier = (read("_dimmer_mult_fg") * read("_pulse_mult_fg")).clamp(0.0, 1.0);

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
        value
    }

    /// Convert to DMX commands
    pub fn to_dmx_commands(&self, fixture_info: &FixtureInfo) -> Vec<DmxCommand> {
        let mut commands = Vec::new();
        let has_dedicated_dimmer = fixture_info.channels.contains_key("dimmer");

        for (channel_name, state) in &self.channels {
            if let Some(&channel_offset) = fixture_info.channels.get(channel_name) {
                let dmx_channel = fixture_info.address + channel_offset - 1;
                let value = self.effective_channel_value(channel_name, state, has_dedicated_dimmer);
                let dmx_value = (value * 255.0) as u8;

                commands.push(DmxCommand {
                    universe: fixture_info.universe,
                    channel: dmx_channel,
                    value: dmx_value,
                });
            }
        }

        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_multiplier_channel ──────────────────────────────────────

    #[test]
    fn is_multiplier_dimmer_bg() {
        assert!(is_multiplier_channel("_dimmer_mult_bg"));
    }

    #[test]
    fn is_multiplier_dimmer_fg() {
        assert!(is_multiplier_channel("_dimmer_mult_fg"));
    }

    #[test]
    fn is_multiplier_pulse_mid() {
        assert!(is_multiplier_channel("_pulse_mult_mid"));
    }

    #[test]
    fn is_multiplier_regular_channel() {
        assert!(!is_multiplier_channel("red"));
        assert!(!is_multiplier_channel("dimmer"));
        assert!(!is_multiplier_channel("strobe"));
    }

    // ── ChannelState::new ──────────────────────────────────────────

    #[test]
    fn channel_state_new_clamps_above_one() {
        let cs = ChannelState::new(1.5, EffectLayer::Background, BlendMode::Replace);
        assert_eq!(cs.value, 1.0);
    }

    #[test]
    fn channel_state_new_clamps_below_zero() {
        let cs = ChannelState::new(-0.5, EffectLayer::Background, BlendMode::Replace);
        assert_eq!(cs.value, 0.0);
    }

    #[test]
    fn channel_state_new_normal_value() {
        let cs = ChannelState::new(0.75, EffectLayer::Midground, BlendMode::Add);
        assert_eq!(cs.value, 0.75);
        assert_eq!(cs.layer, EffectLayer::Midground);
        assert_eq!(cs.blend_mode, BlendMode::Add);
    }

    // ── ChannelState::blend_with — Replace ─────────────────────────

    #[test]
    fn blend_replace() {
        let a = ChannelState::new(0.3, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.8, EffectLayer::Foreground, BlendMode::Replace);
        let result = a.blend_with(b);
        assert!((result.value - 0.8).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Multiply ────────────────────────

    #[test]
    fn blend_multiply() {
        let a = ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.6, EffectLayer::Midground, BlendMode::Multiply);
        let result = a.blend_with(b);
        assert!((result.value - 0.3).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Add ─────────────────────────────

    #[test]
    fn blend_add_no_overflow() {
        let a = ChannelState::new(0.3, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.4, EffectLayer::Midground, BlendMode::Add);
        let result = a.blend_with(b);
        assert!((result.value - 0.7).abs() < 1e-9);
    }

    #[test]
    fn blend_add_clamped_to_one() {
        let a = ChannelState::new(0.7, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.8, EffectLayer::Midground, BlendMode::Add);
        let result = a.blend_with(b);
        assert!((result.value - 1.0).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Overlay ─────────────────────────

    #[test]
    fn blend_overlay_dark_base() {
        // base < 0.5 → multiply formula: 2 * base * overlay
        let a = ChannelState::new(0.25, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::Midground, BlendMode::Overlay);
        let result = a.blend_with(b);
        assert!((result.value - 0.25).abs() < 1e-9); // 2 * 0.25 * 0.5 = 0.25
    }

    #[test]
    fn blend_overlay_bright_base() {
        // base >= 0.5 → screen formula: 1 - 2*(1-base)*(1-overlay)
        let a = ChannelState::new(0.75, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::Midground, BlendMode::Overlay);
        let result = a.blend_with(b);
        // 1 - 2*(0.25)*(0.5) = 1 - 0.25 = 0.75
        assert!((result.value - 0.75).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Screen ──────────────────────────

    #[test]
    fn blend_screen() {
        let a = ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::Midground, BlendMode::Screen);
        let result = a.blend_with(b);
        // 1 - (0.5)*(0.5) = 0.75
        assert!((result.value - 0.75).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — layer propagation ───────────────

    #[test]
    fn blend_uses_higher_layer() {
        let a = ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::Foreground, BlendMode::Multiply);
        let result = a.blend_with(b);
        assert_eq!(result.layer, EffectLayer::Foreground);
        assert_eq!(result.blend_mode, BlendMode::Multiply);
    }

    #[test]
    fn blend_keeps_self_blend_mode_when_higher() {
        let a = ChannelState::new(0.5, EffectLayer::Foreground, BlendMode::Add);
        let b = ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply);
        let result = a.blend_with(b);
        assert_eq!(result.layer, EffectLayer::Foreground);
        assert_eq!(result.blend_mode, BlendMode::Add);
    }

    // ── FixtureState ───────────────────────────────────────────────

    #[test]
    fn fixture_state_default_empty() {
        let fs = FixtureState::default();
        assert!(fs.channels.is_empty());
    }

    #[test]
    fn fixture_state_set_and_get_channel() {
        let mut fs = FixtureState::new();
        let cs = ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace);
        fs.set_channel("red".to_string(), cs);
        assert_eq!(fs.get_channel("red"), Some(&cs));
    }

    #[test]
    fn fixture_state_from_channels() {
        let channels = vec![
            (
                "red".to_string(),
                ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
            ),
            (
                "green".to_string(),
                ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
            ),
        ];
        let fs = FixtureState::from_channels(channels);
        assert_eq!(fs.channels.len(), 2);
    }

    // ── FixtureState::blend_with ───────────────────────────────────

    #[test]
    fn fixture_state_blend_adds_new_channels() {
        let mut fs1 = FixtureState::new();
        fs1.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );

        let mut fs2 = FixtureState::new();
        fs2.set_channel(
            "green".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );

        fs1.blend_with(&fs2);
        assert!(fs1.get_channel("red").is_some());
        assert!(fs1.get_channel("green").is_some());
    }

    #[test]
    fn fixture_state_blend_blends_existing_channels() {
        let mut fs1 = FixtureState::new();
        fs1.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );

        let mut fs2 = FixtureState::new();
        fs2.set_channel(
            "red".to_string(),
            ChannelState::new(0.8, EffectLayer::Midground, BlendMode::Replace),
        );

        fs1.blend_with(&fs2);
        let red = fs1.get_channel("red").unwrap();
        assert!((red.value - 0.8).abs() < 1e-9); // Replace mode
    }

    #[test]
    fn fixture_state_blend_multiplier_overwrites() {
        let mut fs1 = FixtureState::new();
        fs1.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply),
        );

        let mut fs2 = FixtureState::new();
        fs2.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.8, EffectLayer::Background, BlendMode::Multiply),
        );

        fs1.blend_with(&fs2);
        let mult = fs1.get_channel("_dimmer_mult_bg").unwrap();
        // Multiplier channels overwrite, not blend
        assert!((mult.value - 0.8).abs() < 1e-9);
    }

    // ── effective_channel_value ─────────────────────────────────────

    #[test]
    fn effective_value_with_dedicated_dimmer() {
        let mut fs = FixtureState::new();
        let cs = ChannelState::new(0.8, EffectLayer::Background, BlendMode::Replace);
        fs.set_channel("red".to_string(), cs);
        // With a dedicated dimmer, multipliers should NOT apply
        let value = fs.effective_channel_value("red", &cs, true);
        assert!((value - 0.8).abs() < 1e-9);
    }

    #[test]
    fn effective_value_without_dimmer_no_multipliers() {
        let mut fs = FixtureState::new();
        let cs = ChannelState::new(0.8, EffectLayer::Background, BlendMode::Replace);
        fs.set_channel("red".to_string(), cs);
        // No multipliers set → defaults to 1.0 → no change
        let value = fs.effective_channel_value("red", &cs, false);
        assert!((value - 0.8).abs() < 1e-9);
    }

    #[test]
    fn effective_value_with_dimmer_multiplier() {
        let mut fs = FixtureState::new();
        let cs = ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace);
        fs.set_channel("red".to_string(), cs);
        fs.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply),
        );
        // Only bg dimmer set (0.5), mid/fg default to 1.0, pulse defaults to 1.0
        // combined = 0.5 * 1.0 * 1.0 * 1.0 * 1.0 * 1.0 = 0.5
        let value = fs.effective_channel_value("red", &cs, false);
        assert!((value - 0.5).abs() < 1e-9);
    }

    #[test]
    fn effective_value_non_rgb_channel_unaffected() {
        let mut fs = FixtureState::new();
        let cs = ChannelState::new(0.8, EffectLayer::Background, BlendMode::Replace);
        fs.set_channel("strobe".to_string(), cs);
        fs.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.1, EffectLayer::Background, BlendMode::Multiply),
        );
        // Non-RGB channels are not affected by multipliers
        let value = fs.effective_channel_value("strobe", &cs, false);
        assert!((value - 0.8).abs() < 1e-9);
    }

    #[test]
    fn effective_value_foreground_replace_uses_fg_only() {
        let mut fs = FixtureState::new();
        let cs = ChannelState::new(1.0, EffectLayer::Foreground, BlendMode::Replace);
        fs.set_channel("red".to_string(), cs);
        fs.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.1, EffectLayer::Background, BlendMode::Multiply),
        );
        fs.set_channel(
            "_dimmer_mult_fg".to_string(),
            ChannelState::new(0.5, EffectLayer::Foreground, BlendMode::Multiply),
        );
        // Foreground+Replace uses only fg multiplier: dimmer_fg * pulse_fg = 0.5 * 1.0
        let value = fs.effective_channel_value("red", &cs, false);
        assert!((value - 0.5).abs() < 1e-9);
    }

    // ── to_dmx_commands ────────────────────────────────────────────

    fn make_fixture_info(channels: Vec<(&str, u16)>, address: u16) -> FixtureInfo {
        let ch: HashMap<String, u16> = channels.iter().map(|(n, o)| (n.to_string(), *o)).collect();
        FixtureInfo::new(
            "test".to_string(),
            1,
            address,
            "generic".to_string(),
            ch,
            None,
        )
    }

    #[test]
    fn to_dmx_commands_basic() {
        let fixture = make_fixture_info(vec![("red", 1), ("green", 2), ("blue", 3)], 10);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].universe, 1);
        assert_eq!(cmds[0].channel, 10); // address(10) + offset(1) - 1
        assert_eq!(cmds[0].value, 255);
    }

    #[test]
    fn to_dmx_commands_skips_unknown_channels() {
        let fixture = make_fixture_info(vec![("red", 1)], 1);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "nonexistent".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert!(cmds.is_empty());
    }

    #[test]
    fn to_dmx_commands_skips_multiplier_channels() {
        let fixture = make_fixture_info(vec![("red", 1)], 1);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        // Multiplier channels have no fixture mapping, so they produce no commands
        assert!(cmds.is_empty());
    }

    #[test]
    fn to_dmx_commands_half_value() {
        let fixture = make_fixture_info(vec![("dimmer", 1)], 1);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "dimmer".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].value, 127); // 0.5 * 255 = 127
    }
}
