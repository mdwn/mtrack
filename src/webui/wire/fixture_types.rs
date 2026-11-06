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

//! The fixture type API's payloads (`/api/lighting/fixture-types`, the GDTF
//! pages), and the venue listing's.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::lighting::findings::VenueError;
use crate::lighting::fixture_data::{FixtureType, MovementLimits, StrobeCurve, Venue};
use crate::webui::wire::messages::{FixturePlacement, StageVenue};

/// A file in a lighting directory that could not be parsed, reported
/// beside the ones that could.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LightingFileError {
    pub file: String,
    pub error: String,
}

/// `GET /api/lighting/fixture-types`: every fixture type, by name, and the
/// files that did not read.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureTypeList {
    pub fixture_types: HashMap<String, FixtureTypeEntry>,
    pub errors: Vec<LightingFileError>,
}

/// A fixture type with the file it came from. The listing carries
/// `footprint` (and, for a GDTF type, `gdtf`); a single type's answer
/// carries `dsl` instead.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureTypeEntry {
    /// Whether it is a GDTF's, with no channels of its own.
    pub referential: bool,
    /// The addresses a fixture of a hand-written type occupies; `null` for
    /// a GDTF's, whose footprint is its mode's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Option<u16>>,
    pub rich: bool,
    pub fixture_type: FixtureType,
    /// The curve the type states (`strobe_curve:`), which the form editor
    /// sends back with a save; absent is automatic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strobe_curve: Option<StrobeCurve>,
    /// The type's file; `null` for a GDTF in the library with no record.
    pub file: Option<String>,
    pub extension: Option<String>,
    /// A GDTF type's archive in brief, for its card; `null` when the
    /// archive is missing or does not parse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gdtf: Option<Option<GdtfSummary>>,
    /// The type's file as written; `null` with no file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsl: Option<Option<String>>,
}

/// What a GDTF type's card says about its archive.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GdtfSummary {
    pub fixture: String,
    pub manufacturer: String,
    pub modes: usize,
    /// The drawn mode's first beam, as far as the archive states it.
    pub beam: Option<BeamBrief>,
    /// In the asset store, once a rig has been made for the type.
    pub thumbnail: Option<String>,
    /// Venue fixtures of this type.
    pub used_by: usize,
    /// The modes venue fixtures use, most-used first.
    pub in_use: Vec<ModeUse>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BeamBrief {
    #[serde(rename = "type")]
    pub beam_type: Option<String>,
    pub angle: Option<f64>,
}

/// How many venue fixtures use a mode.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ModeUse {
    pub mode: String,
    pub count: usize,
}

/// What a GDTF archive's modes let a show do.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GdtfInspection {
    pub modes: Vec<GdtfMode>,
}

/// One DMX mode of a GDTF archive. A mode that cannot be imported has
/// only its name, footprint and `refused`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GdtfMode {
    pub name: String,
    /// DMX addresses the mode occupies.
    pub footprint: u16,
    /// What a show can do in the mode, named as the fit endpoint names
    /// capabilities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<crate::lighting::fit::Capability>>,
    /// What a show can ask of a fixture that the mode can do
    /// (`Want::met_by`): dimming through colour counts as dimming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wants: Option<Vec<crate::lighting::findings::Want>>,
    /// Cells of a pixel mode (0 for a one-colour fixture).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cells: Option<usize>,
    /// The strobe channel's rate, when it carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strobe_range: Option<StrobeRange>,
    /// The channel map: `[address offset, name]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<(u16, String)>>,
    /// Why the mode cannot be imported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StrobeRange {
    pub min_hz: Option<f64>,
    pub max_hz: f64,
}

/// `GET /api/lighting/fixture-types/{name}/gdtf`: what a GDTF type's
/// archive holds, for its page. Paths are in the asset store.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureTypeGdtf {
    /// The archive, project-relative.
    pub archive: String,
    /// The rig model for the 3D view; `null` when none could be made.
    pub rig: Option<String>,
    pub thumbnail: Option<String>,
    /// The drawn mode's first beam; each figure `null` when the archive
    /// does not state it.
    pub beam: Option<GdtfBeam>,
    /// The archive's own description of the fixture: a stranger's text.
    pub about: Option<String>,
    /// The venues with fixtures of this type, each in patch order with the
    /// mode it is driven in, spelled as the archive spells it.
    pub venues: Vec<TypeVenue>,
    pub inspection: GdtfInspection,
}

