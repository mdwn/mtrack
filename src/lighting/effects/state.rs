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

use super::composite::{self, Op};
use super::fixture::FixtureInfo;
use super::physical::PhysicalState;
use super::types::{BlendMode, EffectLayer};

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
    /// How far a `Replace` moves from the value beneath toward `value`, 0.0
    /// to 1.0. A replace fading in or out is a crossfade with the layers
    /// below it, not its own colour scaled toward black (#521). Always 1.0
    /// for every other blend mode, and once blended.
    pub mix: f64,
}

impl ChannelState {
    pub fn new(value: f64, layer: EffectLayer, blend_mode: BlendMode) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            layer,
            blend_mode,
            mix: 1.0,
        }
    }

    /// This state with a partial `mix` (see [`ChannelState::mix`]).
    pub fn with_mix(self, mix: f64) -> Self {
        Self {
            mix: mix.clamp(0.0, 1.0),
            ..self
        }
    }

    /// This state as it stands with nothing beneath it: a partial replace
    /// over nothing fades from zero, as it always has.
    pub fn resolved(self) -> Self {
        Self {
            value: self.value * self.mix,
            mix: 1.0,
            ..self
        }
    }

    /// Blend this channel state with another
    pub fn blend_with(&self, other: ChannelState) -> ChannelState {
        let blended_value = blend_value(self.value, other.value, other.blend_mode, other.mix);

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
            mix: 1.0,
        }
    }
}

/// One value laid over another by a blend mode. `mix` is how far a
/// `Replace` moves toward `over`; the other modes ignore it.
pub(super) fn blend_value(base: f64, over: f64, mode: BlendMode, mix: f64) -> f64 {
    match mode {
        BlendMode::Replace => crate::lighting::channels::lerp(base, over, mix),
        BlendMode::Multiply => base * over,
        BlendMode::Add => (base + over).min(1.0),
        BlendMode::Overlay => {
            // Overlay: multiply if base < 0.5, screen if base >= 0.5
            if base < 0.5 {
                2.0 * base * over
            } else {
                1.0 - 2.0 * (1.0 - base) * (1.0 - over)
            }
        }
        BlendMode::Screen => 1.0 - (1.0 - base) * (1.0 - over),
    }
}

