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

//! The MVR import and export API's payloads (`/api/lighting/mvr`), and the
//! aim points a venue gains from its fixed fixtures.

use serde::Serialize;

use crate::lighting::findings::VenueError;
use crate::lighting::geometry::Vec3;
use crate::lighting::mvr_types::{MvrExport, MvrImport, MvrPlan, SceneView};

/// `POST /api/lighting/mvr/inspect`: what an import would do, and the file
/// seen from above.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Debug)]
pub struct MvrInspection {
    pub file_name: String,
    pub report: MvrPlan,
    pub scene: SceneView,
}

/// `POST /api/lighting/mvr/import`: the plan (`write` false), or what the
/// import did (`write` true), with the venue's reload and its load failure.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Debug)]
pub struct MvrImportAnswer {
    pub write: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<MvrPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<MvrImport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reloaded: Option<bool>,
    /// After a write: the current venue no longer loads, and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub venue_error: Option<Option<VenueError>>,
}

/// `GET /api/lighting/mvr/export/summary`: what an export would hold, and
/// the name a download would carry.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Debug)]
pub struct MvrExportSummary {
    #[serde(flatten)]
    pub export: MvrExport,
    pub file_name: String,
}

/// `POST /api/lighting/mvr/export/keep`: where the kept copy is.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Debug)]
pub struct MvrKept {
    pub kept: String,
}

/// `POST /api/lighting/venues/{name}/aim-points`: the focus points made.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Debug)]
pub struct AimPoints {
    pub created: Vec<AimPoint>,
    pub reloaded: bool,
    /// The venue file's version after the change.
    pub version: String,
    /// The current venue no longer loads after the change.
    pub venue_error: Option<VenueError>,
}

/// A focus point made where a fixed fixture's rest beam meets the deck.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Debug)]
pub struct AimPoint {
    pub name: String,
    pub fixture: String,
    pub point: Vec3,
}
