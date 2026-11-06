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

//! What a channel carries, read off its name: a level or colour going dark
//! takes to 0, where a head points, or a setting. The engine's crossfades
//! and its fade to dark, the snapshots and the web UI's stage views all
//! read a channel by these.

/// How a channel crosses a crossfade between two engines' frames
/// (the engine's `crossfade_commands`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CrossKind {
    /// Intensity, colour, zoom, focus: fade.
    Fade,
    /// Pan and tilt: fade, holding a side's position where the other side
    /// drives none.
    Position,
    /// Anything whose in-between values are other settings: switch at the
    /// midpoint.
    Snap,
}

/// Whether a channel named `channel` is a level or a colour, which going
/// dark takes to 0 -- not a position, an optic (zoom, focus, iris, frost)
/// or a setting, which it leaves as they are.
pub(crate) fn fades_to_dark(channel: &str) -> bool {
    matches!(CrossKind::of(channel), CrossKind::Fade)
        && !matches!(base_channel(channel), "zoom" | "focus" | "iris" | "frost")
}

/// A channel's name without its section (`dimmer:background`, as GDTF
/// profiles with several emitters name theirs) or its fine byte.
pub(crate) fn base_channel(channel: &str) -> &str {
    let base = channel.split(':').next().unwrap_or(channel);
    base.strip_suffix("_fine").unwrap_or(base)
}

/// Whether a channel carries where a head points -- pan or tilt, a section
/// (`pan:1`) or fine byte of either -- rather than what it shows.
pub(crate) fn is_position_channel(channel: &str) -> bool {
    matches!(CrossKind::of(channel), CrossKind::Position)
}

impl CrossKind {
    pub(crate) fn of(channel: &str) -> CrossKind {
        match base_channel(channel) {
            "pan" | "tilt" => CrossKind::Position,
            "dimmer" | "red" | "green" | "blue" | "white" | "warm_white" | "cool_white" | "uv"
            | "amber" | "lime" | "cyan" | "magenta" | "yellow" | "indigo" | "zoom" | "focus"
            | "iris" | "frost" | "ct" | "cto" | "ctb" | "color_temp"
            // GDTF's amber and lime, as their attributes come through.
            | "coloradd_ry" | "coloradd_gy" => CrossKind::Fade,
            // GDTF's other additive and RGB colour attributes, likewise.
            base if base.starts_with("coloradd_") || base.starts_with("colorrgb_") => {
                CrossKind::Fade
            }
            _ => CrossKind::Snap,
        }
    }
}

/// Whether a snapshot channel carries a level that lights the rig -- one
/// going dark takes to 0, as the engine has it (`fades_to_dark`) -- rather
/// than where a head points, an optic or a setting. The channel's name is
/// read under the `#n` a snapshot names a ganged channel's mirrors by.
pub fn is_level_channel(name: &str) -> bool {
    let base = match name.split_once('#') {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    };
    fades_to_dark(base)
}

/// Whether a fixture whose channels hold `values` is lit: some level
/// channel is above 0 (`is_level_channel`). The rig is dark when no
/// fixture is.
pub fn is_lit<'a>(values: impl IntoIterator<Item = (&'a str, f64)>) -> bool {
    values
        .into_iter()
        .any(|(name, value)| value > 0.0 && is_level_channel(name))
}