/// Represents the current state of a fixture
#[derive(Debug, Clone)]
pub struct FixtureState {
    pub channels: HashMap<String, ChannelState>,
    /// What the show asked for in physical units (degrees), resolved into
    /// bytes only when DMX is produced. Never scaled by a layer master.
    pub physical: PhysicalState,
    /// The rate a software strobe (one on a fixture with no strobe
    /// channel, flashing its dimmer or colour) is flashing this fixture
    /// at, so the state stream can say so: the bytes alone only show the
    /// phase. Later writer wins, as the effect order runs. `None` when no
    /// software strobe drives it.
    pub strobe_hz: Option<f64>,
    /// What effects asked of the fixture's colour (`red`, `green`, `blue`,
    /// `white`) and of its level, in the order they came, each with its
    /// layer. Not channels: they are composited, bottom layer up, into the
    /// plain channels by [`Self::resolve`] once the frame's layers are all
    /// in (design 3.4). Empty once resolved.
    pub(super) ops: Vec<Op>,
    /// How many effects' ops `ops` holds, so the ops of one effect stay
    /// together when states merge.
    pub(super) groups: u32,
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
            physical: PhysicalState::default(),
            strobe_hz: None,
            ops: Vec::new(),
            groups: 0,
        }
    }

    /// A channel as the tests read it: a plain one, or the latest colour
    /// or level an effect asked for that has not been resolved yet.
    #[cfg(test)]
    pub fn get_channel(&self, name: &str) -> Option<&ChannelState> {
        match composite::op_kind(name) {
            Some(kind) => self
                .ops
                .iter()
                .rev()
                .find(|op| op.kind == kind)
                .map(|op| &op.state),
            None => self.channels.get(name),
        }
    }

    /// Create a FixtureState from an iterator of channel name/state pairs.
    pub fn from_channels(channels: impl IntoIterator<Item = (String, ChannelState)>) -> Self {
        let mut out = Self::new();
        for (name, state) in channels {
            out.set_channel(name, state);
        }
        out
    }

    /// Set a channel value. A colour channel or the internal level
    /// ([`composite::INTENSITY`]) is recorded as an op to composite, not
    /// blended into a channel; setting one again within the same effect's
    /// state replaces it.
    pub fn set_channel(&mut self, channel_name: String, state: ChannelState) {
        match composite::op_kind(&channel_name) {
            Some(kind) => {
                if self.groups == 0 {
                    self.groups = 1;
                }
                let group = self.groups - 1;
                self.ops
                    .retain(|op| !(op.group == group && op.kind == kind));
                self.ops.push(Op { kind, state, group });
            }
            None => {
                self.channels.insert(channel_name, state);
            }
        }
    }

    /// Every value the state carries, plain channels and unresolved colour
    /// and level alike, for tests that read an effect's output as it is
    /// before the frame is composited.
    #[cfg(test)]
    pub fn all_states(&self) -> impl Iterator<Item = &ChannelState> {
        self.channels
            .values()
            .chain(self.ops.iter().map(|op| &op.state))
    }

    /// Forgets colour and level not yet composited, for a state that
    /// belongs to no fixture.
    pub fn drop_unresolved(&mut self) {
        self.ops.clear();
        self.groups = 0;
    }

    /// Scale every value this state carries, channels and ops alike (a
    /// layer's intensity master).
    pub fn scale_values(&mut self, factor: f64) {
        for channel_state in self.channels.values_mut() {
            channel_state.value *= factor;
        }
        for op in &mut self.ops {
            op.state.value *= factor;
        }
    }

    /// Composite the colour and level effects asked for, bottom layer up,
    /// into plain `red/green/blue/white/dimmer` channels for a fixture (or
    /// a cell) with `channels` (design 3.4). Values already in the plain
    /// channels (a MIDI DMX show's) are the bottom of the stack.
    pub fn resolve<V>(&mut self, channels: &HashMap<String, V>) {
        composite::resolve_plain(self, channels);
    }

    /// Blend this fixture state with another
    pub fn blend_with(&mut self, other: &FixtureState) {
        self.physical.blend_with(&other.physical);
        if other.strobe_hz.is_some() {
            self.strobe_hz = other.strobe_hz;
        }
        // Colour and level are composited at the end of the frame.
        for op in &other.ops {
            self.ops.push(Op {
                group: op.group + self.groups,
                ..*op
            });
        }
        self.groups += other.groups;
        // Blend other channels normally
        for (channel_name, other_state) in &other.channels {
            if let Some(self_state) = self.channels.get(channel_name) {
                // Blend existing channel
                let blended_state = self_state.blend_with(*other_state);

                self.channels.insert(channel_name.clone(), blended_state);
            } else {
                // Add new channel
                self.channels
                    .insert(channel_name.clone(), other_state.resolved());
            }
        }
    }

    /// The flags a CMY fixture's colour goes out as: on a fixture mixing
    /// subtractively (`has_cmy` of its `channels`) each primary an effect
    /// drives, as the flag that removes it at its complement — cyan
    /// removes red, so full red is cyan at 0 (each 0 to 1). The engine's
    /// colour is kept in RGB so layers blend alike everywhere; this is the
    /// one place it turns into the flags the head is sent, and what the
    /// fixture's snapshot shows in place of its red, green and blue. A
    /// primary no effect drives yields nothing (its flag is left alone),
    /// and a fixture that does not mix subtractively nothing at all.
    pub fn cmy_flags<V>(
        &self,
        channels: &HashMap<String, V>,
    ) -> impl Iterator<Item = (&'static str, f64)> + '_ {
        let subtractive = super::fixture::has_cmy(channels);
        crate::lighting::channels::CMY_OF_RGB
            .map(|(rgb, cmy)| {
                let state = self.channels.get(rgb).filter(|_| subtractive)?;
                Some((cmy, 1.0 - state.value))
            })
            .into_iter()
            .flatten()
    }
}

impl FixtureState {
    /// A cell's channel values (0..1), by channel name, from this
    /// fixture-level state with the cell's own state (`own`) blended over
    /// it (design 17.3). Both are resolved: the brightness model has
    /// already put each cell's colour and level in plain channels.
    pub fn cell_values(
        &self,
        cell: &crate::lighting::types::Cell,
        own: Option<&FixtureState>,
    ) -> Vec<(String, f64)> {
        let mut out = Vec::with_capacity(cell.channels.len());
        for channel_name in cell.channels.keys() {
            let base = self.channels.get(channel_name);
            let over = own.and_then(|o| o.channels.get(channel_name));
            let state = match (base, over) {
                (Some(b), Some(o)) => b.blend_with(*o),
                (Some(b), None) => b.resolved(),
                (None, Some(o)) => o.resolved(),
                (None, None) => continue,
            };
            out.push((channel_name.clone(), state.value));
        }
        out
    }

