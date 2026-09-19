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

//! GDTF (General Device Type Format) parsing and distillation.
//!
//! A `.gdtf` file is a zip archive whose `description.xml` describes a
//! fixture: modes, channels, functions, physical data. This module reads the
//! subset mtrack consumes and distills one mode into a [`FixtureType`] — the
//! datasheet-typable control data. Everything else (wheels, matrix template
//! channels, 3D models, emitters, protocols) is skipped, loudly, in the
//! distillation warnings.
//!
//! GDTF files are untrusted input — a venue's file arrives by email from a
//! stranger — so the archive layer enforces hard caps and never extracts to
//! disk, and the XML layer caps nesting depth and performs no DTD or entity
//! expansion. Per the venue-exchange design, parsing happens at import or
//! prewarm time only, never at show time.
//!
//! Nothing in the player runtime calls this module yet: it ships ahead of
//! the importer (P1a) with its consumers being tests, and the import
//! surface arrives together with the referential fixture syntax.

mod archive;
mod description;
mod distiller;
mod rig;

#[cfg(test)]
pub(crate) use archive::tests::build_zip;
#[cfg(test)]
pub(crate) use description::tests::SYNTHETIC_DESCRIPTION;

pub use archive::{list_model_files, read_assets, read_description_xml, Assets};
pub use description::{
    parse_description, BeamData, Description, GeometryKind, GeometryNode, Matrix4, Model, IDENTITY,
};
pub use distiller::{distill, mode_summaries, Distilled, ModeSummary};
pub use rig::{
    aim_calibration, beam_direction, beam_ray, distill_rig, RigBeam, RigModel, RigNode, RigRole,
    RigShape, DEFAULT_BEAM_ANGLE, RIG_VERSION,
};

use std::error::Error;
use std::fmt;

/// An error from GDTF parsing or distillation.
#[derive(Debug)]
pub struct GdtfError(String);

impl GdtfError {
    pub(crate) fn new(message: impl Into<String>) -> GdtfError {
        GdtfError(message.into())
    }
}

impl fmt::Display for GdtfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for GdtfError {}

/// Parses a GDTF archive's description into the spec-shaped subset mtrack
/// consumes. The bytes are the whole `.gdtf` file.
pub fn parse_archive(bytes: &[u8]) -> Result<Description, GdtfError> {
    let xml = read_description_xml(bytes)?;
    parse_description(&xml)
}

/// How a requested mode name was matched against a description.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeMatch {
    /// The mode's name as the GDTF spells it — what a `.fixture` file pins.
    pub name: String,
    /// Whether the match needed normalization (case, whitespace,
    /// punctuation) — a console-export drift worth reporting.
    pub normalized: bool,
}

/// Finds the mode a reference names. The spec requires an exact name, but
/// console exports drift: a normalized comparison (case, whitespace and
/// punctuation folded) is tried second and reported as such. Anything else
/// is an error listing the candidates — mode selection is human input, and
/// guessing wrong silently patches the wrong personality.
pub fn match_mode(description: &Description, requested: &str) -> Result<ModeMatch, GdtfError> {
    if let Some(mode) = description.modes.iter().find(|m| m.name == requested) {
        return Ok(ModeMatch {
            name: mode.name.clone(),
            normalized: false,
        });
    }
    let wanted = normalize_mode_name(requested);
    let mut folded = description
        .modes
        .iter()
        .filter(|m| normalize_mode_name(&m.name) == wanted);
    let candidates = || {
        description
            .modes
            .iter()
            .map(|m| format!("\"{}\"", m.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (folded.next(), folded.next()) {
        (Some(mode), None) => Ok(ModeMatch {
            name: mode.name.clone(),
            normalized: true,
        }),
        (Some(first), Some(second)) => Err(GdtfError::new(format!(
            "GDTF \"{}\": mode \"{requested}\" is ambiguous — it could be \"{}\" or \"{}\"; \
             name one exactly. Its modes are: {}",
            description.name,
            first.name,
            second.name,
            candidates()
        ))),
        _ => Err(GdtfError::new(format!(
            "GDTF \"{}\" has no mode matching \"{requested}\"; its modes are: {}",
            description.name,
            candidates()
        ))),
    }
}

/// Folds case, whitespace and punctuation so "8: RGBS" and "8 rgbs" agree.
fn normalize_mode_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod match_tests {
    use super::*;

    fn description() -> Description {
        let xml = read_description_xml(&build_zip(&[(
            "description.xml",
            SYNTHETIC_DESCRIPTION.as_bytes(),
        )]))
        .unwrap();
        parse_description(&xml).unwrap()
    }

    #[test]
    fn exact_then_normalized_then_error() {
        let description = description();
        let exact = match_mode(&description, "8: RGBS").unwrap();
        assert_eq!(exact.name, "8: RGBS");
        assert!(!exact.normalized);

        let drifted = match_mode(&description, "8 rgbs").unwrap();
        assert_eq!(drifted.name, "8: RGBS");
        assert!(drifted.normalized);

        let err = match_mode(&description, "Nope").unwrap_err().to_string();
        assert!(err.contains("no mode matching \"Nope\""), "{err}");
        assert!(err.contains("\"8: RGBS\""), "candidates are listed: {err}");
    }
}
