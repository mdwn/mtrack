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

mod mvr;
mod report;
pub mod scene_view;

use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};

use super::gdtf;
use super::system::LightingSystem;

pub(crate) use mvr::to_stage;
pub use mvr::{
    import_mvr, import_mvr_bytes, inspect_mvr, inspect_mvr_bytes, HandEdit, MvrImport,
    MvrImportOptions, MvrKeep, MvrPlan, PlannedFixture, PlannedFixtureType, PlannedFocusPoint,
    RemovedFixture, VenueRefused,
};
pub use report::{GdtfImport, RefusedMode};

/// An archive of the same file name but different content is already in
/// the project's library. Its own error type so a caller can tell it from
/// a bad archive (the web API answers it with 409).
#[derive(Debug)]
pub struct LibraryClash(pub String);

impl std::fmt::Display for LibraryClash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for LibraryClash {}

/// An MVR's embedded GDTF whose record could not be claimed: the whole
/// import is refused (never a suffixed file), with what the user can do.
/// Its own type so a caller can answer it as the conflict it is.
#[derive(Debug)]
pub struct RecordRefused {
    /// The archive's file name, as the MVR names it.
    pub gdtf: String,
    /// The fixture types directory, project-relative.
    pub types_dir: String,
    pub refusal: super::files::Refusal,
}

impl std::fmt::Display for RecordRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use super::files::Refusal;
        let types = self.types_dir.trim_end_matches('/');
        let quoted = |names: &[String]| {
            names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match &self.refusal {
            Refusal::Taken { file, by } if by.is_empty() => write!(f, "{types}/{file} is there")?,
            Refusal::Taken { file, by } => write!(f, "{types}/{file} holds {}", quoted(by))?,
            Refusal::ClaimedFor { file, by } => write!(
                f,
                "{types}/{file} would also be the record of \"{by}\", another GDTF in this MVR"
            )?,
            Refusal::Declared { name, file, .. } => {
                write!(f, "{types}/{file} already declares \"{name}\"")?
            }
            Refusal::Library { name, archive } => {
                write!(f, "\"{name}\" is the GDTF library type in {archive}")?
            }
            Refusal::Unreadable { .. } => return write!(f, "{}", self.refusal),
        }
        if !matches!(self.refusal, Refusal::ClaimedFor { .. }) {
            f.write_str("; rename or move that file, or")?;
        } else {
            f.write_str(";")?;
        }
        write!(
            f,
            " import {} on its own first under a name of your own (`mtrack import-gdtf --name \
             <name>`, or the Fixtures page)",
            self.gdtf
        )
    }
}

impl Error for RecordRefused {}

/// A GDTF fixture name made a fixture type name: letters, digits, spaces,
/// hyphens and underscores, everything else dropped, runs of spaces
/// collapsed (a quote or brace would end the name in the file). Falls back
/// to `fixture` when nothing is left.
pub fn suggested_type_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .collect();
    let collapsed = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "fixture".to_string()
    } else {
        collapsed
    }
}

/// [`crate::util::write_file`] with the path and any deploy hint in the
/// error — a bare "Read-only file system (os error 30)" names no file and
/// no fix.
pub(crate) fn write(path: &Path, contents: &[u8]) -> Result<(), Box<dyn Error>> {
    crate::util::write_file(path, contents)
        .map_err(|e| annotate(crate::util::WriteTarget::File(path), e))
}

/// [`crate::util::create_dir_all`] with the same annotation.
pub(crate) fn create_dir(path: &Path) -> Result<(), Box<dyn Error>> {
    crate::util::create_dir_all(path)
        .map_err(|e| annotate(crate::util::WriteTarget::Directory(path), e))
}

fn annotate(target: crate::util::WriteTarget<'_>, error: std::io::Error) -> Box<dyn Error> {
    crate::util::write_failure_message(target, &error).into()
}

