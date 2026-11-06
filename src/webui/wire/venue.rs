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

//! The venue API's payloads (`/api/lighting/venues`).

use serde::Serialize;

use crate::lighting::findings::VenueError;
use crate::lighting::patch::{PatchOverlap, PatchOverrun};

/// `GET /api/lighting/venues/{name}/patch`: the addresses a venue's
/// fixtures occupy, read from the files, and the player's findings on them.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VenuePatch {
    /// By patch order.
    pub spans: Vec<PatchedFixture>,
    pub overlaps: Vec<PatchOverlap>,
    pub overruns: Vec<PatchOverrun>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PatchedFixture {
    pub fixture: String,
    pub universe: u16,
    pub address: u16,
    /// `null` when its type or mode does not load.
    pub footprint: Option<u16>,
    #[serde(rename = "type")]
    pub fixture_type: String,
    /// The mode the fixture line names; `null` is its type's default.
    pub mode: Option<String>,
}

/// A venue save's answer.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VenueSaved {
    /// Always `saved`.
    pub status: String,
    pub name: String,
    /// Whether it was the running engine's venue, and so re-read.
    pub reloaded: bool,
    pub version: String,
    /// The venue's load failure, when the save left the current venue
    /// failing.
    pub venue_error: Option<VenueError>,
    /// A broken file replaced at the person's word, kept aside: where it
    /// went, by its path in the venues directory.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub kept: Vec<String>,
}