/// A beam as the archive states it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GdtfBeam {
    #[serde(rename = "type")]
    pub beam_type: Option<String>,
    pub beam_angle: Option<f64>,
    pub field_angle: Option<f64>,
    pub luminous_flux: Option<f64>,
    pub color_temperature: Option<f64>,
    /// Watts.
    pub power: Option<f64>,
}

/// A venue's fixtures of a type.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TypeVenue {
    pub name: String,
    pub fixtures: Vec<TypeVenueFixture>,
}

/// A venue fixture and the mode it is driven in; `null` when that names no
/// mode of the archive.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TypeVenueFixture {
    pub name: String,
    pub mode: Option<String>,
}

/// The 409 for a venue or fixture type save or delete refused because the
/// file it would act on does not hold the name (`project_files::may_act_on`).
/// One that does not read carries its parse error: the web UI asks whether
/// to replace (or remove) it, and sends the request again with
/// `?replace_broken=<file>`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LightingFileRefusal {
    pub error: String,
    /// The file's name.
    pub file: String,
    /// Whether it does not read (and may be replaced).
    pub broken: bool,
    pub parse_error: Option<String>,
}

/// A deleted fixture type, and what became of its archive.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureTypeDeleted {
    /// Always `deleted`.
    pub status: String,
    pub name: String,
    pub archive_removed: Option<String>,
    /// Another fixture uses the archive, so it stays.
    pub archive_kept: bool,
}

/// `GET /api/lighting/fixture-types/{name}/settings`: what is the user's
/// about a GDTF fixture.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FixtureSettingsData {
    pub name: String,
    pub movement: MovementLimits,
    /// The curve the record states; `null` is automatic.
    pub strobe_curve: Option<StrobeCurve>,
    /// `null` when no mode has a strobe rate in hertz.
    pub strobe: Option<StrobeTable>,
    /// The settings' version, sent back as `If-Match`.
    pub version: String,
}

/// The GDTF's strobe function: the size of its own table (0: endpoints
/// only) and the curve used when none is stated.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StrobeTable {
    pub steps: usize,
    pub automatic: StrobeCurve,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct SettingsRequest {
    pub name: String,
    #[serde(default)]
    pub movement: MovementLimits,
    /// How a strobe rate maps onto the strobe channel; absent or `null` is
    /// automatic.
    #[serde(default)]
    pub strobe_curve: Option<StrobeCurve>,
    /// `false` answers what the save would do and writes nothing.
    pub write: bool,
    /// The venue files' versions the plan was made against, by file name;
    /// a save refuses when any has changed since.
    #[serde(default)]
    pub venue_versions: HashMap<String, String>,
}

/// What a settings save does (or, planned, would do).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SettingsSaved {
    pub version: String,
    /// A rename: every venue line that names the type, by venue.
    pub rename: Option<Rename>,
    /// The venue files read, by name, with their versions.
    pub venue_versions: BTreeMap<String, String>,
    pub venue_error: Option<VenueError>,
}

/// A fixture type renamed, and the venue lines that follow it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Rename {
    pub from: String,
    pub to: String,
    pub venues: Vec<RenamedLines>,
    pub lines: usize,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RenamedLines {
    pub venue: String,
    pub file: String,
    pub lines: usize,
}

/// `GET /api/lighting/venues`: the venues that parsed, the files that did
/// not, and each file's version. Without a venues directory, only the
/// (empty) venues.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VenueList {
    pub venues: HashMap<String, Venue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<LightingFileError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versions: Option<HashMap<String, String>>,
}

/// `GET /api/lighting/venues/{name}`: a venue, its file and its version.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VenueDetail {
    pub venue: Venue,
    pub dsl: String,
    pub version: String,
}

/// `GET /api/lighting/venues/{name}/scene`: a venue from its files, as the
/// 3D view draws it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct VenueScene {
    pub fixtures: BTreeMap<String, SceneFixture>,
    pub venue: StageVenue,
}

/// A fixture as the 3D view draws it from the files.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SceneFixture {
    #[serde(flatten)]
    pub placement: FixturePlacement,
    pub mode: Option<String>,
}
