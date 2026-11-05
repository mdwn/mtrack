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

//! What a fixture test answers about a mode and a frame: light, so the web
//! UI's WebAssembly module compiles it as it is. The rules are
//! `fixture_test`'s.

use serde::Serialize;

/// What a mode lets a test do.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ControlSupport {
    /// The mode mixes colour (RGB or CMY).
    pub color: bool,
    /// How the level is set, or `null`: the mode has no level at all.
    pub dimmer: Option<DimmerControl>,
    /// The strobe's rate range in Hz, when the mode has a strobe channel.
    pub strobe: Option<HzRange>,
    pub pan: Option<DegreeRange>,
    pub tilt: Option<DegreeRange>,
    /// Extra colour levels the mode has.
    pub levels: Vec<String>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct HzRange {
    pub min_hz: f64,
    pub max_hz: f64,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DegreeRange {
    pub min: f64,
    pub max: f64,
}

/// One channel of the fixture as a test frame sets it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FrameChannel {
    /// 1-based offset within the fixture.
    pub offset: u16,
    /// The DMX address it lands on.
    pub address: u16,
    pub name: String,
    pub value: u8,
    /// Set by a raw value rather than a control.
    pub raw: bool,
}

/// How a test sets a mode's level.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DimmerControl {
    /// A dimmer channel.
    Channel,
    /// No dimmer channel: the level scales the colour.
    Folded,
}
