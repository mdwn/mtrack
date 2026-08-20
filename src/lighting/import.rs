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

/// [`crate::util::write_file`] with the path and any deploy hint in the
/// error — a bare "Read-only file system (os error 30)" names no file and
/// no fix.
fn write(path: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    crate::util::write_file(path, contents)
        .map_err(|e| annotate(crate::util::WriteTarget::File(path), e))
}

/// [`crate::util::create_dir_all`] with the same annotation.
fn create_dir(path: &Path) -> Result<(), Box<dyn Error>> {
    crate::util::create_dir_all(path)
        .map_err(|e| annotate(crate::util::WriteTarget::Directory(path), e))
}

fn annotate(target: crate::util::WriteTarget<'_>, error: std::io::Error) -> Box<dyn Error> {
    let mut message = format!("could not write {}: {error}", target.path().display());
    if let Some(hint) = crate::util::write_failure_hint(target, &error) {
        message.push_str("\n\n");
        message.push_str(&hint);
    }
    message.into()
}

/// Imports one mode of a GDTF archive into a project. All validation runs
/// before anything is written; a refused import leaves the project
/// untouched.
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

    // Every check runs before anything is written: a refused import must
    // leave the project exactly as it found it.
    let archive_file_name: PathBuf = gdtf_path
        .file_name()
        .ok_or("GDTF path has no file name")?
        .into();
    let library_dir = project.join("lighting/library");
    let library_path = library_dir.join(&archive_file_name);
    let library_rel = format!("lighting/library/{}", archive_file_name.display());

    let fixture_dir = project.join(fixture_types_dir);
    let fixture_path = fixture_dir.join(format!("{}.fixture", fixture_filename_stem(&type_name)));
    if fixture_path.exists() {
        return Err(format!(
            "{} already exists; remove it or import with a different name",
            fixture_path.display()
        )
        .into());
    }

    // A same-named archive with the same bytes is a harmless re-import; one
    // with different bytes would silently re-source every .fixture that
    // already points at it. Refuse rather than corrupt.
    let mut replaced_archive = false;
    if library_path.exists() {
        let existing = std::fs::read(&library_path)
            .map_err(|e| format!("cannot read existing {}: {e}", library_path.display()))?;
        if existing != bytes {
            return Err(format!(
                "{library_rel} already exists with different content — other fixture \
                 types may reference it; rename the source file (the library filename \
                 follows it) or remove the existing archive deliberately",
            )
            .into());
        }
        replaced_archive = true;
    }

    create_dir(&library_dir)?;
    if !replaced_archive {
        write(&library_path, &bytes)?;
    }
    create_dir(&fixture_dir)?;
    let definition = format!(
        "# Imported from {} (\"{}\" by {}).\n\
         # Channels come from the GDTF; this file carries only overrides.\n\
         fixture_type \"{type_name}\"\n  from gdtf(\"{library_rel}\", mode \"{mode}\")\n{{\n}}\n",
        archive_file_name.display(),
        description.name,
        description.manufacturer,
    );
    write(&fixture_path, definition.as_bytes())?;

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
    fn a_conflicting_archive_is_refused_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        // First import from archive A.
        let a = write_synthetic_gdtf(dir.path());
        import_gdtf(
            &a,
            "8: RGBS",
            Some("Brick"),
            &project,
            "lighting/fixture_types",
        )
        .unwrap();
        let original = std::fs::read(project.join("lighting/library/synth.gdtf")).unwrap();

        // A different archive under the same basename must refuse — Brick's
        // .fixture points at that library path.
        let other_dir = dir.path().join("elsewhere");
        std::fs::create_dir_all(&other_dir).unwrap();
        let b = other_dir.join("synth.gdtf");
        std::fs::write(
            &b,
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                // Same shape, different bytes.
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                    .replace("Synth Brick", "Other Brick")
                    .as_bytes(),
            )]),
        )
        .unwrap();
        let err = import_gdtf(
            &b,
            "8: RGBS",
            Some("Other"),
            &project,
            "lighting/fixture_types",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("different content"), "{err}");
        assert_eq!(
            std::fs::read(project.join("lighting/library/synth.gdtf")).unwrap(),
            original,
            "a refused import must not touch the library archive"
        );
        assert!(!project
            .join("lighting/fixture_types/other.fixture")
            .exists());

        // The same bytes under a new type name are a harmless re-import.
        let report = import_gdtf(
            &a,
            "8: RGBS",
            Some("Twin"),
            &project,
            "lighting/fixture_types",
        )
        .unwrap();
        assert!(report.replaced_archive);
    }

    #[test]
    fn a_name_collision_leaves_the_archive_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let gdtf = write_synthetic_gdtf(dir.path());
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        import_gdtf(
            &gdtf,
            "8: RGBS",
            Some("Brick"),
            &project,
            "lighting/fixture_types",
        )
        .unwrap();

        // Clobber the library copy so any rewrite would be visible.
        let library = project.join("lighting/library/synth.gdtf");
        let before = b"sentinel".to_vec();
        std::fs::write(&library, &before).unwrap();

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
        assert_eq!(
            std::fs::read(&library).unwrap(),
            before,
            "the name-collision check must run before any write"
        );
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