    /// Convert to DMX commands.
    ///
    /// Normalized channels resolve through the fixture's channel
    /// definitions: an 8-bit channel gets the byte the engine always wrote,
    /// a 16-bit one gets its fine byte too. Physical intents (pan, tilt in
    /// degrees) resolve through the channel's range and win over any
    /// normalized value written to the same channel.
    pub fn to_dmx_commands(&self, fixture_info: &FixtureInfo) -> Vec<DmxCommand> {
        self.to_dmx_commands_with_cells(fixture_info, |_| None)
    }

    /// [`Self::to_dmx_commands`] for a pixel fixture (design §17.3): the
    /// channels a cell derives are written per cell, each cell's bytes
    /// resolved from this state with the cell's own state (from
    /// `cell_state`, by cell name) blended over it by the usual layer
    /// rules. With no cell states the bytes are exactly the ganged ones.
    pub fn to_dmx_commands_with_cells<'a>(
        &self,
        fixture_info: &FixtureInfo,
        cell_state: impl Fn(&str) -> Option<&'a FixtureState>,
    ) -> Vec<DmxCommand> {
        // The engine resolves every frame's states before they get here; a
        // state built by hand may still hold colour and level to composite.
        let pending = !self.ops.is_empty()
            || fixture_info
                .cells
                .iter()
                .any(|c| cell_state(&c.name).is_some_and(|s| !s.ops.is_empty()));
        if pending {
            let mut parent = self.clone();
            let mut cells: HashMap<String, FixtureState> = fixture_info
                .cells
                .iter()
                .filter_map(|c| cell_state(&c.name).map(|s| (c.name.clone(), s.clone())))
                .collect();
            composite::resolve_fixture(&mut parent, &mut cells, fixture_info);
            return parent.emit_commands(fixture_info, |name| cells.get(name));
        }
        self.emit_commands(fixture_info, cell_state)
    }

    /// The bytes of a resolved state.
    fn emit_commands<'a>(
        &self,
        fixture_info: &FixtureInfo,
        cell_state: impl Fn(&str) -> Option<&'a FixtureState>,
    ) -> Vec<DmxCommand> {
        let mut commands = Vec::new();
        // A byte for the channel at `offset`, where the fixture has one.
        let command = |(offset, value): (u16, u8)| {
            Some(DmxCommand {
                universe: fixture_info.universe,
                channel: fixture_info.dmx_channel(offset)?,
                value,
            })
        };
        let physical =
            super::physical::resolve_physical(&self.physical, &fixture_info.channel_defs);
        let cell_owned = |channel: &str| {
            fixture_info
                .cells
                .first()
                .is_some_and(|c| c.channels.contains_key(channel))
        };

        for (channel_name, state) in &self.channels {
            if physical.iter().any(|(name, _)| name == channel_name) || cell_owned(channel_name) {
                continue;
            }
            if let Some(def) = fixture_info.channel_defs.get(channel_name) {
                let value = state.value;
                commands
                    .extend(super::physical::resolve_normalized(def, value).filter_map(command));
            }
        }
        // A CMY fixture has no red, green or blue of its own: the engine's
        // colour, kept in RGB so layers blend alike everywhere, goes out as
        // its complement — cyan removes red, so full red is cyan at 0. A
        // primary no effect drives is left alone, open, as a colour-less
        // fixture is. Written after the named channels, so an effect's
        // colour wins over a `static` naming the flag directly.
        for (cmy, value) in self.cmy_flags(&fixture_info.channel_defs) {
            if let Some(def) = fixture_info.channel_defs.get(cmy) {
                commands
                    .extend(super::physical::resolve_normalized(def, value).filter_map(command));
            }
        }
        for cell in &fixture_info.cells {
            for (channel_name, value) in self.cell_values(cell, cell_state(&cell.name)) {
                let def = &cell.channels[&channel_name];
                commands
                    .extend(super::physical::resolve_normalized(def, value).filter_map(command));
            }
        }
        for (_, resolved) in physical {
            commands.extend(resolved.bytes.into_iter().filter_map(command));
        }

        commands
    }
}

