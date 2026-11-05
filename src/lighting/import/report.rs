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

//! What an import reports: light, so the web UI's WebAssembly module
//! compiles it as it is.

use serde::Serialize;

/// What an import did, for reporting: the fixture the user now has, in
/// their terms, and the files behind it for the CLI and MVR import.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Debug, Serialize)]
pub struct GdtfImport {
    /// The fixture type's name — what venues and shows call it.
    pub type_name: String,
    /// The archive's own fixture name.
    pub fixture: String,
    pub manufacturer: String,
    /// How many modes the archive has; a venue fixture may use any it can
    /// drive.
    pub modes: usize,
    /// Where the archive is, project-relative.
    pub archive: String,
    /// The archive was already in the project as `type_name`: nothing was
    /// written.
    pub already_imported: bool,
    /// The name the archive's fixture name would have given, when another
    /// fixture already has it and the archive's file stem was added.
    pub renamed_from: Option<String>,
    /// Modes mtrack cannot drive, and why.
    pub refused_modes: Vec<RefusedMode>,
    /// mtrack's record of the type, project-relative, when one was written
    /// (a name given, or one pinned on a collision).
    pub fixture_file: Option<String>,
}

/// A mode of an archive that cannot be imported, and why.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Debug, Serialize, PartialEq)]
pub struct RefusedMode {
    pub mode: String,
    pub reason: String,
}
