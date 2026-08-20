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

//! GDTF import: the one implementation behind the CLI command and the MCP
//! tool.
//!
//! An import copies the archive into `<project>/lighting/library/`, writes a
//! GDTF-referential `.fixture` definition, and warms the expansion cache
//! through the same code path the player's loader takes — so a successful
//! import is, by construction, a fixture type that will load.

use std::error::Error;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::gdtf;
use super::system::LightingSystem;

/// What an import did, for reporting: files written, the channels the
/// fixture ended up with, and everything the distiller skipped or guessed.
#[derive(Debug, Serialize)]
pub struct GdtfImport {
    /// The imported fixture type's name.
    pub type_name: String,
    /// The distilled mode.
    pub mode: String,
    /// Where the archive landed, project-relative.
    pub archive: String,
    /// Whether an existing library archive of the same name was replaced.
    pub replaced_archive: bool,
    /// The written `.fixture` definition, project-relative.
    pub fixture_file: String,
    /// The resolved channels: offset → name, sorted by offset.
    pub channels: Vec<(u16, String)>,
    /// Distillation warnings — what was skipped or approximated.
    pub warnings: Vec<String>,
}

/// A fixture-type name reduced to a safe filename stem.
fn fixture_filename_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "fixture".to_string()
    } else {
        out
    }
}

/// Imports one mode of a GDTF archive into a project. Nothing is written
/// until the mode has distilled successfully.
pub fn import_gdtf(
    gdtf_path: &Path,
    mode: &str,
    name: Option<&str>,
    project: &Path,
    fixture_types_dir: &str,
) -> Result<GdtfImport, Box<dyn Error>> {
    let bytes = std::fs::read(gdtf_path)
        .map_err(|e| format!("cannot read GDTF archive {}: {e}", gdtf_path.display()))?;
    let description = gdtf::parse_archive(&bytes)?;
    let type_name = name.unwrap_or(&description.name).to_string();

    // Distill up front so a bad mode or an unsupported (pixel) mode fails
    // before anything lands on disk.
    let distilled = gdtf::distill(&description, mode, &type_name)?;

    let library_dir = project.join("lighting/library");
    std::fs::create_dir_all(&library_dir)?;
    let archive_file_name: PathBuf = gdtf_path
        .file_name()
        .ok_or("GDTF path has no file name")?
        .into();
    let library_path = library_dir.join(&archive_file_name);
    let library_rel = format!("lighting/library/{}", archive_file_name.display());
    let replaced_archive = library_path.exists();
    std::fs::write(&library_path, &bytes)?;

    let fixture_dir = project.join(fixture_types_dir);
    std::fs::create_dir_all(&fixture_dir)?;
    let fixture_path = fixture_dir.join(format!("{}.fixture", fixture_filename_stem(&type_name)));
    if fixture_path.exists() {
        return Err(format!(
            "{} already exists; remove it or import with a different name",
            fixture_path.display()
        )
        .into());
    }
    let definition = format!(
        "# Imported from {} (\"{}\" by {}).\n\
         # Channels come from the GDTF; this file carries only overrides.\n\
         fixture_type \"{type_name}\"\n  from gdtf(\"{library_rel}\", mode \"{mode}\")\n{{\n}}\n",
        archive_file_name.display(),
        description.name,
        description.manufacturer,
    );
    std::fs::write(&fixture_path, &definition)?;

    // Warm the cache and prove the written definition loads, through the
    // exact expansion path the player takes at startup.
    let written = super::parser::parse_fixture_types(&definition)?;
    let fixture_type = written
        .get(&type_name)
        .ok_or("written definition did not parse back")?;
    let expanded = LightingSystem::expand_referential(&type_name, fixture_type, project)?;

    let mut channels: Vec<(u16, String)> = expanded
        .channels()
        .iter()
        .map(|(name, offset)| (*offset, name.clone()))
        .collect();
    channels.sort();

    Ok(GdtfImport {
        type_name,
        mode: mode.to_string(),
        archive: library_rel,
        replaced_archive,
        fixture_file: format!(
            "{}/{}",
            fixture_types_dir.trim_end_matches('/'),
            fixture_path.file_name().unwrap_or_default().display()
        ),
        channels,
        warnings: distilled.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_synthetic_gdtf(dir: &Path) -> PathBuf {
        let path = dir.join("synth.gdtf");
        std::fs::write(
            &path,
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .unwrap();
        path
    }

    #[test]
    fn imports_write_archive_fixture_and_cache() {
        let dir = tempfile::tempdir().unwrap();
        let gdtf = write_synthetic_gdtf(dir.path());
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        let report = import_gdtf(
            &gdtf,
            "8: RGBS",
            Some("Brick"),
            &project,
            "lighting/fixture_types",
        )
        .unwrap();

        assert_eq!(report.type_name, "Brick");
        assert!(!report.replaced_archive);
        assert_eq!(report.fixture_file, "lighting/fixture_types/brick.fixture");
        assert_eq!(
            report.channels,
            vec![
                (1, "red".to_string()),
                (2, "green".to_string()),
                (3, "blue".to_string()),
                (4, "strobe".to_string()),
            ]
        );
        assert!(project.join("lighting/library/synth.gdtf").exists());
        assert!(project
            .join("lighting/fixture_types/brick.fixture")
            .exists());
        assert_eq!(
            std::fs::read_dir(project.join("lighting/.cache"))
                .unwrap()
                .count(),
            1,
            "import warms the expansion cache"
        );

        // Same name again refuses rather than clobbering.
        let err = import_gdtf(
            &gdtf,
            "8: RGBS",
            Some("Brick"),
            &project,
            "lighting/fixture_types",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("already exists"), "{err}");
    }

    #[test]
    fn a_bad_mode_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let gdtf = write_synthetic_gdtf(dir.path());
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        let err = import_gdtf(
            &gdtf,
            "No Such Mode",
            None,
            &project,
            "lighting/fixture_types",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no mode named"), "{err}");
        assert!(!project.join("lighting").exists());
    }

    #[test]
    fn filename_stems() {
        assert_eq!(fixture_filename_stem("PB15 PixelBrick"), "pb15_pixelbrick");
        assert_eq!(fixture_filename_stem("Robe-Esprite"), "robe_esprite");
        assert_eq!(fixture_filename_stem("///"), "fixture");
    }
}
