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

//! A song's `sections:` entries: light, so the web UI's WebAssembly module
//! compiles it as it is and the UI's section type is generated from it.

use serde::{Deserialize, Serialize};

/// A named section of a song defined by measure boundaries.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Section {
    /// Display name for this section (e.g., "verse", "chorus").
    pub name: String,
    /// Start measure (1-indexed, inclusive).
    pub start_measure: usize,
    /// End measure (1-indexed, exclusive).
    pub end_measure: usize,
    /// Start beat within the start measure (1-based, fractional allowed;
    /// omitted = 1, the measure boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_beat: Option<f64>,
    /// End beat within the end measure (1-based, fractional allowed;
    /// omitted = 1, keeping the end bound exclusive at the measure line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_beat: Option<f64>,
    /// Optional display color for the web UI timeline (hex "#rrggbb").
    /// Without one the UI rotates through its palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}
