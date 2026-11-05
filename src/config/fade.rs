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

//! A transition's fade as every surface reports it: light, so the web UI's
//! WebAssembly module compiles it as it is.

use serde::Serialize;

/// A fade's curve.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    /// Quick in, slow out: the level holds up through the fade.
    EqualPower,
    /// A straight ramp in amplitude.
    Linear,
}

/// A fade as configured, for status reports: a `null` `exclude` is the
/// default (a fade-out's `loop_crossfade_cut`, a fade-in's none), as is a
/// `null` `lights` (the length).
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FadeStatus {
    pub length: String,
    pub curve: FadeCurve,
    pub exclude: Option<Vec<String>>,
    pub lights: Option<String>,
}
