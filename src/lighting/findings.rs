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

//! What the lighting checks report -- lint's warnings and what a show asks
//! of a group, the fixture library's findings, a venue that will not
//! register: light, so the web UI's WebAssembly module compiles it as it is.

use serde::Serialize;

/// Something a show asks of a group: what an effect needs its fixtures to
/// have ([`Want::of`]), or a pixel fixture for `per: cell`.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Want {
    /// A `move`: pan or tilt channels.
    Move,
    /// A colour cue (static colour, cycle, rainbow): colour channels.
    Color,
    /// A `strobe`: a strobe channel, or a dimmer or colour channels, which
    /// the engine strobes in software.
    Strobe,
    /// `per: cell`: a pixel fixture.
    Cells,
    /// A `dimmer` or `pulse`: a dimmer channel or colour channels.
    Dimmer,
}

/// A non-fatal finding.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Warning {
    /// Stable identifier, so callers can filter without matching prose.
    pub kind: &'static str,
    pub message: String,
}

/// Something about the library a person should know: two archives stating
/// one name (`gdtf-name-collision`), or an archive that is not a readable
/// GDTF (`gdtf-unreadable`). Neither stops the other fixtures.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LibraryFinding {
    pub kind: &'static str,
    /// The archive's file name.
    pub file: String,
    pub message: String,
}

/// A venue that does not register: the fixture that fails it and why.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VenueError {
    pub venue: String,
    pub fixture: String,
    pub reason: String,
}
