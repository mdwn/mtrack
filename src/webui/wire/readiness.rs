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

//! `GET /api/lighting/readiness`: the facts behind the Lighting overview's
//! checks. The facts only -- every rule that makes a verdict of them is the
//! page's.

use serde::Serialize;

use crate::lighting::findings::{LibraryFinding, VenueError, Warning};

/// The readiness facts for the player's running profile.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Readiness {
    /// Whether the player has a DMX engine.
    pub dmx: bool,
    /// The current venue; `null` without one.
    pub venue: Option<ReadinessVenue>,
    /// The current venue does not load: one fixture that cannot be driven
    /// fails the whole venue, and then no fixture lights.
    pub venue_error: Option<VenueError>,
    /// Fixtures patched over part of each other's addresses
    /// (`patch-overlap`) or past address 512 (`patch-overrun`).
    pub patch_warnings: Vec<Warning>,
    pub fixture_types: ReadinessTypes,
    /// The groups the shows target, with what each finds.
    pub groups: Vec<ReadinessGroup>,
    /// Every song with lighting, its files and what lint says of them, by
    /// song.
    pub shows: Vec<ReadinessShow>,
    pub output: ReadinessOutput,
}

/// The current venue's size.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessVenue {
    pub name: String,
    pub fixtures: usize,
    /// How many of them the venue places on the stage.
    pub placed: usize,
    pub focus_points: Vec<String>,
}

/// The fixture types the venue uses, those it cannot, and the library's
/// own findings.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessTypes {
    pub in_use: Vec<String>,
    pub unresolved: Vec<UnresolvedFixture>,
    pub library: Vec<LibraryFinding>,
}

/// A fixture whose type or mode does not resolve, and why.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UnresolvedFixture {
    pub fixture: String,
    #[serde(rename = "type")]
    pub fixture_type: String,
    pub reason: String,
}

/// A group the shows target.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessGroup {
    pub name: String,
    /// How many fixtures it finds in the current venue.
    pub fixtures: usize,
    /// The songs whose shows use it.
    pub songs: Vec<String>,
}

/// A song's lighting.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessShow {
    pub song: String,
    pub files: Vec<String>,
    /// Why the song's show does not load (the song does not, then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub warnings: Vec<Warning>,
}

/// The DMX universes the venue uses, those with no output, and what olad
/// says of them.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessOutput {
    pub universes: Vec<u16>,
    pub unconfigured: Vec<u16>,
    /// `null` when nothing was asked.
    pub olad: Option<OladPatch>,
}

/// Whether olad answered, and the universes it has no output patched for.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OladPatch {
    pub reachable: bool,
    pub unpatched: Vec<u16>,
}