#[cfg(test)]
mod tests {
    use super::super::physical::{Intent, PhysicalParameter};
    use super::*;

    // ── ChannelState::new ──────────────────────────────────────────

    #[test]
    fn channel_state_new_clamps_above_one() {
        let cs = ChannelState::new(1.5, EffectLayer::BACKGROUND, BlendMode::Replace);
        assert_eq!(cs.value, 1.0);
    }

    #[test]
    fn channel_state_new_clamps_below_zero() {
        let cs = ChannelState::new(-0.5, EffectLayer::BACKGROUND, BlendMode::Replace);
        assert_eq!(cs.value, 0.0);
    }

    #[test]
    fn channel_state_new_normal_value() {
        let cs = ChannelState::new(0.75, EffectLayer::MIDGROUND, BlendMode::Add);
        assert_eq!(cs.value, 0.75);
        assert_eq!(cs.layer, EffectLayer::MIDGROUND);
        assert_eq!(cs.blend_mode, BlendMode::Add);
    }

    // ── ChannelState::blend_with — Replace ─────────────────────────

    #[test]
    fn blend_replace() {
        let a = ChannelState::new(0.3, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.8, EffectLayer::FOREGROUND, BlendMode::Replace);
        let result = a.blend_with(b);
        assert!((result.value - 0.8).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Multiply ────────────────────────

    #[test]
    fn blend_multiply() {
        let a = ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.6, EffectLayer::MIDGROUND, BlendMode::Multiply);
        let result = a.blend_with(b);
        assert!((result.value - 0.3).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Add ─────────────────────────────

    #[test]
    fn blend_add_no_overflow() {
        let a = ChannelState::new(0.3, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.4, EffectLayer::MIDGROUND, BlendMode::Add);
        let result = a.blend_with(b);
        assert!((result.value - 0.7).abs() < 1e-9);
    }

    #[test]
    fn blend_add_clamped_to_one() {
        let a = ChannelState::new(0.7, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.8, EffectLayer::MIDGROUND, BlendMode::Add);
        let result = a.blend_with(b);
        assert!((result.value - 1.0).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Overlay ─────────────────────────

    #[test]
    fn blend_overlay_dark_base() {
        // base < 0.5 → multiply formula: 2 * base * overlay
        let a = ChannelState::new(0.25, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::MIDGROUND, BlendMode::Overlay);
        let result = a.blend_with(b);
        assert!((result.value - 0.25).abs() < 1e-9); // 2 * 0.25 * 0.5 = 0.25
    }

    #[test]
    fn blend_overlay_bright_base() {
        // base >= 0.5 → screen formula: 1 - 2*(1-base)*(1-overlay)
        let a = ChannelState::new(0.75, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::MIDGROUND, BlendMode::Overlay);
        let result = a.blend_with(b);
        // 1 - 2*(0.25)*(0.5) = 1 - 0.25 = 0.75
        assert!((result.value - 0.75).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — Screen ──────────────────────────

    #[test]
    fn blend_screen() {
        let a = ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::MIDGROUND, BlendMode::Screen);
        let result = a.blend_with(b);
        // 1 - (0.5)*(0.5) = 0.75
        assert!((result.value - 0.75).abs() < 1e-9);
    }

    // ── ChannelState::blend_with — layer propagation ───────────────

    #[test]
    fn blend_uses_higher_layer() {
        let a = ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace);
        let b = ChannelState::new(0.5, EffectLayer::FOREGROUND, BlendMode::Multiply);
        let result = a.blend_with(b);
        assert_eq!(result.layer, EffectLayer::FOREGROUND);
        assert_eq!(result.blend_mode, BlendMode::Multiply);
    }

    #[test]
    fn blend_keeps_self_blend_mode_when_higher() {
        let a = ChannelState::new(0.5, EffectLayer::FOREGROUND, BlendMode::Add);
        let b = ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Multiply);
        let result = a.blend_with(b);
        assert_eq!(result.layer, EffectLayer::FOREGROUND);
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
        let cs = ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace);
        fs.set_channel("red".to_string(), cs);
        assert_eq!(fs.get_channel("red"), Some(&cs));
    }

    #[test]
    fn fixture_state_from_channels() {
        let channels = vec![
            (
                "red".to_string(),
                ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
            ),
            (
                "green".to_string(),
                ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
            ),
        ];
        let fs = FixtureState::from_channels(channels);
        assert_eq!(fs.ops.len(), 2, "colour is recorded as ops");
    }

    // ── FixtureState::blend_with ───────────────────────────────────

    #[test]
    fn fixture_state_blend_adds_new_channels() {
        let mut fs1 = FixtureState::new();
        fs1.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );

        let mut fs2 = FixtureState::new();
        fs2.set_channel(
            "green".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
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
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );

        let mut fs2 = FixtureState::new();
        fs2.set_channel(
            "red".to_string(),
            ChannelState::new(0.8, EffectLayer::MIDGROUND, BlendMode::Replace),
        );

        fs1.blend_with(&fs2);
        let red = fs1.get_channel("red").unwrap();
        assert!((red.value - 0.8).abs() < 1e-9); // Replace mode
    }

    /// Colour and level are recorded, not blended: both asks survive the
    /// merge, in order, for the end-of-frame composite.
    #[test]
    fn fixture_state_blend_keeps_colour_and_level_as_ops() {
        let mut fs1 = FixtureState::new();
        fs1.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut fs2 = FixtureState::new();
        fs2.set_channel(
            "red".to_string(),
            ChannelState::new(0.8, EffectLayer::MIDGROUND, BlendMode::Replace),
        );
        fs2.set_channel(
            "_intensity".to_string(),
            ChannelState::new(0.4, EffectLayer::MIDGROUND, BlendMode::Multiply),
        );
        fs1.blend_with(&fs2);
        assert!(fs1.channels.is_empty());
        assert_eq!(fs1.ops.len(), 3);
        assert_eq!(fs1.get_channel("red").unwrap().value, 0.8);
        assert_eq!(fs1.get_channel("_intensity").unwrap().value, 0.4);
    }

    /// The layer intensity master scales the ops as it scales channels.
    #[test]
    fn scale_values_scales_ops_and_channels() {
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        fs.set_channel(
            "_intensity".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        fs.set_channel(
            "pan".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        fs.scale_values(0.5);
        assert_eq!(fs.get_channel("red").unwrap().value, 0.5);
        assert_eq!(fs.get_channel("_intensity").unwrap().value, 0.25);
        assert_eq!(fs.get_channel("pan").unwrap().value, 0.5);
    }

    /// A state built with colour and a level still produces the bytes of
    /// the composite when converted by hand.
    #[test]
    fn to_dmx_commands_composites_unresolved_ops() {
        let fixture = make_fixture_info(vec![("dimmer", 1), ("red", 2)], 1);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        fs.set_channel(
            "_intensity".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut cmds = fs.to_dmx_commands(&fixture);
        cmds.sort_by_key(|c| c.channel);
        assert_eq!(
            cmds.iter().map(|c| c.value).collect::<Vec<_>>(),
            vec![128, 255]
        );
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
    fn a_fixture_with_its_own_rgb_gets_no_cmy_complement() {
        // RGB and CMY both: the RGB is written, the flags are left alone.
        let fixture = make_fixture_info(
            vec![
                ("red", 1),
                ("green", 2),
                ("blue", 3),
                ("cyan", 4),
                ("magenta", 5),
                ("yellow", 6),
            ],
            1,
        );
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert_eq!(cmds.len(), 1);
        assert_eq!((cmds[0].channel, cmds[0].value), (1, 255));
    }

    #[test]
    fn to_dmx_commands_basic() {
        let fixture = make_fixture_info(vec![("red", 1), ("green", 2), ("blue", 3)], 10);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].universe, 1);
        assert_eq!(cmds[0].channel, 10); // address(10) + offset(1) - 1
        assert_eq!(cmds[0].value, 255);
    }

    #[test]
    fn a_physical_pan_resolves_through_the_range_and_wins_over_the_channel_write() {
        use crate::lighting::types::{ChannelDef, PhysicalRange, PhysicalUnit};
        let mut fixture = make_fixture_info(vec![("pan", 1), ("pan_fine", 2), ("dimmer", 3)], 10);
        let mut defs = HashMap::new();
        defs.insert(
            "pan".to_string(),
            ChannelDef {
                offset: 1,
                fine: Some(2),
                range: Some(PhysicalRange {
                    from: -270.0,
                    to: 270.0,
                    unit: PhysicalUnit::Degrees,
                }),
                functions: Vec::new(),
                mirrors: Vec::new(),
            },
        );
        defs.insert("dimmer".to_string(), ChannelDef::at(3));
        fixture = fixture.with_channel_defs(defs);

        let mut fs = FixtureState::new();
        // A normalized write to pan, as `static pan: 100%` produces...
        fs.set_channel(
            "pan".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        fs.set_channel(
            "dimmer".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        // ...loses to a physical intent on the same channel.
        fs.physical.set(
            PhysicalParameter::Pan,
            Intent {
                degrees: 0.0,
                layer: EffectLayer::BACKGROUND,
            },
        );
        let mut cmds = fs.to_dmx_commands(&fixture);
        cmds.sort_by_key(|c| c.channel);
        let bytes: Vec<(u16, u8)> = cmds.iter().map(|c| (c.channel, c.value)).collect();
        assert_eq!(
            bytes,
            vec![(10, 0x80), (11, 0x00), (12, 128)],
            "0° is mid-travel over coarse+fine at address 10/11; the dimmer byte is untouched"
        );
    }

    #[test]
    fn a_normalized_write_to_a_sixteen_bit_channel_fans_out() {
        use crate::lighting::types::ChannelDef;
        let mut fixture = make_fixture_info(vec![("pan", 1), ("pan_fine", 2)], 1);
        let mut defs = HashMap::new();
        defs.insert(
            "pan".to_string(),
            ChannelDef {
                offset: 1,
                fine: Some(2),
                range: None,
                functions: Vec::new(),
                mirrors: Vec::new(),
            },
        );
        fixture = fixture.with_channel_defs(defs);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "pan".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut cmds = fs.to_dmx_commands(&fixture);
        cmds.sort_by_key(|c| c.channel);
        assert_eq!(cmds.len(), 2);
        assert_eq!((cmds[0].channel, cmds[0].value), (1, 128));
        assert_eq!(cmds[1].channel, 2);
    }

    #[test]
    fn to_dmx_commands_skips_unknown_channels() {
        let fixture = make_fixture_info(vec![("red", 1)], 1);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "nonexistent".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert!(cmds.is_empty());
    }

    #[test]
    fn to_dmx_commands_half_value() {
        let fixture = make_fixture_info(vec![("dimmer", 1)], 1);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "dimmer".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].value, 128); // 0.5 * 255 = 128.5, rounded
    }

    // ── to_dmx_commands_with_cells (design §17.3) ─────────────────────

    /// A two-cell fixture at address 10: fixture-level `red` at offset 1
    /// (mirrored to offset 2, the pre-cell path — cells must win over it),
    /// `dimmer` at offset 3 (which no cell owns), cell "a" owning byte 1
    /// and cell "b" owning byte 2.
    fn two_cell_fixture() -> FixtureInfo {
        use crate::lighting::types::{Cell, ChannelDef};

        let fixture = make_fixture_info(vec![("red", 1), ("dimmer", 3)], 10);
        let mut defs = HashMap::new();
        defs.insert(
            "red".to_string(),
            ChannelDef {
                offset: 1,
                fine: None,
                range: None,
                functions: Vec::new(),
                mirrors: vec![(2, None)],
            },
        );
        defs.insert("dimmer".to_string(), ChannelDef::at(3));
        let mut fixture = fixture.with_channel_defs(defs);
        fixture.cells = vec![
            Cell {
                name: "a".to_string(),
                channels: HashMap::from([("red".to_string(), ChannelDef::at(1))]),
                offset: [0.0, 0.0, 0.0],
            },
            Cell {
                name: "b".to_string(),
                channels: HashMap::from([("red".to_string(), ChannelDef::at(2))]),
                offset: [0.0, 0.0, 0.0],
            },
        ];
        fixture
    }

    fn byte_at(commands: &[DmxCommand], channel: u16) -> u8 {
        commands
            .iter()
            .find(|c| c.channel == channel)
            .map(|c| c.value)
            .unwrap_or(0)
    }

    #[test]
    fn with_no_cell_states_both_cells_show_the_fixture_value() {
        let fixture = two_cell_fixture();
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands_with_cells(&fixture, |_| None);
        // Channel 10 = byte 1 (cell "a"), channel 11 = byte 2 (cell "b").
        assert_eq!(byte_at(&cmds, 10), 128, "cell a == fixture value");
        assert_eq!(byte_at(&cmds, 11), 128, "cell b == fixture value");
    }

    #[test]
    fn a_cell_state_on_a_higher_layer_with_replace_wins_for_its_own_byte_only() {
        let fixture = two_cell_fixture();
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut cell_b = FixtureState::new();
        cell_b.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::FOREGROUND, BlendMode::Replace),
        );
        let cmds =
            fs.to_dmx_commands_with_cells(
                &fixture,
                |name| {
                    if name == "b" {
                        Some(&cell_b)
                    } else {
                        None
                    }
                },
            );
        assert_eq!(byte_at(&cmds, 10), 128, "cell a stays at the fixture value");
        assert_eq!(byte_at(&cmds, 11), 255, "cell b takes the cell's value");
    }

    #[test]
    fn a_cell_state_on_the_same_layer_with_multiply_blends_into_a_product() {
        let fixture = two_cell_fixture();
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut cell_b = FixtureState::new();
        cell_b.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Multiply),
        );
        let cmds =
            fs.to_dmx_commands_with_cells(
                &fixture,
                |name| {
                    if name == "b" {
                        Some(&cell_b)
                    } else {
                        None
                    }
                },
            );
        assert_eq!(byte_at(&cmds, 10), 128, "cell a untouched");
        // 0.5 * 0.5 = 0.25 -> resolve_normalized(0.25) = 63.75, rounded to 64
        assert_eq!(byte_at(&cmds, 11), 64, "cell b is the product");
    }

