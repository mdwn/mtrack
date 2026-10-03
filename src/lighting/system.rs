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

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use super::distill::DistillCache;
use super::gdtf;

use super::patch;
use super::types::{Fixture, FixtureType, Venue};
use crate::config::lighting::{GroupConstraint, LogicalGroup};
use crate::config::Lighting;

/// The lighting system configuration.
pub struct LightingSystem {
    /// The native fixture types, as declared. A GDTF-sourced type is not
    /// here: it has no channels until a fixture names a mode.
    fixture_types: HashMap<String, FixtureType>,

    /// Every GDTF-sourced type as declared (unexpanded): what a venue
    /// fixture's own mode expands from (venue-exchange design §21).
    referential: HashMap<String, FixtureType>,

    /// The file each fixture type was declared in, for errors that must say
    /// where to look.
    fixture_type_files: HashMap<String, PathBuf>,

    /// Expansions of the modes venue fixtures name, by (type, mode as the
    /// fixture line writes it). Filled at load and on every venue reload
    /// (a user's venue edit, which can arrive during playback), for every
    /// loaded venue — venues can be switched — and never at a cue.
    mode_expansions: HashMap<(String, String), FixtureType>,

    /// Why a (type, mode) a venue fixture names did not expand. Retried on
    /// every expansion pass: the cause (an archive briefly unreadable, a
    /// fixed typo in the type file) may have gone.
    mode_errors: HashMap<(String, String), String>,

    /// Notes on modes that matched only after normalization, by (type, mode
    /// as written): the fixture loads, and the file should be corrected.
    mode_warnings: HashMap<(String, String), String>,

    /// The file each venue was read from.
    venue_files: HashMap<String, PathBuf>,

    /// Why a fixture type named in a file did not load (a GDTF archive that
    /// would not expand, rich syntax in a `.light` file), by type name.
    /// Recorded where the loader already knows, so a caller can say why
    /// instead of only that the type is missing.
    fixture_type_errors: HashMap<String, String>,

    /// Fixture type files that did not parse at all, as `file: error`. The
    /// types inside are unnamed, so these explain any type that is missing.
    fixture_type_file_errors: Vec<String>,

    /// Venues.
    venues: HashMap<String, Venue>,

    /// Current venue.
    current_venue: Option<String>,

    /// Inline fixtures.
    inline_fixtures: HashMap<String, String>,

    /// Logical groups with role-based constraints.
    logical_groups: HashMap<String, LogicalGroup>,

    /// Cached group resolutions per venue.
    group_cache: HashMap<String, HashMap<String, Vec<String>>>,

    /// Each MVR-seeded venue's scenery in the asset store, by venue name
    /// (design §16.3), filled when venues load.
    scenery: HashMap<String, String>,

    /// Why a venue's scenery is missing, when its distillation failed —
    /// the 3D view says so rather than showing a bare deck in silence.
    scenery_errors: HashMap<String, String>,

    /// The MVR file each venue's scenery was distilled from, as (path,
    /// modified time, length): a reload whose archive is unchanged reuses
    /// the entry without reading or hashing the file again.
    scenery_sources: HashMap<String, (PathBuf, Option<std::time::SystemTime>, u64)>,

    /// The project directory the system loaded from, for the asset store.
    project_root: Option<PathBuf>,

    /// Where the venues were loaded from, so an edited venue can be re-read
    /// without rebuilding the whole system.
    venues_source: Option<VenuesSource>,

    /// Where the fixture types were loaded from, so an edit to a type (its
    /// name, its movement limits) can be re-read the same way.
    fixture_types_path: Option<PathBuf>,

    /// The current venue's problems as last logged, so a reload (every
    /// stage-view drag is one) logs them again only when they change.
    logged_venue_report: Option<Vec<String>>,

    /// What the GDTF library holds that a person should know (two archives
    /// stating one name, a file that is not a GDTF), venue-independent.
    library_findings: Vec<super::library::LibraryFinding>,

    /// A GDTF type's mode names, read at load for a type some venue fixture
    /// uses without a mode — the load error lists them.
    type_modes: HashMap<String, Vec<String>>,
}

/// The venues directory a system loaded, as the config names it (or the
/// default) and as resolved.
#[derive(Clone, Debug)]
struct VenuesSource {
    /// The directory, relative to the project: configured, or the default.
    configured: String,
    /// The resolved directory.
    path: PathBuf,
}

impl Default for LightingSystem {
    fn default() -> Self {
        Self::new()
    }
}

/// A configured directory that is a file is a configuration mistake, and
/// fails the load; one that does not exist yet simply holds nothing.
fn not_a_file(dir: &Path) -> Result<(), Box<dyn Error>> {
    if dir.exists() && !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()).into());
    }
    Ok(())
}

/// Why [`LightingSystem::type_in_mode`] has no type to drive.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeInModeError {
    /// No fixture type of that name.
    Unknown(String),
    /// The mode is missing, not wanted, or does not distil.
    Mode(String),
}

impl std::fmt::Display for TypeInModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeInModeError::Unknown(name) => write!(f, "no fixture type named \"{name}\""),
            TypeInModeError::Mode(why) => f.write_str(why),
        }
    }
}

/// The effects engine's view of one fixture of `fixture_type` (a native
/// type, or a GDTF type's expansion in a mode) patched at `universe` /
/// `address`: its channels, their definitions (fine bytes, ranges,
/// functions), strobe range, movement limits, rig and cells. A venue
/// fixture is registered through this, and so is the fixture test's
/// temporary one, so the two resolve values to DMX alike.
pub fn fixture_info_for(
    name: &str,
    universe: u16,
    address: u16,
    type_name: &str,
    fixture_type: &FixtureType,
) -> crate::lighting::effects::FixtureInfo {
    let mut fixture_info = crate::lighting::effects::FixtureInfo::new(
        name.to_string(),
        universe,
        address,
        type_name.to_string(),
        fixture_type.channels().clone(),
        fixture_type.max_strobe_frequency(),
    );
    fixture_info.min_strobe_frequency = fixture_type.min_strobe_frequency();
    fixture_info.strobe_dmx_offset = fixture_type.strobe_dmx_offset();
    fixture_info.channel_defs = fixture_type.channel_defs().clone();
    fixture_info.movement = *fixture_type.movement();
    fixture_info.strobe_curve = fixture_type.effective_strobe_curve();
    fixture_info.rig = fixture_type.rig().map(str::to_string);
    fixture_info.aim = fixture_type.aim();
    fixture_info.cells = fixture_type.cells().to_vec();
    fixture_info
}

impl LightingSystem {
    /// Creates a new lighting system.
    pub fn new() -> LightingSystem {
        LightingSystem {
            fixture_types: HashMap::new(),
            referential: HashMap::new(),
            fixture_type_files: HashMap::new(),
            mode_expansions: HashMap::new(),
            mode_errors: HashMap::new(),
            mode_warnings: HashMap::new(),
            venue_files: HashMap::new(),
            fixture_type_errors: HashMap::new(),
            fixture_type_file_errors: Vec::new(),
            venues: HashMap::new(),
            current_venue: None,
            inline_fixtures: HashMap::new(),
            logical_groups: HashMap::new(),
            group_cache: HashMap::new(),
            scenery: HashMap::new(),
            scenery_errors: HashMap::new(),
            scenery_sources: HashMap::new(),
            project_root: None,
            venues_source: None,
            fixture_types_path: None,
            logged_venue_report: None,
            library_findings: Vec::new(),
            type_modes: HashMap::new(),
        }
    }

    /// The venues directory, relative to the project (configured, or the
    /// default), once the system has loaded.
    pub fn venues_dir(&self) -> Option<&str> {
        self.venues_source.as_ref().map(|s| s.configured.as_str())
    }

    /// Re-reads every venue from the directory the system was loaded from,
    /// so an edit to the current venue (positions, focus points) reaches the
    /// running engine without a hardware reload. Fixture types are not
    /// re-read: a venue edit does not change them, and re-expanding
    /// referential types is the expensive part of a load. A mode the edited
    /// venues name that no venue named before is expanded here — a venue
    /// edit is a user's act, never a cue, though it can come during
    /// playback — and the expansions already made are kept.
    pub fn reload_venues(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(source) = self.venues_source.clone() else {
            return Err("no venues directory was loaded".into());
        };
        let mut fresh = LightingSystem::new();
        fresh.load_venues_directory(&source.path)?;
        self.venues = fresh.venues;
        self.venue_files = fresh.venue_files;
        self.group_cache.clear();
        if let Some(root) = self.project_root.clone() {
            self.expand_venue_modes(&root);
            self.refresh_scenery(&root);
        }
        Ok(())
    }

    /// Re-reads the fixture types and then the venues from the directories
    /// the system was loaded from: an edit to a type's settings (its name or
    /// movement limits — and a rename rewrites the venue
    /// lines that use it) reaches the running engine without a hardware
    /// reload. A user's act, never a cue: expansions come through the
    /// distill cache, and the modes venues name are expanded afresh.
    pub fn reload_fixture_types(&mut self) -> Result<(), Box<dyn Error>> {
        let (Some(path), Some(root)) = (self.fixture_types_path.clone(), self.project_root.clone())
        else {
            return Err("no fixture types directory was loaded".into());
        };
        let mut fresh = LightingSystem::new();
        fresh.load_fixture_types_directory(&path)?;
        fresh.load_library(&root, Some(&path));
        self.fixture_types = fresh.fixture_types;
        self.referential = fresh.referential;
        self.fixture_type_files = fresh.fixture_type_files;
        self.fixture_type_errors = fresh.fixture_type_errors;
        self.fixture_type_file_errors = fresh.fixture_type_file_errors;
        self.mode_warnings = fresh.mode_warnings;
        self.library_findings = fresh.library_findings;
        // A type's archive or name may have changed: no expansion of the
        // old one stands.
        self.mode_expansions.clear();
        self.mode_errors.clear();
        self.type_modes.clear();
        self.reload_venues()
    }

    /// The store-relative path of a venue's scenery file, when the venue
    /// was seeded from an MVR whose scenery has been distilled.
    pub fn scenery(&self, venue: &str) -> Option<&str> {
        self.scenery.get(venue).map(String::as_str)
    }

    /// Why a venue's scenery is missing, when it should have had some.
    pub fn scenery_error(&self, venue: &str) -> Option<&str> {
        self.scenery_errors.get(venue).map(String::as_str)
    }

