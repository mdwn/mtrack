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

//! MVR (My Virtual Rig) parsing.
//!
//! An `.mvr` file is how a venue's patch arrives: a zip whose
//! `GeneralSceneDescription.xml` lists the patched fixtures — name, GDTF
//! reference and mode, universe/address, 3D transform — with the referenced
//! GDTF archives embedded beside it. This module reads the subset mtrack
//! consumes (venue-exchange design §6); scenery, trusses, and everything
//! else in the scene graph is passed over.
//!
//! Coordinate facts, verified against the MVR specification: the scene is
//! right-handed Z-up in **millimeters**, and the origin is wherever the
//! authoring console put it — conversion to mtrack's stage convention
//! (meters, downstage-center origin) plus the re-origin step is import-time
//! work, not parsing work, and lives with the importer.
//!
//! MVR files are untrusted input — a stranger emails one to the band — so
//! the archive layer enforces hard caps and never extracts to disk, and the
//! XML layer caps nesting depth with no DTD or entity expansion.
//!
//! Nothing in the player runtime calls this module yet: the import surface
//! and the `.venue` syntax arrive together in the next phase slice.

mod archive;
mod scene;

pub use archive::{list_gdtf_entries, read_gdtf_entry, read_scene_xml};
pub use scene::{parse_scene, Matrix, MvrFixture, Scene};

use std::error::Error;
use std::fmt;

/// An error from MVR parsing.
#[derive(Debug)]
pub struct MvrError(String);

impl MvrError {
    pub(crate) fn new(message: impl Into<String>) -> MvrError {
        MvrError(message.into())
    }
}

impl fmt::Display for MvrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for MvrError {}

/// Parses an MVR archive's scene into the consumed subset. The bytes are
/// the whole `.mvr` file.
pub fn parse_archive(bytes: &[u8]) -> Result<Scene, MvrError> {
    let xml = read_scene_xml(bytes)?;
    parse_scene(&xml)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full exchange chain: an MVR embedding a GDTF, walked the way the
    /// importer will — scene → spec reference → embedded archive → distilled
    /// fixture type.
    #[test]
    fn scene_to_embedded_gdtf_to_distilled_fixture() {
        let gdtf_bytes = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )]);
        let mvr_bytes = crate::lighting::gdtf::build_zip(&[
            (
                "GeneralSceneDescription.xml",
                scene::tests::SYNTHETIC_SCENE.as_bytes(),
            ),
            ("Astera_PB15.gdtf", gdtf_bytes.as_slice()),
        ]);

        let scene = parse_archive(&mvr_bytes).unwrap();
        let brick = &scene.fixtures[0];
        assert_eq!(brick.gdtf_spec.as_deref(), Some("Astera_PB15.gdtf"));

        let embedded = read_gdtf_entry(&mvr_bytes, brick.gdtf_spec.as_deref().unwrap()).unwrap();
        let description = crate::lighting::gdtf::parse_archive(&embedded).unwrap();
        let distilled = crate::lighting::gdtf::distill(
            &description,
            brick.gdtf_mode.as_deref().unwrap(),
            &brick.name,
        )
        .unwrap();
        assert_eq!(distilled.fixture_type.channels().get("strobe"), Some(&4));
        assert_eq!(distilled.fixture_type.strobe_dmx_offset(), Some(7));
        // The patch and transform ride alongside for the venue seeder.
        assert_eq!(brick.addresses.first(), Some(&(1, 1)));
        assert_eq!(brick.matrix.unwrap().o, [-2000.0, 3500.0, 4200.0]);
    }
}