/// The record mtrack writes for a GDTF fixture: its name and archive. The
/// type is the whole archive (design §22): every venue fixture of it states
/// its own mode, so the record names none. Movement limits are added to the
/// body by the fixture page when set.
pub(crate) fn gdtf_definition(
    type_name: &str,
    library_rel: &str,
    archive_file_name: &str,
    description: &gdtf::Description,
) -> String {
    format!(
        "# Imported from {archive_file_name} (\"{}\" by {}).\n\
         # Written and kept by mtrack: channels come from the GDTF; this file\n\
         # carries the fixture's name and movement limits.\n\
         fixture_type \"{type_name}\"\n  from gdtf(\"{library_rel}\")\n{{\n}}\n",
        description.name, description.manufacturer,
    )
}

/// The fixture types already in a directory: name → the GDTF archive each
/// references (project-relative), `None` for a hand-written one. Files that
/// do not parse are skipped: they declare nothing this import could clash
/// with that the loader would accept.
pub fn existing_types(dir: &Path) -> std::collections::BTreeMap<String, Option<String>> {
    // The project's one reader of type files, subdirectories included: a
    // name a type in a subdirectory has is as taken as one at the top.
    super::project_files::type_files(dir)
        .items
        .into_iter()
        .map(|d| {
            let archive = d.item.source().map(|s| s.path.clone());
            (d.name, archive)
        })
        .collect()
}

/// The modes of an archive mtrack cannot drive, and why: the distiller's
/// refusal, or a name an earlier mode already has (a venue line names a
/// mode by name, so the repeat could never be chosen).
pub fn refused_modes(description: &gdtf::Description, type_name: &str) -> Vec<RefusedMode> {
    let mut out = Vec::new();
    for (index, mode) in description.modes.iter().enumerate() {
        let reason = if description.modes[..index]
            .iter()
            .any(|m| m.name == mode.name)
        {
            Some("another mode has this name, so it cannot be chosen".to_string())
        } else {
            gdtf::distill(description, &mode.name, type_name)
                .err()
                .map(|e| e.to_string())
        };
        if let Some(reason) = reason {
            out.push(RefusedMode {
                mode: mode.name.clone(),
                reason,
            });
        }
    }
    out
}

/// The mode a GDTF fixture is drawn in on its own page: the first the archive offers
/// that distils. The web UI's 3D view and thumbnail look its rig up by this
/// name, so the import warms exactly that.
pub fn first_drivable_mode(description: &gdtf::Description, type_name: &str) -> Option<String> {
    description
        .modes
        .iter()
        .find(|m| gdtf::distill(description, &m.name, type_name).is_ok())
        .map(|m| m.name.clone())
}

/// Imports a GDTF archive into a project (design §22.2). A GDTF in the
/// library is a fixture, so an import is a copy: the archive is validated
/// and put in `lighting/library/`, and it is a fixture type named from its
/// own fixture name that a venue fixture can use in any of its modes. The
/// same archive again changes nothing (`already_imported`); a file of the
/// same name with other content is refused ([`LibraryClash`]), since
/// fixtures may already depend on it. A record is written only to pin a
/// name: one given with `name`, or the `Name (stem)` a newcomer takes when
/// another fixture already has its name (so a later file that sorts first
/// cannot move it). All validation runs before anything is written.
pub fn import_gdtf(
    gdtf_path: &Path,
    name: Option<&str>,
    project: &Path,
    fixture_types_dir: &str,
) -> Result<GdtfImport, Box<dyn Error>> {
    let bytes = std::fs::read(gdtf_path)
        .map_err(|e| format!("cannot read GDTF archive {}: {e}", gdtf_path.display()))?;
    let archive_file_name = gdtf_path
        .file_name()
        .ok_or("GDTF path has no file name")?
        .to_string_lossy()
        .into_owned();
    import_gdtf_bytes(&bytes, &archive_file_name, name, project, fixture_types_dir)
}