    /// Distills every MVR-seeded venue's scenery into the asset store. A
    /// venue whose scenery cannot be distilled loads without any: the 3D
    /// view is the only consumer, and a bare deck beats a missing venue.
    fn refresh_scenery(&mut self, base_path: &Path) {
        let previous = std::mem::take(&mut self.scenery);
        let previous_sources = std::mem::take(&mut self.scenery_sources);
        self.scenery_errors.clear();
        let base_path = if base_path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            base_path
        };
        let cache = DistillCache::new(base_path.join("lighting").join(".cache"));
        for (name, venue) in &self.venues {
            let Some(source) = venue.source() else {
                continue;
            };
            let archive_path = base_path.join(&source.mvr);
            // Unchanged archive (same path, modified time and length) and a
            // store entry still there: nothing to read or hash again. A
            // venue edit that saves the file must not cost a pass over a
            // hundred-megabyte MVR.
            let stamp = std::fs::metadata(&archive_path)
                .ok()
                .map(|m| (archive_path.clone(), m.modified().ok(), m.len()));
            if let (Some(stamp), Some(prev), Some(rel)) =
                (&stamp, previous_sources.get(name), previous.get(name))
            {
                if prev == stamp && cache.assets_dir().join(rel).is_file() {
                    self.scenery.insert(name.clone(), rel.clone());
                    self.scenery_sources.insert(name.clone(), stamp.clone());
                    continue;
                }
            }
            let result = (|| -> Result<(String, Vec<String>), Box<dyn Error>> {
                let canonical = archive_path
                    .canonicalize()
                    .map_err(|e| format!("cannot read MVR {}: {e}", archive_path.display()))?;
                if !canonical.starts_with(base_path.canonicalize()?) {
                    return Err(
                        format!("MVR path {} escapes the project directory", source.mvr).into(),
                    );
                }
                let bytes = std::fs::read(&canonical)?;
                let parsed: std::cell::OnceCell<crate::lighting::mvr::Scene> =
                    std::cell::OnceCell::new();
                cache.ensure_scenery(&bytes, &source.origin, || {
                    if parsed.get().is_none() {
                        let _ = parsed.set(crate::lighting::mvr::parse_archive(&bytes)?);
                    }
                    Ok(parsed.get().expect("just set"))
                })
            })();
            match result {
                Ok((rel, warnings)) => {
                    for warning in warnings {
                        warn!(venue = name, "Scenery: {warning}");
                    }
                    self.scenery.insert(name.clone(), rel);
                    if let Some(stamp) = stamp {
                        self.scenery_sources.insert(name.clone(), stamp);
                    }
                }
                Err(e) => {
                    warn!(venue = name, error = %e, "No scenery for the 3D view");
                    self.scenery_errors.insert(name.clone(), e.to_string());
                }
            }
        }
    }

    /// Returns an iterator over the (name, venue) pairs known to the system.
    pub fn venues_iter(&self) -> impl Iterator<Item = (&String, &Venue)> {
        self.venues.iter()
    }

    /// The native (hand-written) fixture types: these have channels of
    /// their own. A GDTF type has channels only in a mode, so it is in
    /// [`Self::gdtf_types_iter`] instead.
    pub fn fixture_types_iter(&self) -> impl Iterator<Item = (&String, &FixtureType)> {
        self.fixture_types.iter()
    }

    /// The GDTF types, as declared (their archive and, from a record, their
    /// name and movement limits). Each venue fixture of one states its mode;
    /// the channels, footprint and capabilities are that mode's.
    pub fn gdtf_types_iter(&self) -> impl Iterator<Item = (&String, &FixtureType)> {
        self.referential.iter()
    }

    /// Why a fixture of type `name` cannot be patched, or `None` when the type
    /// loaded. The loader's own reason when it recorded one, otherwise that no
    /// type by that name exists (with any file that failed to parse, since the
    /// missing type may have been inside it).
    pub fn fixture_type_problem(&self, name: &str) -> Option<String> {
        if self.fixture_types.contains_key(name) {
            return None;
        }
        if let Some(reason) = self.fixture_type_errors.get(name) {
            return Some(reason.clone());
        }
        // A GDTF type loaded as a declaration; its fixtures name their
        // modes, and [`Self::fixture_problem`] is where one that cannot be
        // driven says so.
        if self.referential.contains_key(name) {
            return None;
        }
        let mut reason = format!("no fixture type named '{name}' is defined");
        if !self.fixture_type_file_errors.is_empty() {
            reason.push_str("; fixture type files that failed to parse: ");
            reason.push_str(&self.fixture_type_file_errors.join("; "));
        }
        Some(reason)
    }

    /// Returns an iterator over the (name, logical group) pairs known to the
    /// system. These are the tag/constraint-based groups defined under
    /// `dmx.lighting.groups` in the player config.
    pub fn logical_groups_iter(&self) -> impl Iterator<Item = (&String, &LogicalGroup)> {
        self.logical_groups.iter()
    }

    /// Loads the lighting configuration.
    pub fn load(&mut self, config: &Lighting, base_path: &Path) -> Result<(), Box<dyn Error>> {
        info!(
            "Loading lighting system from base path: {}",
            base_path.display()
        );

        // Set current venue
        if let Some(venue) = config.current_venue() {
            self.current_venue = Some(venue.to_string());
        }

        // Load inline fixtures and groups
        self.inline_fixtures = config.fixtures().clone();
        self.logical_groups = config.groups().clone();

        // The project's fixture types and venues, from the configured
        // directories or the defaults (`config::lighting`), each on its own:
        // the web UI saves to the same places, so what a user makes there
        // is what loads. A directory that is not there (any project without
        // lighting) is simply empty.
        let fixture_types_dir = config.fixture_types_dir();
        let path = base_path.join(fixture_types_dir);
        self.load_fixture_types_directory(&path)?;
        self.fixture_types_path = Some(path);

        let venues_dir = config.venues_dir();
        let path = base_path.join(venues_dir);
        self.load_venues_directory(&path)?;
        self.venues_source = Some(VenuesSource {
            configured: venues_dir.to_string(),
            path,
        });

        // Every GDTF in the library no type file points at is a fixture too
        // (design §22.2).
        let types_path = self.fixture_types_path.clone();
        self.load_library(base_path, types_path.as_deref());

        // Types load before venues, so the modes the venues' fixtures name
        // are expanded once both are in — every venue, not only the
        // current one, since a venue switch must not parse an archive.
        self.expand_venue_modes(base_path);

        // Scenery for the 3D view, from each MVR-seeded venue's archive.
        self.project_root = Some(base_path.to_path_buf());
        self.refresh_scenery(base_path);
        Ok(())
    }

    /// Registers the library's unrecorded archives as fixture types (nothing
    /// to expand until a venue fixture names a mode), from the one function
    /// every enumerator asks. A name a type
    /// file already declared is never replaced: the library takes the
    /// archive's file stem instead, so this cannot shadow anything.
    fn load_library(&mut self, base_path: &Path, types_dir: Option<&Path>) {
        let library = super::library::unrecorded_types(base_path, types_dir);
        for library_type in &library.types {
            if self.referential.contains_key(&library_type.name)
                || self.fixture_types.contains_key(&library_type.name)
            {
                continue;
            }
            info!(
                fixture_type = library_type.name.as_str(),
                archive = library_type.file_name.as_str(),
                "Loaded a GDTF from the library as a fixture type; its fixtures name their modes"
            );
            self.referential.insert(
                library_type.name.clone(),
                super::library::declared(library_type),
            );
            self.fixture_type_files.insert(
                library_type.name.clone(),
                base_path.join(&library_type.archive),
            );
        }
        self.library_findings = library.findings;
    }

    /// What the GDTF library holds that a person should know: two archives
    /// stating one fixture name, or a file that is not a readable GDTF.
    pub fn library_findings(&self) -> &[super::library::LibraryFinding] {
        &self.library_findings
    }

    /// Loads fixture types from a directory and its subdirectories, through
    /// the project's one reader of type files ([`project_files::type_files`]),
    /// so the web API lists exactly what loads here. When two files declare
    /// one name, the first in path order keeps it and the clash is logged
    /// and reported as a file error.
    fn load_fixture_types_directory(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        not_a_file(dir)?;
        let read = super::project_files::type_files(dir);
        for declared in read.items {
            self.load_fixture_type(declared.name, declared.item, &declared.file);
        }
        for problem in read.problems {
            warn!(
                file = %problem.file.display(),
                error = %problem.error,
                "Failed to load fixture type file"
            );
            self.fixture_type_file_errors.push(format!(
                "{}: {}",
                super::project_files::display_in(dir, &problem.file),
                problem.error
            ));
        }
        Ok(())
    }

    /// Loads venues from a directory and its subdirectories, through the
    /// project's one reader of venue files ([`project_files::venues`]). One
    /// bad file never stops the rest, and never vanishes either: the
    /// venue-group migration advice is raised as a parse error, and
    /// swallowing it turns a fixable file into "Venue 'x' not found" later.
    fn load_venues_directory(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        not_a_file(dir)?;
        let read = super::project_files::venues(dir);
        for declared in read.items {
            info!(venue = declared.name.as_str(), "Loading venue");
            self.venue_files
                .insert(declared.name.clone(), declared.file.clone());
            self.venues.insert(declared.name, declared.item);
        }
        for problem in read.problems {
            warn!(
                file = %problem.file.display(),
                error = %problem.error,
                "Failed to load venue file"
            );
        }
        Ok(())
    }

    /// Registers one declared fixture type from `path`.
    fn load_fixture_type(&mut self, name: String, fixture_type: FixtureType, path: &Path) {
        self.fixture_type_files
            .insert(name.clone(), path.to_path_buf());
        if fixture_type.source().is_some() {
            // A GDTF type is the whole archive: nothing to expand until a
            // venue fixture names a mode (design §22).
            info!(fixture_type = name, "Loaded a GDTF fixture type");
            self.referential.insert(name, fixture_type);
            return;
        }
        if fixture_type.uses_rich_channels() && path.extension().is_some_and(|ext| ext == "light") {
            // The extension is the version marker: rich channel syntax is
            // the v2 DSL and lives in .fixture files.
            warn!(
                fixture_type = name,
                file = %path.display(),
                "Rich channel syntax (fine, range, functions) belongs in a \
                 .fixture file; rename the file — skipping this type"
            );
            self.fixture_type_errors.insert(
                name,
                "rich channel syntax belongs in a .fixture file, not a .light file".to_string(),
            );
            return;
        }
        info!(fixture_type = name, "Loading fixture type");
        self.fixture_types.insert(name, fixture_type);
    }

    /// Expands a GDTF fixture type in `mode` — a mode a venue fixture names
    /// — through the per-project distill cache (`lighting/.cache/`,
    /// hash-keyed, rebuildable). Parsing the archive happens only on a cold
    /// cache — at load time or on a user's venue edit, never at a cue — and
    /// the fill is logged loudly. The type's
    /// body (movement limits) applies to every mode. Both cache keys already
    /// carry the mode, so each mode is its own entry.
    ///
    /// The second value is a note when `mode` matched the archive's mode
    /// only after normalization (case, whitespace, punctuation), as
    /// [`gdtf::match_mode`] allows: the fixture loads, and the file should
    /// be corrected. Read from the rig, which records the mode it matched,
    /// so it is said on every load, not only a cold one.
    pub(crate) fn expand_mode(
        name: &str,
        fixture_type: &FixtureType,
        mode: &str,
        base_path: &Path,
    ) -> Result<(FixtureType, Option<String>), Box<dyn Error>> {
        let declared = fixture_type
            .source()
            .ok_or_else(|| format!("fixture type \"{name}\" is not GDTF-sourced"))?;
        let source = super::types::GdtfSource {
            path: declared.path.clone(),
        };
        let mut note = None;

        // A bare-filename config path can yield an empty base_path; plain
        // joins tolerate it but canonicalize() does not. Defense in depth —
        // the call sites normalize too.
        let base_path = if base_path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            base_path
        };

        // The archive must live inside the project: a .fixture file is
        // config, but keeping references project-relative means a rig
        // directory stays self-contained (and a stray absolute path can't
        // wander the filesystem).
        let archive_path = base_path.join(&source.path);
        let canonical = archive_path
            .canonicalize()
            .map_err(|e| format!("cannot read GDTF archive {}: {e}", archive_path.display()))?;
        let canonical_base = base_path.canonicalize()?;
        if !canonical.starts_with(&canonical_base) {
            return Err(format!(
                "GDTF archive path {} escapes the project directory",
                source.path
            )
            .into());
        }

        let bytes = std::fs::read(&canonical)
            .map_err(|e| format!("cannot read GDTF archive {}: {e}", canonical.display()))?;
        // Everything that can change the expansion participates in the key;
        // movement is the only override today, so it is the fingerprint.
        let fingerprint = format!("{:?}", fixture_type.movement());
        let key = DistillCache::key(name, &bytes, mode, &fingerprint);
        let cache = DistillCache::new(base_path.join("lighting").join(".cache"));
        // The archive is parsed at most once per call, whichever of the
        // expansion and the rig is missing.
        let parsed: std::cell::OnceCell<gdtf::Description> = std::cell::OnceCell::new();
        let describe = || -> Result<&gdtf::Description, Box<dyn Error>> {
            if parsed.get().is_none() {
                let _ = parsed.set(gdtf::parse_archive(&bytes)?);
            }
            Ok(parsed.get().expect("just set"))
        };
        let mut expanded = cache.get_or_fill(&key, || {
            info!(
                fixture_type = name,
                gdtf = %source.path,
                mode = %mode,
                "Expansion cache is cold — distilling GDTF"
            );
            let description = describe()?;
            let distilled = gdtf::distill(description, mode, name)?;
            // Distillation warnings surface on the cold fill; the import
            // command is the place they're reported interactively.
            for warning in &distilled.warnings {
                warn!(fixture_type = name, "GDTF distillation: {warning}");
            }
            let mut expanded = distilled.fixture_type;
            expanded.set_source(source.clone());
            expanded.set_movement(*fixture_type.movement());
            Ok(expanded)
        })?;
        // The strobe curve changes no channel, so it is not part of the
        // expansion (or its cache key): it rides on the record's type.
        expanded.set_strobe_curve(fixture_type.strobe_curve());
        // The rig (design §16.2) is the 3D view's, not the show's: a rig
        // that cannot be written is logged, and the type loads without one.
        match cache.ensure_rig(&bytes, mode, describe) {
            Ok((rig, warnings)) => {
                for warning in warnings {
                    warn!(fixture_type = name, "Rig model: {warning}");
                }
                // The show aims through the rig's joints (design §18.6);
                // a geometry the closed form cannot follow is said so and
                // the plain convention stands in.
                let model = cache.load_rig(&rig);
                // The rig names the mode it matched; a different spelling
                // here means the match needed normalizing. The archive's
                // spelling stays out of the message (it is archive text).
                if let Ok(model) = &model {
                    if model.mode != mode {
                        let text = format!(
                            "mode \"{mode}\" matches a mode of {} only after normalizing case, \
                             whitespace or punctuation; write it exactly as the GDTF names it",
                            source.path
                        );
                        warn!(fixture_type = name, "{text}");
                        note = Some(text);
                    }
                }
                match model.map(|model| gdtf::aim_calibration(&model)) {
                    Ok(Ok(calibration)) => {
                        if !calibration.frame_is_identity() {
                            info!(
                                fixture_type = name,
                                pan_offset = calibration.pan_offset,
                                tilt_offset = calibration.tilt_offset,
                                "Aiming through the rig's geometry"
                            );
                        }
                        expanded.set_aim(Some(calibration));
                    }
                    Ok(Err(reason)) => warn!(
                        fixture_type = name,
                        "Cannot aim through the rig's geometry ({reason}); using the plain convention"
                    ),
                    Err(e) => warn!(fixture_type = name, error = %e, "Rig model unreadable"),
                }
                expanded.set_rig(Some(rig));
            }
            Err(e) => warn!(fixture_type = name, error = %e, "No rig model for the 3D view"),
        }
        Ok((expanded, note))
    }

    /// Expands every mode a fixture of any loaded venue names (design §21),
    /// reusing what is already expanded and retrying what failed, then
    /// reports the current venue's problems (see
    /// [`Self::report_current_venue`]). Runs at load and on every venue
    /// reload — a user's edit, possibly during playback — never at a cue.
    fn expand_venue_modes(&mut self, base_path: &Path) {
        let mut wanted: Vec<(String, String)> = Vec::new();
        let mut modeless: Vec<String> = Vec::new();
        for venue in self.venues.values() {
            for fixture in venue.fixtures().values() {
                if !self.referential.contains_key(fixture.fixture_type()) {
                    continue;
                }
                let Some(mode) = fixture.mode() else {
                    // A load error; its message lists the modes to choose
                    // from, read now, at load.
                    if !modeless.iter().any(|t| t == fixture.fixture_type()) {
                        modeless.push(fixture.fixture_type().to_string());
                    }
                    continue;
                };
                let key = (fixture.fixture_type().to_string(), mode.to_string());
                if !wanted.contains(&key) {
                    wanted.push(key);
                }
            }
        }
        wanted.sort();
        for type_name in modeless {
            if self.type_modes.contains_key(&type_name) {
                continue;
            }
            let modes = self.referential[&type_name]
                .source()
                .map(|source| base_path.join(&source.path))
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|bytes| gdtf::parse_archive(&bytes).ok())
                .map(|description| description.modes.into_iter().map(|m| m.name).collect())
                .unwrap_or_default();
            self.type_modes.insert(type_name, modes);
        }
        // Modes no venue names any more go; the ones still named are kept
        // without touching the archive again.
        self.mode_expansions.retain(|key, _| wanted.contains(key));
        self.mode_errors.retain(|key, _| wanted.contains(key));
        for key in wanted {
            // A success is kept; a failure is tried again, since its cause
            // may have gone (an archive that was briefly unreadable).
            if self.mode_expansions.contains_key(&key) {
                continue;
            }
            let (type_name, mode) = &key;
            let declared = &self.referential[type_name];
            match Self::expand_mode(type_name, declared, mode, base_path) {
                Ok((expanded, note)) => {
                    info!(
                        fixture_type = type_name,
                        mode = mode,
                        channels = expanded.channels().len(),
                        "Loaded a venue fixture's GDTF mode"
                    );
                    if let Some(note) = note {
                        self.mode_warnings.insert(key.clone(), note);
                    }
                    self.mode_errors.remove(&key);
                    self.mode_expansions.insert(key, expanded);
                }
                Err(e) => {
                    // Said per fixture, aggregated, for the current venue
                    // (`report_current_venue`); other venues' failures are
                    // there for whoever asks (`fixture_problem`, readiness).
                    debug!(
                        fixture_type = type_name,
                        mode = mode,
                        file = %self
                            .fixture_type_files
                            .get(type_name)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        error = %e,
                        "A mode venue fixtures name did not expand; those fixtures will not light"
                    );
                    self.mode_errors.insert(key, e.to_string());
                }
            }
        }

        self.report_current_venue();
    }

    /// Logs the current venue's problems — fixtures that cannot be driven
    /// (which fail the venue), patch overlaps and overruns — one aggregated
    /// line per kind, and only when they differ from what was last logged:
    /// a reload runs on every stage-view drag, and an unchanged problem is
    /// not news. Other venues are not logged; their problems are a
    /// `fixture_problem` or readiness call away.
    fn report_current_venue(&mut self) {
        let lines = self.current_venue_report();
        if self.logged_venue_report.as_ref() == Some(&lines) {
            return;
        }
        let venue = self.current_venue.clone().unwrap_or_default();
        let file = self.venue_file_display(&venue);
        for line in &lines {
            warn!(venue = venue.as_str(), file = %file, "{line}");
        }
        if lines.is_empty()
            && self
                .logged_venue_report
                .as_ref()
                .is_some_and(|l| !l.is_empty())
        {
            info!(
                venue = venue.as_str(),
                "The venue's earlier problems are resolved"
            );
        }
        self.logged_venue_report = Some(lines);
    }

    /// The current venue's problems as log lines, one per kind.
    fn current_venue_report(&self) -> Vec<String> {
        let Some(venue_name) = self.current_venue.as_deref() else {
            return Vec::new();
        };
        let Some(venue) = self.venues.get(venue_name) else {
            return Vec::new();
        };
        fn names(names: &[&str]) -> String {
            const SHOWN: usize = 3;
            let quoted: Vec<String> = names
                .iter()
                .take(SHOWN)
                .map(|n| format!("\"{n}\""))
                .collect();
            if names.len() > SHOWN {
                format!("{} and {} more", quoted.join(", "), names.len() - SHOWN)
            } else {
                quoted.join(", ")
            }
        }
        let mut lines = Vec::new();
        let problems = self.venue_problems(venue_name);
        if let Some((_, first)) = problems.first() {
            let who: Vec<&str> = problems.iter().map(|(f, _)| f.as_str()).collect();
            lines.push(format!(
                "{} fixture(s) cannot be driven, so the venue will not light: {}; first: {first}",
                problems.len(),
                names(&who)
            ));
        }
        let spans = self.patch_spans(venue);
        let overlaps = patch::venue_overlaps(&spans);
        if let Some(first) = overlaps.first() {
            lines.push(format!(
                "{} patch overlap(s); first: {first}",
                overlaps.len()
            ));
        }
        let overruns = patch::venue_overruns(&spans);
        if let Some(first) = overruns.first() {
            let who: Vec<&str> = overruns.iter().map(|o| o.fixture.as_str()).collect();
            lines.push(format!(
                "{} fixture(s) run past address 512: {}; first: {first}",
                overruns.len(),
                names(&who)
            ));
        }
        lines
    }

    /// Every fixture of `venue` that cannot be driven, in patch order, with
    /// its [`Self::fixture_problem`] text. Empty when the venue registers
    /// (or is not loaded).
    pub fn venue_problems(&self, venue: &str) -> Vec<(String, String)> {
        let Some(loaded) = self.venues.get(venue) else {
            return Vec::new();
        };
        loaded
            .fixtures_by_patch()
            .into_iter()
            .filter_map(|fixture| {
                self.fixture_problem(venue, fixture)
                    .map(|problem| (fixture.name().to_string(), problem))
            })
            .collect()
    }

    /// The first fixture of `venue` (in patch order) that cannot be driven
    /// and why — what fails the venue's registration — or `None` when the
    /// venue registers.
    pub fn venue_problem(&self, venue: &str) -> Option<(String, String)> {
        self.venue_problems(venue).into_iter().next()
    }

    /// A venue's file, for messages; empty when the venue was not read
    /// from one.
    fn venue_file_display(&self, venue: &str) -> String {
        self.venue_files
            .get(venue)
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }

    /// A fixture type's file name, for messages.
    fn type_file_display(&self, fixture_type: &str) -> String {
        self.fixture_type_files
            .get(fixture_type)
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "an unknown file".to_string())
    }

    /// The expansion a venue fixture is driven by: a native type's own, or a
    /// GDTF type's in the mode the line names. The error says why not,
    /// naming the fixture and the type (and the type's file where that is
    /// where the fix goes).
    pub fn resolve_fixture_type(&self, fixture: &Fixture) -> Result<&FixtureType, String> {
        let type_name = fixture.fixture_type();
        let declared = self.referential.get(type_name);
        match (fixture.mode(), declared) {
            // Said in the user's terms: this reaches the web UI's banner and
            // readiness view, where a GDTF fixture has no file to point at.
            (None, Some(_)) => Err(format!(
                "fixture \"{}\" of \"{type_name}\" names no mode; every fixture of a GDTF \
                 fixture states its mode — add `mode \"...\"` to its venue line{}",
                fixture.name(),
                self.mode_choices(type_name),
            )),
            (Some(mode), Some(_)) => {
                let key = (type_name.to_string(), mode.to_string());
                if let Some(expanded) = self.mode_expansions.get(&key) {
                    return Ok(expanded);
                }
                Err(match self.mode_errors.get(&key) {
                    Some(reason) => format!(
                        "fixture \"{}\": mode \"{mode}\" of fixture type \"{type_name}\" did \
                         not load: {reason}",
                        fixture.name()
                    ),
                    None => format!(
                        "fixture \"{}\": mode \"{mode}\" of fixture type \"{type_name}\" was \
                         never expanded (the venue did not load through the lighting system)",
                        fixture.name()
                    ),
                })
            }
            (Some(mode), None) if self.fixture_types.contains_key(type_name) => Err(format!(
                "fixture \"{}\" names mode \"{mode}\", but fixture type \"{type_name}\" (in {}) \
                 is not GDTF-sourced and has no modes; remove the mode, or source the type \
                 from a GDTF with `from gdtf(...)`",
                fixture.name(),
                self.type_file_display(type_name)
            )),
            // A native type: its own channels.
            _ => match self.fixture_types.get(type_name) {
                Some(fixture_type) => Ok(fixture_type),
                None => Err(format!(
                    "Fixture type '{type_name}' not found for fixture \"{}\": {}",
                    fixture.name(),
                    self.fixture_type_problem(type_name)
                        .unwrap_or_else(|| "it did not load".to_string())
                )),
            },
        }
    }

    /// ", one of: \"8: RGBS\", \"9: RGBWS\" and N more" for a GDTF type's
    /// modes as read at load; empty when they are not known.
    fn mode_choices(&self, type_name: &str) -> String {
        const SHOWN: usize = 8;
        let Some(modes) = self.type_modes.get(type_name).filter(|m| !m.is_empty()) else {
            return String::new();
        };
        let listed: Vec<String> = modes
            .iter()
            .take(SHOWN)
            .map(|m| format!("\"{m}\""))
            .collect();
        let more = match modes.len().saturating_sub(SHOWN) {
            0 => String::new(),
            n => format!(" and {n} more"),
        };
        format!(", one of: {}{more}", listed.join(", "))
    }

    /// Why a venue fixture cannot be driven, or `None` when it can: what
    /// [`Self::resolve_fixture_type`] says, with the venue and its file.
    pub fn fixture_problem(&self, venue: &str, fixture: &Fixture) -> Option<String> {
        self.resolve_fixture_type(fixture).err().map(|reason| {
            let file = self.venue_file_display(venue);
            if file.is_empty() {
                format!("{reason} (venue \"{venue}\")")
            } else {
                format!("{reason} (venue \"{venue}\", {file})")
            }
        })
    }

    /// The note for a mode that matched its GDTF only after normalization.
    pub fn mode_warning(&self, fixture_type: &str, mode: &str) -> Option<&str> {
        self.mode_warnings
            .get(&(fixture_type.to_string(), mode.to_string()))
            .map(String::as_str)
    }

    /// Each fixture of `venue` as the addresses it occupies, from its own
    /// mode's expansion. A fixture whose type or mode did not load has no
    /// known footprint and is left out rather than guessed at.
    pub fn patch_spans(&self, venue: &Venue) -> Vec<patch::PatchSpan> {
        venue
            .fixtures_by_patch()
            .into_iter()
            .filter_map(|fixture| {
                let fixture_type = self.resolve_fixture_type(fixture).ok()?;
                Some(patch::PatchSpan {
                    fixture: fixture.name().to_string(),
                    universe: fixture.universe(),
                    address: fixture.start_channel(),
                    footprint: fixture_type.footprint(),
                })
            })
            .collect()
    }

    /// Fixtures of the current venue patched over part of each other's
    /// addresses (a gang at one identical span is not an overlap).
    pub fn current_venue_overlaps(&self) -> Vec<patch::PatchOverlap> {
        self.get_current_venue()
            .map(|venue| patch::venue_overlaps(&self.patch_spans(venue)))
            .unwrap_or_default()
    }

    /// Fixtures of the current venue whose footprint runs past address 512.
    pub fn current_venue_overruns(&self) -> Vec<patch::PatchOverrun> {
        self.get_current_venue()
            .map(|venue| patch::venue_overruns(&self.patch_spans(venue)))
            .unwrap_or_default()
    }

    /// Gets the current venue name.
    pub fn current_venue(&self) -> Option<&str> {
        self.current_venue.as_deref()
    }

    /// Gets the current venue object and its fixtures.
    pub fn get_current_venue(&self) -> Option<&crate::lighting::types::Venue> {
        let venue_name = self.current_venue.as_deref()?;
        self.venues.get(venue_name)
    }

    /// Resolves a logical group to concrete fixture names for the current venue.
    /// Returns an empty vector if the group cannot be resolved (graceful fallback).
    pub fn resolve_logical_group(
        &mut self,
        group_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let venue_name = self.current_venue().ok_or("No current venue selected")?;

        // Check cache first
        if let Some(cached) = self
            .group_cache
            .get(venue_name)
            .and_then(|venue_cache| venue_cache.get(group_name))
        {
            return Ok(cached.clone());
        }

        // Get the logical group definition
        let logical_group = self
            .logical_groups
            .get(group_name)
            .ok_or_else(|| format!("Logical group '{}' not found", group_name))?;

        // Get the current venue
        let venue = self
            .venues
            .get(venue_name)
            .ok_or_else(|| format!("Venue '{}' not found", venue_name))?;

        // Resolve fixtures based on constraints
        let resolved_fixtures = self.resolve_group_constraints(logical_group, venue)?;

        // Cache the result
        self.group_cache
            .entry(venue_name.to_string())
            .or_default()
            .insert(group_name.to_string(), resolved_fixtures.clone());

        Ok(resolved_fixtures)
    }

    /// Resolves a logical group with graceful fallback - returns empty vector if group cannot be resolved.
    /// This allows songs to work even when some groups aren't available at the current venue.
    pub fn resolve_logical_group_graceful(&mut self, group_name: &str) -> Vec<String> {
        self.resolve_logical_group_graceful_inner(group_name, &mut Vec::new())
    }

    /// `FallbackTo` is an author-supplied group name that nothing validates, so
    /// `a -> b -> a` (or `a -> a`) is expressible. Every group takes the
    /// fallback branch when no venue is selected, which is the normal state for
    /// a config UI, so the cycle has to be broken here rather than assumed away.
    fn resolve_logical_group_graceful_inner(
        &mut self,
        group_name: &str,
        seen: &mut Vec<String>,
    ) -> Vec<String> {
        if seen.iter().any(|g| g == group_name) {
            warn!(
                group = group_name,
                chain = ?seen,
                "Cycle in logical group FallbackTo constraints; resolving to no fixtures"
            );
            return Vec::new();
        }
        seen.push(group_name.to_string());

        match self.resolve_logical_group(group_name) {
            Ok(fixtures) => fixtures,
            Err(_) => {
                // Check if the group has a FallbackTo constraint
                let fallback_group =
                    if let Some(logical_group) = self.logical_groups.get(group_name) {
                        logical_group.constraints().iter().find_map(|constraint| {
                            if let GroupConstraint::FallbackTo(fallback_group) = constraint {
                                Some(fallback_group.clone())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    };

                if let Some(fallback_group) = fallback_group {
                    return self.resolve_logical_group_graceful_inner(&fallback_group, seen);
                }

                Vec::new()
            }
        }
    }

    /// A fixture type as the fixture test drives it: a native type's own
    /// channels, or a GDTF type's expansion in `mode` (already expanded for
    /// a venue, else expanded now through the distill cache). A GDTF type
    /// needs a mode and a native type takes none.
    pub fn type_in_mode(
        &self,
        type_name: &str,
        mode: Option<&str>,
        base_path: &Path,
    ) -> Result<FixtureType, TypeInModeError> {
        match (self.referential.get(type_name), mode) {
            (Some(_), None) => Err(TypeInModeError::Mode(format!(
                "\"{type_name}\" is a GDTF fixture: choose one of its modes"
            ))),
            (Some(declared), Some(mode)) => {
                let key = (type_name.to_string(), mode.to_string());
                if let Some(expanded) = self.mode_expansions.get(&key) {
                    return Ok(expanded.clone());
                }
                Self::expand_mode(type_name, declared, mode, base_path)
                    .map(|(expanded, _)| expanded)
                    .map_err(|e| {
                        TypeInModeError::Mode(format!(
                            "mode \"{mode}\" of \"{type_name}\" cannot be driven: {e}"
                        ))
                    })
            }
            (None, Some(_)) if self.fixture_types.contains_key(type_name) => {
                Err(TypeInModeError::Mode(format!(
                    "\"{type_name}\" is written by hand and has no modes"
                )))
            }
            (None, _) => self
                .fixture_types
                .get(type_name)
                .cloned()
                .ok_or_else(|| TypeInModeError::Unknown(type_name.to_string())),
        }
    }

    /// The GDTF types' archives, project-relative, by type name.
    pub fn gdtf_archive(&self, type_name: &str) -> Option<&str> {
        self.referential
            .get(type_name)
            .and_then(|t| t.source())
            .map(|s| s.path.as_str())
    }

    /// Gets all fixtures from the current venue for effects engine registration
    pub fn get_current_venue_fixtures(
        &self,
    ) -> Result<Vec<crate::lighting::effects::FixtureInfo>, Box<dyn Error>> {
        let venue_name = self.current_venue().ok_or("No current venue selected")?;
        let venue = self
            .venues
            .get(venue_name)
            .ok_or_else(|| format!("Venue '{}' not found", venue_name))?;

        let mut fixture_infos = Vec::new();

        for (name, fixture) in venue.fixtures() {
            // The fixture's own mode's channels (or its native type's). A
            // fixture that cannot be driven fails the venue, as a missing
            // type always has: registering the rest would light a rig with
            // a hole in it and no error to say so.
            let fixture_type = self.resolve_fixture_type(fixture)?;

            let mut fixture_info = fixture_info_for(
                name,
                fixture.universe(),
                fixture.start_channel(),
                fixture.fixture_type(),
                fixture_type,
            );
            fixture_info.position = fixture.position();
            fixture_info.rotation = fixture.rotation();
            fixture_info.beam_angle = fixture.beam_angle();

            fixture_infos.push(fixture_info);
        }

        Ok(fixture_infos)
    }

    /// Resolves group constraints to fixture names.
    fn resolve_group_constraints(
        &self,
        logical_group: &LogicalGroup,
        venue: &Venue,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let mut candidates: Vec<&Fixture> = venue.fixtures().values().collect();
        let mut min_count = 1;
        let mut max_count = candidates.len();
        let mut allow_empty = false;
        let mut preferred_tags: Vec<String> = Vec::new();

        // Apply constraints
        for constraint in logical_group.constraints() {
            match constraint {
                GroupConstraint::AllOf(required_tags) => {
                    candidates.retain(|fixture| {
                        required_tags.iter().all(|tag| fixture.tags().contains(tag))
                    });
                }
                GroupConstraint::AnyOf(any_tags) => {
                    candidates
                        .retain(|fixture| any_tags.iter().any(|tag| fixture.tags().contains(tag)));
                }
                GroupConstraint::Prefer(tags) => {
                    // Store preferred tags for later sorting (after count constraints)
                    preferred_tags = tags.clone();
                }
                GroupConstraint::MinCount(count) => {
                    min_count = *count;
                }
                GroupConstraint::MaxCount(count) => {
                    max_count = *count;
                }
                GroupConstraint::AllowEmpty(allow) => {
                    allow_empty = *allow;
                }
                GroupConstraint::FallbackTo(_) => {
                    // FallbackTo is handled at a higher level in resolve_logical_group_graceful
                    // This constraint is processed during group resolution, not constraint resolution
                }
            }
        }

        // Apply count constraints
        if candidates.len() < min_count {
            if allow_empty {
                return Ok(Vec::new());
            } else {
                return Err(format!(
                    "Not enough fixtures found for group '{}': found {}, required {}",
                    logical_group.name(),
                    candidates.len(),
                    min_count
                )
                .into());
            }
        }

        // Sort fixtures: first by preference (if Prefer constraint exists), then by name
        // This ensures preferred fixtures are selected first, but within each preference level,
        // fixtures are sorted lexicographically for consistent chase ordering
        if !preferred_tags.is_empty() {
            // Sort by preference score first, then by name
            candidates.sort_by(|a, b| {
                let a_score = preferred_tags
                    .iter()
                    .filter(|tag| a.tags().contains(tag))
                    .count();
                let b_score = preferred_tags
                    .iter()
                    .filter(|tag| b.tags().contains(tag))
                    .count();
                // First compare by preference score (higher is better)
                match b_score.cmp(&a_score) {
                    std::cmp::Ordering::Equal => {
                        // Tiebreaker: sort by name
                        a.name().cmp(b.name())
                    }
                    other => other,
                }
            });
        } else {
            // No preference constraint - just sort by name
            candidates.sort_by(|a, b| a.name().cmp(b.name()));
        }

        // Take up to max_count fixtures
        let selected: Vec<String> = candidates
            .iter()
            .take(max_count)
            .map(|fixture| fixture.name().to_string())
            .collect();

        Ok(selected)
    }

    /// Resolves group names in an effect's target_fixtures to actual fixture names.
    /// Each name in `target_fixtures` is looked up as a logical group; if it resolves,
    /// the individual fixture names replace the group name.
    pub fn resolve_effect_groups(
        &mut self,
        mut effect: super::EffectInstance,
    ) -> super::EffectInstance {
        let mut resolved_fixtures = Vec::new();
        for group_name in &effect.target_fixtures {
            let fixtures = self.resolve_logical_group_graceful(group_name);
            resolved_fixtures.extend(fixtures);
        }
        effect.target_fixtures = resolved_fixtures;
        effect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::lighting::Directories;
    use std::collections::HashMap;

    #[test]
    fn a_fallback_cycle_resolves_to_nothing_instead_of_overflowing_the_stack() {
        // `FallbackTo` names a group and nothing validates that the graph is
        // acyclic. With no venue selected every group takes the fallback
        // branch, which is the normal state for the config UI — and the
        // lighting editor reaches this over HTTP.
        let mut system = LightingSystem::new();
        system.logical_groups.insert(
            "a".to_string(),
            LogicalGroup::new(
                "a".to_string(),
                vec![GroupConstraint::FallbackTo("b".to_string())],
            ),
        );
        system.logical_groups.insert(
            "b".to_string(),
            LogicalGroup::new(
                "b".to_string(),
                vec![GroupConstraint::FallbackTo("a".to_string())],
            ),
        );

        assert!(system.resolve_logical_group_graceful("a").is_empty());

        // A group that falls back to itself is the same problem, one hop short.
        system.logical_groups.insert(
            "self".to_string(),
            LogicalGroup::new(
                "self".to_string(),
                vec![GroupConstraint::FallbackTo("self".to_string())],
            ),
        );
        assert!(system.resolve_logical_group_graceful("self").is_empty());
    }

    #[test]
    fn a_venue_file_that_does_not_parse_leaves_the_others_loaded() {
        // The venue-group migration advice is raised as a parse error, and the
        // loader used to swallow it silently — dropping every venue in the file
        // and surfacing as "Venue 'x' not found" much later.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("legacy.light"),
            "venue \"old\" {\n  fixture \"W1\" RGBW_Par @ 1:1\n  group \"wash\" [\"W1\"]\n}\n",
        )
        .expect("write");
        std::fs::write(
            dir.path().join("current.light"),
            "venue \"new\" {\n  fixture \"W1\" RGBW_Par @ 1:1 tags [\"wash\"]\n}\n",
        )
        .expect("write");

        let mut system = LightingSystem::new();
        system
            .load_venues_directory(dir.path())
            .expect("directory loads");

        assert!(
            system.venues.contains_key("new"),
            "a sibling file's failure must not take the good one with it"
        );
        assert!(
            !system.venues.contains_key("old"),
            "the legacy file genuinely does not parse"
        );
    }

    #[test]
    fn rich_channel_syntax_loads_from_fixture_files_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rich = "fixture_type \"Mover\" {\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n  channel \"dimmer\" @ 3\n}\n";
        std::fs::write(dir.path().join("mover.fixture"), rich).unwrap();
        std::fs::write(
            dir.path().join("wrong_home.light"),
            rich.replace("\"Mover\"", "\"Stray\""),
        )
        .unwrap();
        let mut system = LightingSystem::new();
        system
            .load_fixture_types_directory(dir.path())
            .expect("directory loads");
        assert!(system.fixture_types.contains_key("Mover"));
        assert!(
            !system.fixture_types.contains_key("Stray"),
            "rich syntax in a .light file is refused, loudly"
        );
        let mover = &system.fixture_types["Mover"];
        assert_eq!(mover.channel_defs()["pan"].fine, Some(2));
    }

    #[test]
    fn cell_syntax_loads_from_fixture_files_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rich = "fixture_type \"Bar\" {\n  cell \"1\" at (-0.1, 0, 0) {\n    channel \"red\" @ 1\n    channel \"green\" @ 2\n  }\n  cell \"2\" at (0.1, 0, 0) {\n    channel \"red\" @ 3\n    channel \"green\" @ 4\n  }\n}\n";
        std::fs::write(dir.path().join("bar.fixture"), rich).unwrap();
        std::fs::write(
            dir.path().join("wrong_home.light"),
            rich.replace("\"Bar\"", "\"Stray\""),
        )
        .unwrap();
        let mut system = LightingSystem::new();
        system
            .load_fixture_types_directory(dir.path())
            .expect("directory loads");
        assert!(system.fixture_types.contains_key("Bar"));
        assert!(
            !system.fixture_types.contains_key("Stray"),
            "cell syntax in a .light file is refused, loudly"
        );
        let bar = &system.fixture_types["Bar"];
        assert_eq!(bar.cells().len(), 2);
    }

    /// A referential mover whose GDTF yaws its yoke (the MagicDot SX's
    /// shape) is aimed through that geometry: the type carries a
    /// calibration, and a plain one carries the identity.
    #[test]
    fn referential_fixture_types_aim_through_their_rig() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let library = base.join("lighting/library");
        let ft_dir = base.join("lighting/fixture_types");
        std::fs::create_dir_all(&library).expect("mkdir");
        std::fs::create_dir_all(&ft_dir).expect("mkdir");
        let yawed = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.replace(
            r#"<Axis Name="Yoke" Model="Yoke" Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.1}{0,0,0,1}">"#,
            r#"<Axis Name="Yoke" Model="Yoke" Position="{0,1,0,0}{-1,0,0,0}{0,0,1,-0.1}{0,0,0,1}">"#,
        );
        assert_ne!(yawed, crate::lighting::gdtf::SYNTHETIC_DESCRIPTION);
        for (file, xml) in [
            ("plain.gdtf", crate::lighting::gdtf::SYNTHETIC_DESCRIPTION),
            ("yawed.gdtf", yawed.as_str()),
        ] {
            std::fs::write(
                library.join(file),
                crate::lighting::gdtf::build_zip(&[("description.xml", xml.as_bytes())]),
            )
            .expect("write gdtf");
        }
        std::fs::write(
            ft_dir.join("movers.fixture"),
            "fixture_type \"Plain\"\n  from gdtf(\"lighting/library/plain.gdtf\")\n{ }\n\n\
             fixture_type \"Yawed\"\n  from gdtf(\"lighting/library/yawed.gdtf\")\n{ }\n",
        )
        .expect("write fixture");

        let mut system = LightingSystem::new();
        system.load_fixture_types_directory(&ft_dir).expect("loads");
        let expand = |name: &str| {
            LightingSystem::expand_mode(name, &system.referential[name], "Mover 16bit", base)
                .expect("expands")
                .0
        };
        let plain = expand("Plain").aim().expect("a rig gives a calibration");
        assert!(plain.frame_is_identity(), "{plain:?}");
        assert!(
            (plain.tilt_to_lens[2] + 0.06).abs() < 1e-9,
            "the lens offset comes with it: {plain:?}"
        );
        let yawed = expand("Yawed").aim().expect("a rig gives a calibration");
        assert!(!yawed.frame_is_identity(), "{yawed:?}");
        assert!((yawed.pre[1][0] - 1.0).abs() < 1e-6, "{yawed:?}");
    }

    #[test]
    fn referential_fixture_types_expand_through_the_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let library = base.join("lighting/library");
        let ft_dir = base.join("lighting/fixture_types");
        std::fs::create_dir_all(&library).expect("mkdir");
        std::fs::create_dir_all(&ft_dir).expect("mkdir");
        std::fs::write(
            library.join("synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .expect("write gdtf");
        std::fs::write(
            ft_dir.join("brick.fixture"),
            "fixture_type \"Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n  movement { max_pan_speed: 240.0deg/s }\n}\n",
        )
        .expect("write fixture");

        let mut system = LightingSystem::new();
        system.load_fixture_types_directory(&ft_dir).expect("loads");

        assert!(
            !system.fixture_types.contains_key("Brick"),
            "a GDTF fixture type is a declaration; only a mode has channels"
        );
        let expand = |system: &LightingSystem| {
            LightingSystem::expand_mode("Brick", &system.referential["Brick"], "8: RGBS", base)
                .expect("expands")
                .0
        };
        let brick = expand(&system);
        assert_eq!(brick.channels().get("red"), Some(&1));
        assert_eq!(brick.channels().get("strobe"), Some(&4));
        assert_eq!(brick.strobe_dmx_offset(), Some(7));
        assert_eq!(brick.max_strobe_frequency(), Some(25.0));
        assert_eq!(brick.movement().max_pan_speed, Some(240.0));
        assert_eq!(
            brick.source().map(|s| s.path.as_str()),
            Some("lighting/library/synth.gdtf"),
            "the expansion keeps its provenance"
        );

        // The expansion landed in the per-project cache (hit/miss/corruption
        // mechanics are unit-tested on DistillCache itself; the key covers
        // the archive bytes, so a changed archive is a fresh distillation
        // by design), and the rig beside it in the asset store.
        let expansions = || {
            std::fs::read_dir(base.join("lighting/.cache"))
                .expect("cache dir")
                .filter(|e| e.as_ref().unwrap().path().is_file())
                .count()
        };
        assert_eq!(expansions(), 1);
        let rig = brick.rig().expect("a referential type gets a rig");
        assert!(base.join("lighting/.cache/assets").join(rig).is_file());

        // A reload with the same inputs resolves to the same single entry.
        let mut reloaded = LightingSystem::new();
        reloaded
            .load_fixture_types_directory(&ft_dir)
            .expect("loads");
        let again = expand(&reloaded);
        assert_eq!(expansions(), 1);
        assert_eq!(again.rig(), Some(rig));
    }

    /// A project with the synthetic two-mode GDTF ("8: RGBS": red, green,
    /// blue, strobe; "Mover 16bit": pan, tilt, a fifth byte), `types` as its
    /// fixture types file and `venues` as its venues file, loaded with
    /// `current` as the current venue.
    fn modal_project(
        types: &str,
        venues: &str,
        current: &str,
    ) -> (tempfile::TempDir, LightingSystem) {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        for sub in [
            "lighting/library",
            "lighting/fixture_types",
            "lighting/venues",
        ] {
            std::fs::create_dir_all(base.join(sub)).expect("mkdir");
        }
        std::fs::write(
            base.join("lighting/library/synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .expect("write gdtf");
        std::fs::write(base.join("lighting/fixture_types/types.fixture"), types).expect("write");
        std::fs::write(base.join("lighting/venues/house.venue"), venues).expect("write");
        let config = Lighting::new(
            Some(current.to_string()),
            None,
            None,
            Some(crate::config::lighting::Directories::new(
                Some("lighting/fixture_types".to_string()),
                Some("lighting/venues".to_string()),
            )),
        );
        let mut system = LightingSystem::new();
        system.load(&config, base).expect("loads");
        (dir, system)
    }

    const BRICK_TYPE: &str = "fixture_type \"Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n  movement { max_pan_speed: 240deg/s }\n}\n\nfixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n";

    fn info<'a>(
        infos: &'a [crate::lighting::effects::FixtureInfo],
        name: &str,
    ) -> &'a crate::lighting::effects::FixtureInfo {
        infos.iter().find(|f| f.name == name).expect(name)
    }

    #[test]
    fn two_fixtures_of_one_type_in_two_modes_get_their_own_channels() {
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n  fixture \"Spot\" Brick mode \"Mover 16bit\" @ 1:10\n  fixture \"Same\" Brick mode \"8: RGBS\" @ 1:20\n}\n",
            "house",
        );
        let infos = system.get_current_venue_fixtures().expect("venue resolves");
        let wash = info(&infos, "Wash");
        assert_eq!(wash.channels.get("red"), Some(&1), "its own mode");
        assert!(!wash.channels.contains_key("pan"));
        let spot = info(&infos, "Spot");
        assert_eq!(spot.channels.get("pan"), Some(&1), "its own mode");
        assert_eq!(spot.channel_defs["pan"].fine, Some(2));
        assert!(!spot.channels.contains_key("red"));
        assert_eq!(spot.fixture_type, "Brick", "still the one type");
        // The type's body applies to every mode.
        assert_eq!(spot.movement.max_pan_speed, Some(240.0));
        // One mode, one expansion, whoever names it.
        assert_eq!(info(&infos, "Same").channels, wash.channels);
        assert!(system
            .fixture_problem("house", &system.venues["house"].fixtures()["Spot"])
            .is_none());
        // With no fixture in hand, the type is a declaration: no channels.
        assert!(!system.fixture_types.contains_key("Brick"));
        assert!(system.gdtf_types_iter().any(|(name, _)| name == "Brick"));
    }

    #[test]
    fn a_gdtf_fixture_without_a_mode_is_a_load_error_listing_the_modes() {
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Spot\" Brick mode \"Mover 16bit\" @ 1:1\n}\n\nvenue \"bare\" {\n  fixture \"Lost\" Brick @ 1:1\n}\n",
            "house",
        );
        let infos = system
            .get_current_venue_fixtures()
            .expect("named modes load");
        assert_eq!(info(&infos, "Spot").channels.get("tilt"), Some(&3));
        assert_eq!(
            system.fixture_type_problem("Brick"),
            None,
            "the type loaded"
        );

        let lost = &system.venues["bare"].fixtures()["Lost"];
        let problem = system.fixture_problem("bare", lost).expect("no mode");
        assert!(
            problem.starts_with(
                "fixture \"Lost\" of \"Brick\" names no mode; every fixture of a GDTF \
                 fixture states its mode — add `mode \"...\"` to its venue line, one of: \
                 \"8: RGBS\", \"Mover 16bit\""
            ),
            "{problem}"
        );
        assert!(
            problem.contains("house.venue"),
            "names the venue file: {problem}"
        );
    }

    #[test]
    fn a_long_mode_list_is_capped_in_the_error() {
        let modes: String = (1..=12)
            .map(|n| {
                format!(
                    r#"<DMXMode Name="M{n:02}" Geometry="Base"><DMXChannels><DMXChannel Offset="1" Geometry="Base"><LogicalChannel Attribute="Dimmer"><ChannelFunction Name="Dimmer" Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel></DMXChannels></DMXMode>"#
                )
            })
            .collect();
        let description = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
            .replace("<DMXModes>", &format!("<DMXModes>{modes}"));
        assert_ne!(description, crate::lighting::gdtf::SYNTHETIC_DESCRIPTION);
        let (dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Lost\" Brick @ 1:1\n}\n",
            "house",
        );
        std::fs::write(
            dir.path().join("lighting/library/synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[("description.xml", description.as_bytes())]),
        )
        .unwrap();
        // The modes are read once per types load; a changed archive is a
        // types reload.
        system.reload_fixture_types().unwrap();
        let err = system.get_current_venue_fixtures().unwrap_err().to_string();
        assert!(
            err.ends_with(
                "one of: \"M01\", \"M02\", \"M03\", \"M04\", \"M05\", \"M06\", \"M07\", \
                 \"M08\" and 6 more"
            ),
            "{err}"
        );
    }

    /// A project as it stands once the shows are migrated: a record naming
    /// the archive (a custom name, movement limits, no mode), a hand-written
    /// type beside it, and venue lines that each state a mode. It loads as
    /// it did before the default went away.
    #[test]
    fn a_migrated_production_project_loads_as_before() {
        let (_dir, system) = modal_project(
            "fixture_type \"Custom-Name\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n  \
             movement { max_pan_speed: 240deg/s }\n}\n\nfixture_type \"Par\" {\n  channels: 1\n  \
             channel_map: { \"dimmer\": 1 }\n}\n",
            "venue \"house\" {\n  fixture \"Wash 1\" \"Custom-Name\" mode \"8: RGBS\" @ 1:1\n  \
             fixture \"Wash 2\" \"Custom-Name\" mode \"8: RGBS\" @ 1:5\n  \
             fixture \"Spot\" \"Custom-Name\" mode \"Mover 16bit\" @ 1:9\n  \
             fixture \"Dim\" Par @ 1:20\n}\n",
            "house",
        );
        let infos = system.get_current_venue_fixtures().expect("loads");
        assert_eq!(infos.len(), 4);
        assert_eq!(info(&infos, "Wash 2").channels.get("red"), Some(&1));
        assert_eq!(info(&infos, "Wash 2").fixture_type, "Custom-Name");
        assert_eq!(info(&infos, "Spot").channels.get("pan"), Some(&1));
        assert_eq!(info(&infos, "Spot").movement.max_pan_speed, Some(240.0));
        assert_eq!(info(&infos, "Dim").channels.get("dimmer"), Some(&1));
        assert!(
            !system
                .gdtf_types_iter()
                .any(|(name, _)| name == "Synth Brick"),
            "a recorded archive is not a second fixture type"
        );
        assert!(system.current_venue_report().is_empty());
        assert!(system.library_findings().is_empty());
    }

    /// A first-run project: the Lighting area's files in the default
    /// directories, and a config naming a current venue but no directories.
    fn first_run_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        for sub in [
            "lighting/library",
            "lighting/fixture_types",
            "lighting/venues",
        ] {
            std::fs::create_dir_all(base.join(sub)).expect("mkdir");
        }
        std::fs::write(
            base.join("lighting/library/synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .expect("write gdtf");
        std::fs::write(
            base.join("lighting/fixture_types/par.light"),
            "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .expect("write");
        std::fs::write(
            base.join("lighting/venues/rig.light"),
            "venue \"rig\" {\n  fixture \"Dim\" Par @ 1:1\n  \
             fixture \"Brick\" \"Synth Brick\" mode \"8: RGBS\" @ 1:10\n}\n",
        )
        .expect("write");
        dir
    }

    fn load_with(base: &Path, directories: Option<Directories>) -> LightingSystem {
        let config = Lighting::new(Some("rig".to_string()), None, None, directories);
        let mut system = LightingSystem::new();
        system.load(&config, base).expect("loads");
        system
    }

    #[test]
    fn a_config_without_directories_loads_the_default_ones() {
        // The hardware repro: `current_venue: rig`, no `directories`, and the
        // files where the web UI writes them. It used to be "Venue 'rig' not
        // found".
        let dir = first_run_project();
        let system = load_with(dir.path(), None);
        let infos = system
            .get_current_venue_fixtures()
            .expect("the venue registers");
        assert_eq!(infos.len(), 2);
        assert_eq!(info(&infos, "Dim").channels.get("dimmer"), Some(&1));
        // The GDTF from the library, in the mode its line names.
        assert_eq!(info(&infos, "Brick").channels.get("red"), Some(&1));
        assert_eq!(system.venues_dir(), Some("lighting/venues"));
        assert!(system.current_venue_report().is_empty());
    }

    #[test]
    fn naming_one_directory_leaves_the_other_at_its_default() {
        let dir = first_run_project();
        let base = dir.path();
        std::fs::create_dir_all(base.join("stages")).unwrap();
        std::fs::rename(
            base.join("lighting/venues/rig.light"),
            base.join("stages/rig.light"),
        )
        .unwrap();
        let system = load_with(
            base,
            Some(Directories::new(None, Some("stages".to_string()))),
        );
        // Par comes from the default fixture types directory.
        assert_eq!(system.get_current_venue_fixtures().unwrap().len(), 2);
        assert_eq!(system.venues_dir(), Some("stages"));
    }

    #[test]
    fn a_missing_default_directory_is_silently_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Lighting::new(None, None, None, None);
        let mut system = LightingSystem::new();
        system
            .load(&config, dir.path())
            .expect("a project without lighting loads");
        assert_eq!(system.venues_iter().count(), 0);
        assert_eq!(system.fixture_types_iter().count(), 0);
        assert!(system.fixture_type_file_errors.is_empty());
        assert!(system.library_findings().is_empty());
    }

    #[test]
    fn a_missing_configured_directory_reports_as_before() {
        // Before defaults, a configured directory that was not there loaded
        // as empty, kept as the venues directory, and the current venue was
        // the one not found: still so.
        let dir = tempfile::tempdir().expect("tempdir");
        let system = load_with(
            dir.path(),
            Some(Directories::new(
                Some("nowhere/types".to_string()),
                Some("nowhere/venues".to_string()),
            )),
        );
        assert_eq!(system.venues_iter().count(), 0);
        assert_eq!(system.venues_dir(), Some("nowhere/venues"));
        let err = system.get_current_venue_fixtures().unwrap_err();
        assert!(err.to_string().contains("Venue 'rig' not found"), "{err}");
    }

    #[test]
    fn reloads_read_the_default_directories_too() {
        let dir = first_run_project();
        let mut system = load_with(dir.path(), None);
        std::fs::write(
            dir.path().join("lighting/venues/rig.light"),
            "venue \"rig\" {\n  fixture \"Dim\" Par @ 1:1\n}\n",
        )
        .unwrap();
        system.reload_venues().expect("reloads venues");
        assert_eq!(system.get_current_venue_fixtures().unwrap().len(), 1);
        system.reload_fixture_types().expect("reloads types");
        assert_eq!(system.get_current_venue_fixtures().unwrap().len(), 1);
    }

    #[test]
    fn a_mode_on_a_native_type_is_a_load_error_not_ignored() {
        let (_dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Dim\" Par mode \"8: RGBS\" @ 1:1\n}\n",
            "house",
        );
        let dim = &system.venues["house"].fixtures()["Dim"];
        let problem = system.fixture_problem("house", dim).expect("refused");
        assert!(
            problem.starts_with(
                "fixture \"Dim\" names mode \"8: RGBS\", but fixture type \"Par\" (in \
                 types.fixture) is not GDTF-sourced and has no modes"
            ),
            "{problem}"
        );
        // It fails the venue the way a missing type does.
        let err = system.get_current_venue_fixtures().unwrap_err().to_string();
        assert!(err.contains("fixture \"Dim\" names mode"), "{err}");
        system.current_venue = Some("nowhere".to_string());
        assert!(system.get_current_venue_fixtures().is_err());
    }

    #[test]
    fn a_mode_that_does_not_match_fails_its_venue_by_name() {
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n  fixture \"Typo\" Brick mode \"Mover 61bit\" @ 1:10\n}\n",
            "house",
        );
        let err = system.get_current_venue_fixtures().unwrap_err().to_string();
        assert!(
            err.starts_with(
                "fixture \"Typo\": mode \"Mover 61bit\" of fixture type \"Brick\" did not load:"
            ),
            "{err}"
        );
        assert!(err.contains("no mode matching"), "{err}");
        // The type is untouched, and the overlap check skips the unknown.
        assert_eq!(system.fixture_type_problem("Brick"), None);
        assert!(system.current_venue_overlaps().is_empty());
    }

    #[test]
    fn a_drifted_mode_name_loads_with_a_warning() {
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Spot\" Brick mode \"mover 16BIT\" @ 1:1\n}\n",
            "house",
        );
        let infos = system
            .get_current_venue_fixtures()
            .expect("normalized match loads");
        assert_eq!(info(&infos, "Spot").channels.get("pan"), Some(&1));
        let note = system
            .mode_warning("Brick", "mover 16BIT")
            .expect("the normalization is reported");
        assert!(note.contains("only after normalizing"), "{note}");
        assert!(system.mode_warning("Brick", "8: RGBS").is_none());
    }

    #[test]
    fn every_venue_is_expanded_so_a_switch_parses_nothing() {
        let (dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n}\n\nvenue \"club\" {\n  fixture \"Spot\" Brick mode \"Mover 16bit\" @ 1:1\n}\n",
            "house",
        );
        assert!(system
            .mode_expansions
            .contains_key(&("Brick".to_string(), "Mover 16bit".to_string())));
        // With the archive gone, the other venue still resolves: its mode
        // was expanded at load, not on the switch.
        std::fs::remove_file(dir.path().join("lighting/library/synth.gdtf")).unwrap();
        system.current_venue = Some("club".to_string());
        let infos = system
            .get_current_venue_fixtures()
            .expect("already expanded");
        assert_eq!(info(&infos, "Spot").channels.get("pan"), Some(&1));
    }

    #[test]
    fn a_venue_reload_expands_a_newly_named_mode_and_drops_unused_ones() {
        let (dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n}\n",
            "house",
        );
        let expanded = |system: &LightingSystem| {
            let mut modes: Vec<String> = system
                .mode_expansions
                .keys()
                .map(|(_, mode)| mode.clone())
                .collect();
            modes.sort();
            modes
        };
        assert_eq!(expanded(&system), ["8: RGBS"]);
        std::fs::write(
            dir.path().join("lighting/venues/house.venue"),
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"Mover 16bit\" @ 1:1\n}\n",
        )
        .unwrap();
        system.reload_venues().unwrap();
        let infos = system.get_current_venue_fixtures().expect("re-moded");
        assert_eq!(info(&infos, "Wash").channels.get("pan"), Some(&1));
        std::fs::write(
            dir.path().join("lighting/venues/house.venue"),
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n}\n",
        )
        .unwrap();
        system.reload_venues().unwrap();
        assert_eq!(expanded(&system), ["8: RGBS"], "the mover mode is dropped");
    }

    #[test]
    fn a_type_edit_reaches_the_system_through_a_types_reload() {
        let (dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n}\n",
            "house",
        );
        // Renamed, and the venue line follows (re-moded on the way): what
        // the fixture page's settings save and the venue editor write.
        std::fs::write(
            dir.path().join("lighting/fixture_types/types.fixture"),
            BRICK_TYPE.replace("\"Brick\"", "\"Pixel\""),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("lighting/venues/house.venue"),
            "venue \"house\" {\n  fixture \"Wash\" Pixel mode \"Mover 16bit\" @ 1:1\n}\n",
        )
        .unwrap();
        system.reload_fixture_types().unwrap();
        system.reload_venues().unwrap();
        assert!(system.fixture_type_problem("Brick").is_some());
        let infos = system
            .get_current_venue_fixtures()
            .expect("the renamed type loads");
        assert_eq!(info(&infos, "Wash").channels.get("pan"), Some(&1));
    }

    #[test]
    fn a_fixture_s_footprint_is_its_own_mode_s() {
        // The mover mode is five bytes; "8: RGBS" four. Re-moding Spot
        // grows it into Wash's first address — the line's patch unchanged.
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Spot\" Brick mode \"8: RGBS\" @ 1:1\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:5\n}\n",
            "house",
        );
        assert!(system.current_venue_overlaps().is_empty());
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Spot\" Brick mode \"Mover 16bit\" @ 1:1\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:5\n}\n",
            "house",
        );
        let spans = system.patch_spans(&system.venues["house"]);
        assert_eq!(
            spans
                .iter()
                .map(|s| (s.fixture.as_str(), s.footprint))
                .collect::<Vec<_>>(),
            [("Spot", 5), ("Wash", 4)]
        );
        let overlaps = system.current_venue_overlaps();
        assert_eq!(overlaps.len(), 1);
        assert_eq!((overlaps[0].from, overlaps[0].to), (5, 5));
        assert_eq!(
            (overlaps[0].first.as_str(), overlaps[0].second.as_str()),
            ("Spot", "Wash")
        );
    }

    #[test]
    fn a_plain_project_has_nothing_to_report() {
        // Every GDTF line states its mode, no overlaps (two pars ganged at one address
        // are deliberate): nothing new is logged or reported.
        let (_dir, system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n  fixture \"Dim1\" Par @ 1:20\n  \
             fixture \"Dim2\" Par @ 1:20\n}\n",
            "house",
        );
        assert!(system.venue_problems("house").is_empty());
        assert!(system.venue_problem("house").is_none());
        assert!(system.current_venue_overlaps().is_empty());
        assert!(system.current_venue_overruns().is_empty());
        assert!(system.current_venue_report().is_empty());
        assert_eq!(system.logged_venue_report, Some(Vec::new()));
    }

    #[test]
    fn only_the_current_venue_is_reported_one_line_per_kind() {
        let (dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"A\" Par mode \"x\" @ 1:1\n  fixture \"B\" Par mode \"x\" @ 1:2\n  \
             fixture \"C\" Par mode \"x\" @ 1:3\n  fixture \"D\" Par mode \"x\" @ 1:4\n  \
             fixture \"Spot\" Brick mode \"Mover 16bit\" @ 1:10\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:14\n  \
             fixture \"End\" Brick mode \"8: RGBS\" @ 1:511\n}\n\nvenue \"other\" {\n  fixture \"Z\" Nope @ 1:1\n}\n",
            "house",
        );
        let lines = system.current_venue_report();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].starts_with("4 fixture(s) cannot be driven, so the venue will not light: \"A\", \"B\", \"C\" and 1 more; first: fixture \"A\" names mode"),
            "{}",
            lines[0]
        );
        assert!(lines[1].starts_with("1 patch overlap(s)"), "{}", lines[1]);
        assert!(lines[2].contains("\"End\""), "{}", lines[2]);
        assert!(!lines.iter().any(|l| l.contains("\"Z\"")), "other venue");
        // The other venue's problem is there for whoever asks.
        assert_eq!(system.venue_problem("other").unwrap().0, "Z");
        assert_eq!(system.venue_problem("house").unwrap().0, "A");

        // An unchanged reload does not change what was logged.
        let logged = system.logged_venue_report.clone();
        system.reload_venues().unwrap();
        assert_eq!(system.logged_venue_report, logged);
        let _ = dir;
    }

    #[test]
    fn a_mode_that_failed_is_retried_on_the_next_reload() {
        let (dir, mut system) = modal_project(
            BRICK_TYPE,
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"8: RGBS\" @ 1:1\n}\n",
            "house",
        );
        // The archive goes missing just as a fixture is re-moded.
        let archive = dir.path().join("lighting/library/synth.gdtf");
        let bytes = std::fs::read(&archive).unwrap();
        std::fs::remove_file(&archive).unwrap();
        std::fs::write(
            dir.path().join("lighting/venues/house.venue"),
            "venue \"house\" {\n  fixture \"Wash\" Brick mode \"Mover 16bit\" @ 1:1\n}\n",
        )
        .unwrap();
        system.reload_venues().unwrap();
        assert!(system.get_current_venue_fixtures().is_err());
        assert!(system.venue_problem("house").is_some());

        // It comes back; the next reload tries again and resolves.
        std::fs::write(&archive, bytes).unwrap();
        system.reload_venues().unwrap();
        let infos = system.get_current_venue_fixtures().expect("retried");
        assert_eq!(info(&infos, "Wash").channels.get("pan"), Some(&1));
        assert!(system.mode_errors.is_empty());
        assert!(system.venue_problem("house").is_none());
    }

    #[test]
    fn referential_failures_skip_loudly_and_natives_still_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let ft_dir = base.join("lighting/fixture_types");
        std::fs::create_dir_all(&ft_dir).expect("mkdir");
        std::fs::write(
            ft_dir.join("mixed.fixture"),
            "fixture_type \"Ghost\"\n  from gdtf(\"lighting/library/missing.gdtf\")\n{ }\n\nfixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .expect("write");

        let mut system = LightingSystem::new();
        system.load_fixture_types_directory(&ft_dir).expect("loads");
        assert!(
            !system.fixture_types.contains_key("Ghost"),
            "a failed expansion must not register an empty shell"
        );
        assert!(system.fixture_types.contains_key("Par"));
    }

    #[test]
    fn a_type_that_did_not_load_says_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let ft_dir = base.join("lighting/fixture_types");
        std::fs::create_dir_all(&ft_dir).expect("mkdir");
        std::fs::write(
            ft_dir.join("mixed.fixture"),
            "fixture_type \"Ghost\"\n  from gdtf(\"lighting/library/missing.gdtf\")\n{ }\n\nfixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .expect("write");
        std::fs::write(ft_dir.join("broken.light"), "fixture_type {{{").expect("write");

        let mut system = LightingSystem::new();
        system.load_fixture_types_directory(&ft_dir).expect("loads");
        assert_eq!(system.fixture_type_problem("Par"), None);
        // A GDTF type is a declaration: its archive is read for a mode a
        // fixture names, and that is where a missing one says so.
        assert_eq!(system.fixture_type_problem("Ghost"), None);
        let ghost = LightingSystem::expand_mode("Ghost", &system.referential["Ghost"], "x", base)
            .unwrap_err()
            .to_string();
        assert!(ghost.contains("missing.gdtf"), "{ghost}");
        let nope = system.fixture_type_problem("Nope").expect("nope is absent");
        assert!(nope.contains("no fixture type named 'Nope'"), "{nope}");
        assert!(nope.contains("broken.light"), "{nope}");
    }

    #[test]
    fn referential_archive_paths_cannot_escape_the_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("project");
        let ft_dir = base.join("lighting/fixture_types");
        std::fs::create_dir_all(&ft_dir).expect("mkdir");
        // A real file outside the project the reference tries to reach.
        std::fs::write(dir.path().join("outside.gdtf"), b"whatever").expect("write");
        std::fs::write(
            ft_dir.join("evil.fixture"),
            "fixture_type \"Evil\"\n  from gdtf(\"../outside.gdtf\")\n{ }\n",
        )
        .expect("write");

        let mut system = LightingSystem::new();
        system.load_fixture_types_directory(&ft_dir).expect("loads");
        assert!(!system.fixture_types.contains_key("Evil"));
    }

    #[test]
    fn test_tag_based_group_resolution() {
        let mut system = LightingSystem::new();

        // Create a venue with tagged fixtures
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string(), "rgb".to_string()],
            ),
        );
        fixtures.insert(
            "Wash2".to_string(),
            Fixture::new(
                "Wash2".to_string(),
                "RGBW_Par".to_string(),
                1,
                7,
                vec!["wash".to_string(), "front".to_string(), "rgb".to_string()],
            ),
        );
        fixtures.insert(
            "Mover1".to_string(),
            Fixture::new(
                "Mover1".to_string(),
                "MovingHead".to_string(),
                1,
                101,
                vec!["moving_head".to_string(), "spot".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define logical groups
        let front_wash_group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(2),
            ],
        );

        let movers_group = LogicalGroup::new(
            "movers".to_string(),
            vec![
                GroupConstraint::AnyOf(vec!["moving_head".to_string()]),
                GroupConstraint::MinCount(1),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), front_wash_group);
        system
            .logical_groups
            .insert("movers".to_string(), movers_group);

        // Test resolution
        let front_wash_fixtures = system.resolve_logical_group("front_wash").unwrap();
        assert_eq!(front_wash_fixtures.len(), 2);
        assert!(front_wash_fixtures.contains(&"Wash1".to_string()));
        assert!(front_wash_fixtures.contains(&"Wash2".to_string()));

        let movers_fixtures = system.resolve_logical_group("movers").unwrap();
        assert_eq!(movers_fixtures.len(), 1);
        assert!(movers_fixtures.contains(&"Mover1".to_string()));
    }

    #[test]
    fn test_group_resolution_insufficient_fixtures() {
        let mut system = LightingSystem::new();

        // Create a venue with only one wash fixture
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define a group that requires 3 fixtures
        let group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(3),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), group);

        // Test that resolution fails
        let result = system.resolve_logical_group("front_wash");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Not enough fixtures found"));
    }

    #[test]
    fn test_prefer_constraint() {
        let mut system = LightingSystem::new();

        // Create fixtures with different tag combinations
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );
        fixtures.insert(
            "Wash2".to_string(),
            Fixture::new(
                "Wash2".to_string(),
                "RGBW_Par".to_string(),
                1,
                7,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "premium".to_string(),
                ],
            ),
        );
        fixtures.insert(
            "Wash3".to_string(),
            Fixture::new(
                "Wash3".to_string(),
                "RGBW_Par".to_string(),
                1,
                13,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "premium".to_string(),
                    "rgb".to_string(),
                ],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group with preference for premium fixtures
        let group = LogicalGroup::new(
            "premium_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::Prefer(vec!["premium".to_string()]),
                GroupConstraint::MinCount(2),
                GroupConstraint::MaxCount(2),
            ],
        );

        system
            .logical_groups
            .insert("premium_wash".to_string(), group);

        // Test resolution - should prefer fixtures with "premium" tag
        let fixtures = system.resolve_logical_group("premium_wash").unwrap();
        assert_eq!(fixtures.len(), 2);
        // Should include the two premium fixtures (Wash2 and Wash3) first due to preference
        assert!(fixtures.contains(&"Wash2".to_string()));
        assert!(fixtures.contains(&"Wash3".to_string()));
        // Should not include the non-premium fixture (Wash1) since we only take 2
        assert!(!fixtures.contains(&"Wash1".to_string()));
    }

    #[test]
    fn test_max_count_constraint() {
        let mut system = LightingSystem::new();

        // Create multiple fixtures that match the criteria
        let mut fixtures = HashMap::new();
        for i in 1..=5 {
            fixtures.insert(
                format!("Wash{}", i),
                Fixture::new(
                    format!("Wash{}", i),
                    "RGBW_Par".to_string(),
                    1,
                    (i * 6) as u16,
                    vec!["wash".to_string(), "front".to_string()],
                ),
            );
        }

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group with max count constraint
        let group = LogicalGroup::new(
            "limited_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MaxCount(3),
            ],
        );

        system
            .logical_groups
            .insert("limited_wash".to_string(), group);

        // Test resolution - should limit to 3 fixtures
        let fixtures = system.resolve_logical_group("limited_wash").unwrap();
        assert_eq!(fixtures.len(), 3);
    }

    #[test]
    fn test_any_of_constraint() {
        let mut system = LightingSystem::new();

        // Create fixtures with different tag combinations
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string()],
            ),
        );
        fixtures.insert(
            "Spot1".to_string(),
            Fixture::new(
                "Spot1".to_string(),
                "MovingHead".to_string(),
                1,
                7,
                vec!["spot".to_string()],
            ),
        );
        fixtures.insert(
            "Beam1".to_string(),
            Fixture::new(
                "Beam1".to_string(),
                "Beam".to_string(),
                1,
                13,
                vec!["beam".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group that accepts any of multiple tag types
        let group = LogicalGroup::new(
            "any_light".to_string(),
            vec![
                GroupConstraint::AnyOf(vec![
                    "wash".to_string(),
                    "spot".to_string(),
                    "beam".to_string(),
                ]),
                GroupConstraint::MinCount(2),
            ],
        );

        system.logical_groups.insert("any_light".to_string(), group);

        // Test resolution - should include fixtures with any of the specified tags
        let fixtures = system.resolve_logical_group("any_light").unwrap();
        assert_eq!(fixtures.len(), 3);
        assert!(fixtures.contains(&"Wash1".to_string()));
        assert!(fixtures.contains(&"Spot1".to_string()));
        assert!(fixtures.contains(&"Beam1".to_string()));
    }

    #[test]
    fn test_complex_constraint_combination() {
        let mut system = LightingSystem::new();

        // Create fixtures with various tag combinations
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string(), "rgb".to_string()],
            ),
        );
        fixtures.insert(
            "Wash2".to_string(),
            Fixture::new(
                "Wash2".to_string(),
                "RGBW_Par".to_string(),
                1,
                7,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "rgb".to_string(),
                    "premium".to_string(),
                ],
            ),
        );
        fixtures.insert(
            "Wash3".to_string(),
            Fixture::new(
                "Wash3".to_string(),
                "RGBW_Par".to_string(),
                1,
                13,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "rgb".to_string(),
                    "premium".to_string(),
                ],
            ),
        );
        fixtures.insert(
            "Wash4".to_string(),
            Fixture::new(
                "Wash4".to_string(),
                "RGBW_Par".to_string(),
                1,
                19,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define complex group with multiple constraints
        let group = LogicalGroup::new(
            "complex_group".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::Prefer(vec!["premium".to_string()]),
                GroupConstraint::MinCount(2),
                GroupConstraint::MaxCount(3),
            ],
        );

        system
            .logical_groups
            .insert("complex_group".to_string(), group);

        // Test resolution - should prefer premium fixtures but limit to 3
        let fixtures = system.resolve_logical_group("complex_group").unwrap();
        assert_eq!(fixtures.len(), 3);
        // Should include the premium fixtures first
        assert!(fixtures.contains(&"Wash2".to_string()));
        assert!(fixtures.contains(&"Wash3".to_string()));
        // Should include one non-premium fixture
        assert!(fixtures.contains(&"Wash1".to_string()) || fixtures.contains(&"Wash4".to_string()));
    }

    #[test]
    fn test_group_resolution_no_current_venue() {
        let mut system = LightingSystem::new();

        // Don't set current venue
        system.current_venue = None;

        let group = LogicalGroup::new("test_group".to_string(), vec![GroupConstraint::MinCount(1)]);
        system
            .logical_groups
            .insert("test_group".to_string(), group);

        // Test that resolution fails without current venue
        let result = system.resolve_logical_group("test_group");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No current venue selected"));
    }

    #[test]
    fn test_group_resolution_nonexistent_group() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Try to resolve a group that doesn't exist
        let result = system.resolve_logical_group("nonexistent_group");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Logical group 'nonexistent_group' not found"));
    }

    #[test]
    fn test_group_resolution_nonexistent_venue() {
        let mut system = LightingSystem::new();

        // Set current venue to one that doesn't exist
        system.current_venue = Some("Nonexistent Venue".to_string());

        let group = LogicalGroup::new("test_group".to_string(), vec![GroupConstraint::MinCount(1)]);
        system
            .logical_groups
            .insert("test_group".to_string(), group);

        // Test that resolution fails with nonexistent venue
        let result = system.resolve_logical_group("test_group");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Venue 'Nonexistent Venue' not found"));
    }

    #[test]
    fn test_group_caching() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        let group = LogicalGroup::new(
            "cached_group".to_string(),
            vec![GroupConstraint::MinCount(1)],
        );
        system
            .logical_groups
            .insert("cached_group".to_string(), group);

        // First resolution
        let fixtures1 = system.resolve_logical_group("cached_group").unwrap();
        assert_eq!(fixtures1.len(), 1);
        assert!(fixtures1.contains(&"Wash1".to_string()));

        // Second resolution should use cache
        let fixtures2 = system.resolve_logical_group("cached_group").unwrap();
        assert_eq!(fixtures2.len(), 1);
        assert!(fixtures2.contains(&"Wash1".to_string()));

        // Verify cache was populated
        assert!(system.group_cache.contains_key("Test Venue"));
        assert!(system
            .group_cache
            .get("Test Venue")
            .unwrap()
            .contains_key("cached_group"));
    }

    #[test]
    fn test_graceful_fallback_missing_group() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Try to resolve a group that doesn't exist - should return empty vector
        let fixtures = system.resolve_logical_group_graceful("nonexistent_group");
        assert_eq!(fixtures.len(), 0);
    }

    #[test]
    fn test_graceful_fallback_insufficient_fixtures() {
        let mut system = LightingSystem::new();

        // Create venue with only one wash fixture
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group that requires 3 fixtures but only 1 available
        let group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(3),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), group);

        // Test graceful fallback - should return empty vector
        let fixtures = system.resolve_logical_group_graceful("front_wash");
        assert_eq!(fixtures.len(), 0);
    }

    #[test]
    fn test_allow_empty_constraint() {
        let mut system = LightingSystem::new();

        // Create venue with no matching fixtures
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Mover1".to_string(),
            Fixture::new(
                "Mover1".to_string(),
                "MovingHead".to_string(),
                1,
                1,
                vec!["moving_head".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group that requires wash fixtures but none exist, but allows empty
        let group = LogicalGroup::new(
            "wash_lights".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string()]),
                GroupConstraint::AllowEmpty(true),
            ],
        );

        system
            .logical_groups
            .insert("wash_lights".to_string(), group);

        // Test that group resolves to empty list when no fixtures match
        let fixtures = system.resolve_logical_group("wash_lights").unwrap();
        assert_eq!(fixtures.len(), 0);
    }

    #[test]
    fn test_multiple_groups_graceful() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define one group that exists and one that doesn't
        let front_wash_group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(1),
            ],
        );
        let movers_group = LogicalGroup::new(
            "movers".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["moving_head".to_string()]),
                GroupConstraint::MinCount(1),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), front_wash_group);
        system
            .logical_groups
            .insert("movers".to_string(), movers_group);

        // Test multiple group resolution
        let _group_names = ["front_wash".to_string(), "movers".to_string()];
        let results = system.resolve_logical_group_graceful("front_wash");

        // front_wash should have fixtures
        assert_eq!(results.len(), 1);
        assert!(results.contains(&"Wash1".to_string()));
    }

    // ── FallbackTo constraint ────────────────────────────────────────

    #[test]
    fn test_fallback_to_constraint() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures);
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define a group that requires movers (which don't exist), with FallbackTo wash
        let primary_group = LogicalGroup::new(
            "movers".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["moving_head".to_string()]),
                GroupConstraint::MinCount(1),
                GroupConstraint::FallbackTo("front_wash".to_string()),
            ],
        );

        // Define the fallback group
        let fallback_group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![GroupConstraint::AllOf(vec![
                "wash".to_string(),
                "front".to_string(),
            ])],
        );

        system
            .logical_groups
            .insert("movers".to_string(), primary_group);
        system
            .logical_groups
            .insert("front_wash".to_string(), fallback_group);

        // Movers should fall back to front_wash
        let results = system.resolve_logical_group_graceful("movers");
        assert_eq!(
            results.len(),
            1,
            "FallbackTo should resolve to fallback group"
        );
        assert!(
            results.contains(&"Wash1".to_string()),
            "FallbackTo should contain Wash1"
        );
    }

    // ── get_group ────────────────────────────────────────────────────

    #[test]
    fn test_get_current_venue_fixtures_no_venue() {
        let system = LightingSystem::new();
        let result = system.get_current_venue_fixtures();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No current venue"));
    }

    #[test]
    fn test_get_current_venue_fixtures_with_fixtures() {
        let mut system = LightingSystem::new();

        // Register a fixture type
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        let ft = super::super::types::FixtureType::new("Par".to_string(), channels);
        system.fixture_types.insert("Par".to_string(), ft);

        // Create venue with a fixture
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "front1".to_string(),
            super::super::types::Fixture::new(
                "front1".to_string(),
                "Par".to_string(),
                1,
                10,
                vec!["front".to_string()],
            ),
        );
        let venue = super::super::types::Venue::new("TestVenue".to_string(), fixtures);
        system.venues.insert("TestVenue".to_string(), venue);
        system.current_venue = Some("TestVenue".to_string());

        let infos = system.get_current_venue_fixtures().unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "front1");
        assert_eq!(infos[0].universe, 1);
        assert_eq!(infos[0].address, 10);
    }

    #[test]
    fn test_get_current_venue_fixtures_unknown_type() {
        let mut system = LightingSystem::new();

        // Create venue with a fixture whose type isn't registered
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "broken".to_string(),
            super::super::types::Fixture::new(
                "broken".to_string(),
                "UnknownType".to_string(),
                1,
                1,
                vec![],
            ),
        );
        let venue = super::super::types::Venue::new("TestVenue".to_string(), fixtures);
        system.venues.insert("TestVenue".to_string(), venue);
        system.current_venue = Some("TestVenue".to_string());

        let result = system.get_current_venue_fixtures();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Fixture type 'UnknownType' not found"));
    }

    #[test]
    fn an_mvr_seeded_venue_gets_its_scenery_distilled_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let gdtf = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )]);
        let mvr = crate::lighting::gdtf::build_zip(&[
            (
                "GeneralSceneDescription.xml",
                crate::lighting::mvr::SYNTHETIC_SCENE.as_bytes(),
            ),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
            ("Lid.glb", b"lid".as_slice()),
            ("deck.glb", b"deck".as_slice()),
        ]);
        crate::lighting::import::import_mvr_bytes(
            &mvr,
            "kellys.mvr",
            &crate::lighting::import::MvrImportOptions {
                name: Some("kellys".to_string()),
                ..Default::default()
            },
            &project,
        )
        .unwrap();

        let config = Lighting::new(
            Some("kellys".to_string()),
            None,
            None,
            Some(crate::config::lighting::Directories::new(
                Some("lighting/fixture_types".to_string()),
                Some("lighting/venues".to_string()),
            )),
        );
        let mut system = LightingSystem::new();
        system.load(&config, &project).unwrap();
        let rel = system
            .scenery("kellys")
            .expect("scenery distilled")
            .to_string();
        assert!(project.join("lighting/.cache/assets").join(&rel).is_file());
        let cache = DistillCache::new(project.join("lighting/.cache"));
        let model = cache.scenery(&rel).unwrap();
        assert_eq!(model.objects.len(), 2);
        assert_eq!(model.formats.get("glb"), Some(&2));

        // A reload keeps it without re-reading the archive (the stamp
        // matches); a hand-written venue has none; a venue whose MVR is
        // gone says why.
        system.reload_venues().unwrap();
        assert_eq!(system.scenery("kellys"), Some(rel.as_str()));
        assert!(system.scenery("nowhere").is_none());
        assert!(system.scenery_error("kellys").is_none());
        std::fs::remove_file(project.join("lighting/library/kellys.mvr")).unwrap();
        system.reload_venues().unwrap();
        assert!(system.scenery("kellys").is_none());
        assert!(
            system
                .scenery_error("kellys")
                .is_some_and(|e| e.contains("cannot read MVR")),
            "{:?}",
            system.scenery_error("kellys")
        );
    }
}
