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

#[cfg(test)]
pub(crate) use archive::tests::build_zip;
#[cfg(test)]
pub(crate) use description::tests::SYNTHETIC_DESCRIPTION;

pub use archive::read_description_xml;
pub use description::{parse_description, Description};
pub use distiller::{distill, mode_summaries, Distilled, ModeSummary};

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