/// A library archive with exactly these bytes, by file name.
fn library_copy_of(project: &Path, bytes: &[u8]) -> Option<String> {
    let dir = project.join(super::library::LIBRARY_DIR);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            super::library::is_archive(p)
                && std::fs::metadata(p).is_ok_and(|m| m.len() == bytes.len() as u64)
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .find(|p| crate::util::read_file(p).is_ok_and(|b| b == bytes))
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Where an archive goes in the library and what it will be called — the
/// one rule `import-gdtf`, the web import and MVR import share.
#[derive(Clone, Debug)]
pub struct LibraryEntry {
    /// The fixture type name venue lines use.
    pub name: String,
    /// The archive, project-relative.
    pub archive: String,
    /// The archive's file name in the library.
    pub file_name: String,
    /// The same bytes are already in the library: nothing to copy.
    pub existing: bool,
    /// The record to write, project-relative, when the name is pinned.
    pub record: Option<String>,
    /// The name the archive would have had, when it was taken.
    pub renamed_from: Option<String>,
}

/// What one import has given out so far, across its archives: the type
/// names, and the record files claimed.
#[derive(Debug, Default)]
pub struct Batch {
    pub names: HashSet<String>,
    pub records: super::files::Claimed,
}

/// Works out where an archive goes and what it is called, writing nothing.
/// `batch` holds what this batch already gave out (an MVR with several
/// GDTFs), and gains this one's.
pub fn place_in_library(
    project: &Path,
    fixture_dir: &Path,
    arrival_file_name: &str,
    bytes: &[u8],
    description: &gdtf::Description,
    name: Option<&str>,
    batch: &mut Batch,
) -> Result<LibraryEntry, Box<dyn Error>> {
    let Batch {
        names: claimed,
        records,
    } = batch;
    if Path::new(arrival_file_name).file_name() != Some(std::ffi::OsStr::new(arrival_file_name)) {
        return Err(
            format!("archive file name \"{arrival_file_name}\" is not a bare file name").into(),
        );
    }
    let types = existing_types(fixture_dir);
    let library = super::library::unrecorded_types(project, Some(fixture_dir));
    let copy = library_copy_of(project, bytes);
    let rel = |file: &str| super::library::library_file(project, file).1;

    // Already in the library: it is the fixture it already is.
    if let Some(file) = &copy {
        let archive = rel(file);
        let canonical = project.join(&archive).canonicalize().ok();
        let recorded_as = types.iter().find_map(|(type_name, at)| {
            at.as_deref()
                .and_then(|at| project.join(at).canonicalize().ok())
                .filter(|path| Some(path) == canonical.as_ref())
                .map(|_| type_name.clone())
        });
        let current = recorded_as.or_else(|| {
            library
                .types
                .iter()
                .find(|t| t.archive == archive)
                .map(|t| t.name.clone())
        });
        if let Some(current) = current {
            if name.is_none_or(|name| name == current) {
                claimed.insert(current.clone());
                return Ok(LibraryEntry {
                    name: current,
                    archive,
                    file_name: file.clone(),
                    existing: true,
                    record: None,
                    renamed_from: None,
                });
            }
        }
    }

    // Where it lands: the copy already there, or the file name it arrived
    // with — refused when that name holds other bytes.
    let file_name = match &copy {
        Some(file) => file.clone(),
        None => {
            if project.join(rel(arrival_file_name)).exists() {
                let users: Vec<&str> = types
                    .iter()
                    .filter(|(_, at)| at.as_deref() == Some(rel(arrival_file_name).as_str()))
                    .map(|(name, _)| name.as_str())
                    .chain(
                        library
                            .types
                            .iter()
                            .filter(|t| t.file_name == arrival_file_name)
                            .map(|t| t.name.as_str()),
                    )
                    .collect();
                let used_by = if users.is_empty() {
                    String::new()
                } else {
                    format!(" (that one is {})", users.join(", "))
                };
                let stem = Path::new(arrival_file_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("fixture");
                return Err(Box::new(LibraryClash(format!(
                    "a different GDTF called \"{arrival_file_name}\" is already in this \
                     project{used_by}. Rename your file (for example \"{stem}-2.gdtf\") and \
                     import it again, or delete that fixture first.",
                ))));
            }
            arrival_file_name.to_string()
        }
    };
    let archive = rel(&file_name);
    let taken = |candidate: &str, claimed: &HashSet<String>| {
        types.contains_key(candidate)
            || claimed.contains(candidate)
            || library
                .types
                .iter()
                .any(|t| t.name == candidate && t.archive != archive)
    };
    let (type_name, renamed_from, pinned) = match name {
        // Whether a name given is free is the record's claim's to say.
        Some(name) => {
            if claimed.contains(name) {
                return Err(format!(
                    "this import already names another fixture \"{name}\"; import it under \
                     another name"
                )
                .into());
            }
            (name.to_string(), None, true)
        }
        None => {
            let base = suggested_type_name(&description.name);
            if !taken(&base, claimed) {
                (base, None, false)
            } else {
                let stem = suggested_type_name(
                    Path::new(&file_name)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("gdtf"),
                );
                let with_stem = format!("{base} ({stem})");
                let mut candidate = with_stem.clone();
                let mut n = 2;
                while taken(&candidate, claimed) {
                    candidate = format!("{with_stem} #{n}");
                    n += 1;
                }
                (candidate, Some(base), true)
            }
        }
    };
    // A pinned name's record is a new file, claimed by the one rule for
    // new type files: at `<stem>.fixture`, refused (naming the file) when
    // the name is declared, another archive's library name, or its file
    // (in any case) is there or another of this batch's records. Never a
    // suffixed file beside it.
    let record = if pinned {
        let claim = records.record_for(fixture_dir, &archive, &type_name, &library.types)?;
        Some(
            claim
                .path()
                .strip_prefix(project)
                .unwrap_or(claim.path())
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };
    claimed.insert(type_name.clone());
    Ok(LibraryEntry {
        name: type_name,
        archive,
        file_name,
        existing: copy.is_some(),
        record,
        renamed_from,
    })
}

/// An import's library copies, written one by one before what goes with
/// them: each stands once written, as an import's library always has, and
/// [`Self::failed`] says which when something after fails.
#[derive(Debug, Default)]
pub struct LibraryCopies {
    /// Written so far, project-relative.
    pub written: Vec<String>,
}

impl LibraryCopies {
    /// Writes the new library file `file_name`.
    /// Writes the new library file `file_name`, made with `create_new` (its
    /// name was checked free): a write that fails leaves no partial copy
    /// for a later import to take for one ([`crate::util::write_file_tracked`]).
    pub fn write(
        &mut self,
        project: &Path,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        let (path, rel) = super::library::library_file(project, file_name);
        create_dir(path.parent().unwrap_or(project)).map_err(|e| self.failed(e))?;
        crate::util::write_file_tracked(&path, bytes, crate::util::WriteMode::CreateNew).map_err(
            |failure| {
                self.failed(annotate(
                    crate::util::WriteTarget::File(&path),
                    failure.error,
                ))
            },
        )?;
        self.written.push(rel);
        Ok(())
    }

    /// Writes a [`LibraryEntry`]'s copy, when it is new.
    pub fn write_entry(
        &mut self,
        project: &Path,
        entry: &LibraryEntry,
        bytes: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        if entry.existing {
            return Ok(());
        }
        self.write(project, &entry.file_name, bytes)
    }

    /// `error`, saying which copies were written before it, in the one
    /// wording for a failure's outcome ([`super::writes::saved_after_failure`]).
    pub fn failed(&self, error: Box<dyn Error>) -> Box<dyn Error> {
        format!(
            "{error}; {}",
            super::writes::saved_after_failure(&self.written, &[])
        )
        .into()
    }
}

/// A pinned [`LibraryEntry`]'s record, ready to be made.
#[derive(Debug)]
pub struct RecordWrite {
    pub path: PathBuf,
    pub text: String,
    /// The file as the user knows it: project-relative.
    pub shown: String,
}

impl RecordWrite {
    /// Makes the record's directory: before any library copy is written,
    /// so a failure here has written nothing.
    pub fn create_dir(&self) -> Result<(), Box<dyn Error>> {
        match self.path.parent() {
            Some(parent) => create_dir(parent),
            None => Ok(()),
        }
    }

    /// Makes the record among `changes`, with whatever goes with it.
    pub fn create(
        &self,
        changes: &mut super::writes::Changes,
    ) -> Result<(), super::writes::WriteError> {
        changes.create(&self.path, self.text.as_bytes(), self.shown.clone())
    }
}

/// The record a pinned [`LibraryEntry`] writes, its text proven to read
/// back; writing nothing. `None` when the name is not pinned.
pub fn record_to_write(
    project: &Path,
    entry: &LibraryEntry,
    description: &gdtf::Description,
) -> Result<Option<RecordWrite>, Box<dyn Error>> {
    let Some(record) = &entry.record else {
        return Ok(None);
    };
    let text = gdtf_definition(&entry.name, &entry.archive, &entry.file_name, description);
    // A record that does not read back is not written.
    let parsed = super::parser::parse_fixture_types(&text)?;
    if !parsed.contains_key(&entry.name) {
        return Err(format!("the record for \"{}\" did not read back", entry.name).into());
    }
    Ok(Some(RecordWrite {
        path: project.join(record),
        text,
        shown: record.clone(),
    }))
}

/// [`import_gdtf`] over in-memory bytes — the shape uploads arrive in.
/// `archive_file_name` names the archive inside `lighting/library/` and must
/// be a bare file name.
pub fn import_gdtf_bytes(
    bytes: &[u8],
    archive_file_name: &str,
    name: Option<&str>,
    project: &Path,
    fixture_types_dir: &str,
) -> Result<GdtfImport, Box<dyn Error>> {
    let description = gdtf::parse_archive(bytes)?;
    if description.modes.is_empty() {
        return Err("this GDTF has no DMX modes, so there is nothing to patch".into());
    }
    let fixture_dir = project.join(fixture_types_dir);
    let entry = place_in_library(
        project,
        &fixture_dir,
        archive_file_name,
        bytes,
        &description,
        name,
        &mut Batch::default(),
    )?;
    let already_imported = entry.existing && entry.record.is_none();
    if !already_imported {
        // Everything that does not need the copy first, so a failure before
        // it has written nothing.
        let record = record_to_write(project, &entry, &description)?;
        if let Some(record) = &record {
            record.create_dir()?;
        }
        let mut copies = LibraryCopies::default();
        copies.write_entry(project, &entry, bytes)?;
        if let Some(record) = &record {
            // Made new, as it was claimed; a failure takes back what it made,
            // and says the library copy stands.
            super::writes::Changes::all_or_nothing_keeping(copies.written, |changes| {
                record.create(changes)
            })?;
        }
        // The 3D view and the thumbnail draw a GDTF fixture in the first
        // mode it can drive: warm that rig now so they are instant.
        if let Some(drawn) = first_drivable_mode(&description, &entry.name) {
            let declared = super::library::declared(&super::library::LibraryType {
                name: entry.name.clone(),
                archive: entry.archive.clone(),
                file_name: entry.file_name.clone(),
                fixture: description.name.clone(),
                manufacturer: description.manufacturer.clone(),
                renamed_from: None,
            });
            LightingSystem::expand_mode(&entry.name, &declared, &drawn, project)?;
        }
    }
    Ok(GdtfImport {
        refused_modes: if already_imported {
            Vec::new()
        } else {
            refused_modes(&description, &entry.name)
        },
        type_name: entry.name,
        fixture: description.name.clone(),
        manufacturer: description.manufacturer.clone(),
        modes: description.modes.len(),
        archive: entry.archive,
        already_imported,
        renamed_from: entry.renamed_from,
        fixture_file: entry.record,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_bytes(name: &str) -> Vec<u8> {
        crate::testutil::lighting::synthetic_gdtf_of(
            &crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.replace("Synth Brick", name),
        )
    }

    use crate::testutil::lighting::write_synthetic_gdtf;

    fn project(dir: &Path) -> PathBuf {
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        project
    }

    const TYPES: &str = "lighting/fixture_types";

    #[test]
    fn an_import_is_a_copy_and_the_library_makes_it_a_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let gdtf = write_synthetic_gdtf(dir.path());
        let project = project(dir.path());

        let report = import_gdtf(&gdtf, None, &project, TYPES).unwrap();

        assert_eq!(report.type_name, "Synth Brick", "named from the archive");
        assert_eq!(report.fixture, "Synth Brick");
        assert_eq!(report.manufacturer, "mtrack synthetic");
        assert_eq!(report.modes, 2);
        assert_eq!(report.archive, "lighting/library/synth.gdtf");
        assert!(!report.already_imported);
        assert_eq!(report.fixture_file, None);
        assert!(
            report.refused_modes.is_empty(),
            "{:?}",
            report.refused_modes
        );
        assert!(project.join("lighting/library/synth.gdtf").is_file());
        assert!(
            !project.join(TYPES).exists(),
            "an import writes no record: the archive is the fixture"
        );
        let library =
            crate::lighting::library::unrecorded_types(&project, Some(&project.join(TYPES)));
        assert_eq!(library.types[0].name, "Synth Brick");
        // The rig the fixture page draws a GDTF fixture with is there.
        let rig = crate::lighting::distill::DistillCache::rig_path(
            &synthetic_bytes("Synth Brick"),
            "8: RGBS",
        );
        assert!(
            project.join("lighting/.cache/assets").join(rig).is_file(),
            "the first drivable mode's rig is warmed"
        );
    }

    #[test]
    fn importing_the_same_archive_again_changes_nothing_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let gdtf = write_synthetic_gdtf(dir.path());
        let project = project(dir.path());
        import_gdtf(&gdtf, None, &project, TYPES).unwrap();

        // Same bytes under another file name: still the same fixture.
        let copy = dir.path().join("downloaded-again.gdtf");
        std::fs::copy(&gdtf, &copy).unwrap();
        let report = import_gdtf(&copy, None, &project, TYPES).unwrap();

        assert!(report.already_imported);
        assert_eq!(report.type_name, "Synth Brick");
        assert_eq!(report.archive, "lighting/library/synth.gdtf");
        assert!(!project
            .join("lighting/library/downloaded-again.gdtf")
            .exists());

        // Recorded under another name: that name is what it is called.
        std::fs::create_dir_all(project.join(TYPES)).unwrap();
        std::fs::write(
            project.join(TYPES).join("brick.fixture"),
            "fixture_type \"Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n}\n",
        )
        .unwrap();
        let report = import_gdtf(&copy, None, &project, TYPES).unwrap();
        assert!(report.already_imported);
        assert_eq!(report.type_name, "Brick");
    }

    #[test]
    fn a_name_another_fixture_has_pins_the_newcomer_with_its_stem() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let first = write_synthetic_gdtf(dir.path());
        import_gdtf(&first, None, &project, TYPES).unwrap();

        // A different archive of the same fixture name, whose file sorts
        // first: it must not take the name venues already use.
        let other = dir.path().join("a-brick-v2.gdtf");
        std::fs::write(
            &other,
            crate::testutil::lighting::synthetic_gdtf_of(
                &crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                    .replace("mtrack synthetic", "mtrack synthetic v2"),
            ),
        )
        .unwrap();
        let report = import_gdtf(&other, None, &project, TYPES).unwrap();

        assert!(!report.already_imported);
        assert_eq!(report.type_name, "Synth Brick (a-brick-v2)");
        assert_eq!(report.renamed_from.as_deref(), Some("Synth Brick"));
        assert_eq!(
            report.fixture_file.as_deref(),
            Some("lighting/fixture_types/synth_brick_a-brick-v2.fixture")
        );
        let library =
            crate::lighting::library::unrecorded_types(&project, Some(&project.join(TYPES)));
        let names: Vec<&str> = library.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Synth Brick"], "the incumbent keeps its name");
        let records = existing_types(&project.join(TYPES));
        assert_eq!(
            records.get("Synth Brick (a-brick-v2)"),
            Some(&Some("lighting/library/a-brick-v2.gdtf".to_string()))
        );
    }

    #[test]
    fn a_different_archive_of_a_library_name_is_refused_in_the_user_s_terms() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let a = write_synthetic_gdtf(dir.path());
        import_gdtf(&a, None, &project, TYPES).unwrap();
        let original = std::fs::read(project.join("lighting/library/synth.gdtf")).unwrap();

        let other_dir = dir.path().join("elsewhere");
        std::fs::create_dir_all(&other_dir).unwrap();
        let b = other_dir.join("synth.gdtf");
        std::fs::write(&b, synthetic_bytes("Other Brick")).unwrap();
        let err = import_gdtf(&b, None, &project, TYPES).unwrap_err();

        assert!(err.downcast_ref::<LibraryClash>().is_some());
        let text = err.to_string();
        assert!(
            text.contains("a different GDTF called \"synth.gdtf\" is already in this project"),
            "{text}"
        );
        assert!(text.contains("that one is Synth Brick"), "{text}");
        assert!(text.contains("synth-2.gdtf"), "{text}");
        assert!(!text.contains(".fixture"), "{text}");
        assert_eq!(
            std::fs::read(project.join("lighting/library/synth.gdtf")).unwrap(),
            original,
            "a refused import must not touch the library archive"
        );
    }

    #[test]
    fn a_name_writes_a_record_of_the_copy_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let a = write_synthetic_gdtf(dir.path());
        import_gdtf(&a, None, &project, TYPES).unwrap();

        let report = import_gdtf(&a, Some("Twin"), &project, TYPES).unwrap();
        let library: Vec<_> = std::fs::read_dir(project.join("lighting/library"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(library, ["synth.gdtf"], "the copy already there is used");
        assert!(!report.already_imported);
        assert_eq!(report.type_name, "Twin");
        let record = std::fs::read_to_string(project.join(TYPES).join("twin.fixture")).unwrap();
        assert!(
            record.contains("fixture_type \"Twin\"\n  from gdtf(\"lighting/library/synth.gdtf\")"),
            "{record}"
        );
        // The same archive under that name again changes nothing.
        let again = import_gdtf(&a, Some("Twin"), &project, TYPES).unwrap();
        assert!(again.already_imported);
        assert_eq!(again.type_name, "Twin");
        // Another archive under that name is refused, and nothing moves.
        let other = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.replace("Synth Brick", "Other");
        let bytes = crate::lighting::gdtf::build_zip(&[("description.xml", other.as_bytes())]);
        let err = import_gdtf_bytes(&bytes, "other.gdtf", Some("Twin"), &project, TYPES)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("\"Twin\" is already declared in twin.fixture"),
            "{err}"
        );
        assert!(!project.join("lighting/library/other.gdtf").exists());
    }

    /// The files in the types directory, by name.
    fn type_files_in(project: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(project.join(TYPES))
            .map(|entries| {
                entries
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn a_pinned_name_whose_file_is_taken_is_refused_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let gdtf = write_synthetic_gdtf(dir.path());
        std::fs::create_dir_all(project.join(TYPES)).unwrap();
        // `par.fixture` holds another type: the name "Par" is free, its
        // file is not.
        std::fs::write(
            project.join(TYPES).join("par.fixture"),
            "fixture_type \"Old Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();

        let err = import_gdtf(&gdtf, Some("Par"), &project, TYPES)
            .unwrap_err()
            .to_string();

        assert_eq!(
            err,
            "par.fixture is taken by \"Old Par\"; choose another name"
        );
        assert_eq!(type_files_in(&project), ["par.fixture"], "no par_2.fixture");
        assert!(
            !project.join("lighting/library").exists(),
            "nothing is written when the record is refused"
        );
    }

    #[test]
    fn a_pinned_name_whose_file_is_there_in_another_case_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let gdtf = write_synthetic_gdtf(dir.path());
        std::fs::create_dir_all(project.join(TYPES)).unwrap();
        std::fs::write(
            project.join(TYPES).join("Wash.fixture"),
            "fixture_type \"Big Wash\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();

        let err = import_gdtf(&gdtf, Some("wash"), &project, TYPES)
            .unwrap_err()
            .to_string();

        assert_eq!(
            err,
            "Wash.fixture is taken by \"Big Wash\"; choose another name"
        );
        assert_eq!(type_files_in(&project), ["Wash.fixture"]);
    }

    #[test]
    fn a_pinned_name_another_archive_has_in_the_library_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let first = write_synthetic_gdtf(dir.path());
        import_gdtf(&first, None, &project, TYPES).unwrap();

        let other = synthetic_bytes("Other Brick");
        let err = import_gdtf_bytes(&other, "other.gdtf", Some("Synth Brick"), &project, TYPES)
            .unwrap_err();

        assert!(err
            .downcast_ref::<crate::lighting::files::Refusal>()
            .is_some());
        assert_eq!(
            err.to_string(),
            "\"Synth Brick\" is the GDTF library type in lighting/library/synth.gdtf; choose \
             another name"
        );
        assert!(!project.join("lighting/library/other.gdtf").exists());
        assert_eq!(type_files_in(&project), Vec::<String>::new());
    }

    #[test]
    fn a_newcomer_s_stem_name_whose_file_is_taken_is_refused_not_suffixed() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let first = write_synthetic_gdtf(dir.path());
        import_gdtf(&first, None, &project, TYPES).unwrap();
        std::fs::create_dir_all(project.join(TYPES)).unwrap();
        std::fs::write(
            project.join(TYPES).join("synth_brick_v2.fixture"),
            "# notes kept by hand\n",
        )
        .unwrap();

        // The library names the newcomer "Synth Brick (v2)", as before; its
        // record's file is taken, so the import is refused.
        let other = crate::testutil::lighting::synthetic_gdtf_of(
            &crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("mtrack synthetic", "mtrack synthetic v2"),
        );
        let err = import_gdtf_bytes(&other, "v2.gdtf", None, &project, TYPES)
            .unwrap_err()
            .to_string();

        assert_eq!(
            err,
            "synth_brick_v2.fixture already exists; choose another name"
        );
        assert_eq!(type_files_in(&project), ["synth_brick_v2.fixture"]);
        assert!(!project.join("lighting/library/v2.gdtf").exists());
    }

    #[test]
    fn a_library_copy_that_fails_part_way_leaves_nothing_and_the_retry_imports() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let bytes = synthetic_bytes("Synth Brick");

        crate::util::inject_faults(&[crate::util::Fault::AfterTruncating]);
        let err = import_gdtf_bytes(&bytes, "synth.gdtf", None, &project, TYPES)
            .unwrap_err()
            .to_string();
        crate::util::inject_faults(&[]);

        assert!(err.contains("synth.gdtf"), "{err}");
        assert!(err.ends_with("; nothing was saved"), "{err}");
        let library = project.join("lighting/library");
        let left: Vec<_> = std::fs::read_dir(&library)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "no partial copy or staged copy: {left:?}");

        let report = import_gdtf_bytes(&bytes, "synth.gdtf", None, &project, TYPES).unwrap();
        assert!(!report.already_imported);
        assert_eq!(report.type_name, "Synth Brick");
        assert_eq!(std::fs::read(library.join("synth.gdtf")).unwrap(), bytes);
    }

    #[test]
    fn an_archive_with_no_modes_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let xml = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION;
        let start = xml.find("<DMXModes>").unwrap();
        let end = xml.find("</DMXModes>").unwrap() + "</DMXModes>".len();
        let empty = format!("{}<DMXModes/>{}", &xml[..start], &xml[end..]);
        let bytes = crate::lighting::gdtf::build_zip(&[("description.xml", empty.as_bytes())]);
        let err = import_gdtf_bytes(&bytes, "empty.gdtf", None, &project, TYPES)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no DMX modes"), "{err}");
        assert!(!project.join("lighting").exists());
    }

    #[test]
    fn suggested_names_keep_only_name_characters() {
        assert_eq!(
            suggested_type_name("PB15 \"Pixel\"  Brick"),
            "PB15 Pixel Brick"
        );
        assert_eq!(suggested_type_name("{}"), "fixture");
    }
}