    #[test]
    fn to_dmx_commands_is_unchanged_for_a_fixture_without_cells() {
        let fixture = make_fixture_info(vec![("red", 1), ("green", 2), ("blue", 3)], 10);
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands(&fixture);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].channel, 10);
        assert_eq!(cmds[0].value, 255);
    }

    // ── cell_values (design §17.4) ────────────────────────────────────

    fn red_cell() -> crate::lighting::types::Cell {
        crate::lighting::types::Cell {
            name: "2".to_string(),
            channels: HashMap::from([(
                "red".to_string(),
                crate::lighting::types::ChannelDef::at(5),
            )]),
            offset: [0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn cell_values_multiplies_a_cells_own_state_over_the_bed() {
        let mut fs = FixtureState::new();
        fs.channels.insert(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut own = FixtureState::new();
        own.channels.insert(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::MIDGROUND, BlendMode::Multiply),
        );
        let values = fs.cell_values(&red_cell(), Some(&own));
        assert_eq!(values, vec![("red".to_string(), 0.5)]);
    }

    #[test]
    fn cell_values_a_cell_only_channel_takes_the_cells_value() {
        let fs = FixtureState::new(); // fixture has no "red" of its own
        let mut own = FixtureState::new();
        own.channels.insert(
            "red".to_string(),
            ChannelState::new(0.75, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let values = fs.cell_values(&red_cell(), Some(&own));
        assert_eq!(values, vec![("red".to_string(), 0.75)]);
    }

    /// A level on the cell's own state dims the cell's colour, on a cell
    /// with no dimmer, through the composite.
    #[test]
    fn a_cells_own_level_dims_its_colour() {
        let fixture = two_cell_fixture();
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let mut own = FixtureState::new();
        own.set_channel(
            "_intensity".to_string(),
            ChannelState::new(0.0, EffectLayer::MIDGROUND, BlendMode::Multiply),
        );
        let cmds = fs.to_dmx_commands_with_cells(&fixture, |name| (name == "b").then_some(&own));
        assert_eq!(byte_at(&cmds, 10), 255, "cell a keeps the fixture's red");
        assert_eq!(byte_at(&cmds, 11), 0, "cell b is dimmed out");
    }

    #[test]
    fn a_fixture_level_channel_no_cell_owns_is_written_once() {
        let fixture = two_cell_fixture();
        let mut fs = FixtureState::new();
        fs.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        fs.set_channel(
            "dimmer".to_string(),
            ChannelState::new(1.0, EffectLayer::BACKGROUND, BlendMode::Replace),
        );
        let cmds = fs.to_dmx_commands_with_cells(&fixture, |_| None);
        let dimmer_cmds: Vec<_> = cmds.iter().filter(|c| c.channel == 12).collect();
        assert_eq!(
            dimmer_cmds.len(),
            1,
            "dimmer written exactly once: {cmds:?}"
        );
        assert_eq!(dimmer_cmds[0].value, 255);
    }
}
