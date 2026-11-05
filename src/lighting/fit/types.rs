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

//! What the fit page and MCP's `suggest_group_tags` say: light, so the web
//! UI's WebAssembly module compiles it as it is. The rules that make it are
//! `fit`'s.

use serde::Serialize;

use crate::lighting::findings::Want;
use crate::lighting::geometry::Vec3;

/// The tags a group's constraints ask of a fixture.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Needs {
    /// Every one must be present.
    pub all_of: Vec<String>,
    /// At least one must be present.
    pub any_of: Vec<String>,
    /// Preferred, never required.
    pub prefer: Vec<String>,
}

/// How high a fixture hangs.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Height {
    /// z < 0.5 m.
    Deck,
    /// z < 2.5 m.
    Low,
    /// Above.
    Truss,
}

/// How far upstage a fixture hangs, by thirds of the venue's y-extent.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Downstage,
    Mid,
    Upstage,
}

/// One cluster of like fixtures: the same type, hanging in the same place.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Cluster {
    pub fixtures: Vec<String>,
    #[serde(rename = "type")]
    pub fixture_type: String,
    /// The place in words ("upstage truss"); absent when unplaced.
    #[serde(rename = "where")]
    pub place: Option<String>,
    pub height: Option<Height>,
    pub depth: Option<Depth>,
}

/// Why a cluster is the suggestion.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Reason {
    pub count: usize,
    #[serde(rename = "type")]
    pub fixture_type: String,
    #[serde(rename = "where")]
    pub place: Option<String>,
    pub height: Option<Height>,
    pub depth: Option<Depth>,
    /// What the shows ask of the group, which the cluster can do.
    pub can: Vec<Want>,
}

/// Fixtures and the tags that would put them in a group.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub fixtures: Vec<String>,
    pub tags: Vec<String>,
    pub reason: Reason,
}

/// The answer for one group.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Fit {
    pub suggestion: Option<Suggestion>,
    pub others: Vec<Cluster>,
    /// With no suggestion: the wants no fixture in the venue meets.
    pub unmet: Vec<Want>,
    /// With no suggestion and every want met by *some* fixture: none meets them
    /// all at once.
    pub unmet_together: bool,
}

/// Everything the fit page and `suggest_group_tags` say, gathered from the
/// running player.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FitReport {
    /// `None` when the player has no DMX engine or no current venue.
    pub venue: Option<FitVenue>,
    /// One entry per group any show targets, in name order.
    pub groups: Vec<FitGroup>,
    /// The focus points the shows aim at that the venue lacks.
    pub focus_points_wanted: Vec<FocusWanted>,
    pub output: FitOutput,
}

/// The current venue's fixtures, as the fit page offers them.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FitVenue {
    pub name: String,
    pub fixtures: Vec<FitFixtureView>,
    pub focus_points: Vec<String>,
    /// Why the venue does not register (it lights nothing then), instead
    /// of an empty rig.
    pub error: Option<FitVenueError>,
}

/// A fixture of the venue, as the fit page offers it.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FitFixtureView {
    pub name: String,
    #[serde(rename = "type")]
    pub fixture_type: String,
    pub tags: Vec<String>,
    pub position: Option<Vec3>,
    /// What it can do, as the engine names it (`color`, `pan_tilt`, ...).
    pub capabilities: Vec<String>,
}

/// The fixture that fails the venue, when one does, and why.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FitVenueError {
    pub fixture: Option<String>,
    pub reason: String,
}

/// A group the shows target: what it needs, finds and is asked, and the
/// suggestion that would satisfy it.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FitGroup {
    pub name: String,
    /// Whether the running profile declares the group.
    pub defined: bool,
    pub needs: Needs,
    /// The fixtures it finds in the current venue now.
    pub fixtures: Vec<String>,
    pub songs: Vec<String>,
    pub wants: Vec<Want>,
    #[serde(flatten)]
    pub fit: Fit,
}

/// A focus point the shows aim at that the venue lacks.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FocusWanted {
    pub name: String,
    pub songs: Vec<String>,
}

/// The universes the venue streams to that have no output, or that olad
/// has not patched.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FitOutput {
    pub unconfigured: Vec<u16>,
    pub unpatched: Vec<u16>,
    /// Whether olad's web server answered; `null` when nothing was asked.
    pub reachable: Option<bool>,
    pub ola_http_port: Option<u16>,
}
