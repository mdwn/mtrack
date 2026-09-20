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

//! The fixture distillation cache.
//!
//! A referential fixture type (`from gdtf(...)`) is expanded — GDTF archive +
//! mode distilled into a full [`FixtureType`] — at import or prewarm time,
//! never at show time. The expansion lives here, keyed by everything that can
//! change its result: the archive bytes, the mode, the distiller version, and
//! the override fingerprint. A changed archive or an upgraded distiller
//! regenerates on the next fill; nothing regenerates silently mid-show.
//!
//! The cache directory is per-project (`lighting/.cache/`), gitignored, and
//! rebuildable from the committed GDTF archives.
//!
//! Beside the expansions sits the asset store (`lighting/.cache/assets/`,
//! design §16.2): per archive, content-addressed by its bytes, the meshes
//! and thumbnail copied out of it and one rig model per mode. The web UI
//! serves the store to the 3D view; nothing at show time reads it.

use std::error::Error;
use std::path::{Path, PathBuf};

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::gdtf::{self, Description, RigModel, RIG_VERSION};
use super::mvr::{self, Matrix, Scene};
use super::types::{Cell, ChannelDef, FixtureType, GdtfSource, MovementLimits};

/// The asset store's directory under the cache.
pub const ASSETS_DIR: &str = "assets";

/// Bumped whenever the scenery written for the same MVR can change.
pub const SCENERY_VERSION: u32 = 1;

/// The most bytes of meshes copied out of one MVR.
const MAX_SCENERY_BYTES_TOTAL: u64 = 256 * 1024 * 1024;

/// A venue's scenery as the store writes it (design §16.3): every scene
/// object with its transform in stage space and the meshes it draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneryModel {
    pub version: u32,
    /// The objects, in document order.
    pub objects: Vec<SceneryObject>,
    /// Mesh files by extension: how much of the scenery each format holds,
    /// drawn or not.
    pub formats: BTreeMap<String, usize>,
    /// What was skipped and why.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One piece of scenery.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneryObject {
    pub name: String,
    /// `SceneObject`, `Truss`, `Support`, ...
    pub kind: String,
    pub layer: String,
    /// Row-major 4×4 in stage space: the MVR basis (scale and all) with
    /// the translation in meters, re-origined like the fixtures.
    pub transform: [[f64; 4]; 4],
    /// Meshes the store holds, by path relative to the scenery file.
    pub meshes: Vec<SceneryMesh>,
    /// Mesh files the 3D view cannot draw (`.3ds` and the like), by name.
    pub skipped: Vec<String>,
}

/// A drawable mesh of an object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneryMesh {
    pub file: String,
    /// Row-major 4×4 relative to the object, translation in meters.
    pub transform: [[f64; 4]; 4],
}

/// A name reduced to its lowercase ASCII letters and digits — what two
/// spellings of one non-ASCII name agree on.
fn ascii_skeleton(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// An MVR transform as a row-major 4×4 with the translation converted from
/// millimeters, re-origined by `origin_mm`.
fn scenery_transform(matrix: &Matrix, origin_mm: &[f64; 3]) -> [[f64; 4]; 4] {
    [
        [
            matrix.u[0],
            matrix.v[0],
            matrix.w[0],
            (matrix.o[0] - origin_mm[0]) / 1000.0,
        ],
        [
            matrix.u[1],
            matrix.v[1],
            matrix.w[1],
            (matrix.o[1] - origin_mm[1]) / 1000.0,
        ],
        [
            matrix.u[2],
            matrix.v[2],
            matrix.w[2],
            (matrix.o[2] - origin_mm[2]) / 1000.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// A mesh entry name as a file name in the store: the archive's own name
/// reduced to safe characters, made unique by a hash when two reduce alike.
fn mesh_file_name(entry: &str, taken: &mut std::collections::HashSet<String>) -> String {
    let base = entry.rsplit('/').next().unwrap_or(entry);
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) => (s, e.to_ascii_lowercase()),
        None => (base, String::new()),
    };
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.is_empty() {
        "mesh".to_string()
    } else {
        safe
    };
    let mut name = format!("{safe}.{ext}");
    if !taken.insert(name.clone()) {
        let hash = format!("{:x}", Sha256::digest(entry.as_bytes()));
        name = format!("{safe}-{}.{ext}", &hash[..8]);
        taken.insert(name.clone());
    }
    name
}

/// Bumped whenever the distiller's output for the same source can change.
/// Part of the cache key, so an upgrade regenerates every expansion.
pub const DISTILLER_VERSION: u32 = 4;

/// The cache's on-disk representation of a distilled fixture type.
///
/// The cache is machine-only, so it owns its format outright: this struct —
/// not [`FixtureType`]'s public serialization, which deliberately omits the
/// structured fields — is what entries round-trip through. Deserialized
/// entries rebuild through the same normalizing constructor as everything
/// else, so a cache read can't produce a de-normalized fixture type.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    name: String,
    channel_defs: HashMap<String, ChannelDef>,
    max_strobe_frequency: Option<f64>,
    min_strobe_frequency: Option<f64>,
    strobe_dmx_offset: Option<u8>,
    source: Option<GdtfSource>,
    movement: MovementLimits,
    #[serde(default)]
    cells: Vec<Cell>,
}

impl From<&FixtureType> for CacheEntry {
    fn from(fixture_type: &FixtureType) -> CacheEntry {
        CacheEntry {
            name: fixture_type.name().to_string(),
            channel_defs: fixture_type.channel_defs().clone(),
            max_strobe_frequency: fixture_type.max_strobe_frequency(),
            min_strobe_frequency: fixture_type.min_strobe_frequency(),
            strobe_dmx_offset: fixture_type.strobe_dmx_offset(),
            source: fixture_type.source().cloned(),
            movement: *fixture_type.movement(),
            cells: fixture_type.cells().to_vec(),
        }
    }
}

impl From<CacheEntry> for FixtureType {
    fn from(entry: CacheEntry) -> FixtureType {
        let mut fixture_type = FixtureType::from_parts(
            entry.name,
            entry.channel_defs,
            entry.max_strobe_frequency,
            entry.min_strobe_frequency,
            entry.strobe_dmx_offset,
        );
        if let Some(source) = entry.source {
            fixture_type.set_source(source);
        }
        fixture_type.set_movement(entry.movement);
        fixture_type.set_cells(entry.cells);
        fixture_type
    }
}

/// A content-addressed store of distilled fixture types.
pub struct DistillCache {
    dir: PathBuf,
}

impl DistillCache {
    /// Creates a cache over the given directory. The directory is created
    /// lazily on the first write.
    pub fn new(dir: PathBuf) -> DistillCache {
        DistillCache { dir }
    }

    /// The directory this cache stores expansions in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Computes the cache key for an expansion. Everything that can change
    /// the expanded result participates: the referring fixture type's name
    /// (the expansion carries it — two refs to the same GDTF+mode must not
    /// share an entry), archive bytes, mode, distiller version, and the
    /// overrides fingerprint (the textual form of the referential fixture's
    /// own body).
    pub fn key(
        fixture_name: &str,
        archive_bytes: &[u8],
        mode: &str,
        overrides_fingerprint: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(fixture_name.as_bytes());
        hasher.update([0u8]);
        hasher.update(archive_bytes);
        hasher.update([0u8]);
        hasher.update(mode.as_bytes());
        hasher.update([0u8]);
        hasher.update(DISTILLER_VERSION.to_le_bytes());
        hasher.update([0u8]);
        hasher.update(overrides_fingerprint.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// Reads a cached expansion. A missing or unreadable entry is a miss —
    /// corrupt cache files are deleted and refilled, never trusted.
    pub fn get(&self, key: &str) -> Option<FixtureType> {
        let path = self.entry_path(key);
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                // A miss the fill path will repair — but an unreadable
                // entry (permissions, IO) is not a normal miss, so say so.
                warn!(file = %path.display(), error = %e, "Failed to read expansion cache entry");
                return None;
            }
        };
        match serde_json::from_str::<CacheEntry>(&content) {
            Ok(entry) => Some(entry.into()),
            Err(e) => {
                warn!(file = %path.display(), error = %e, "Discarding corrupt expansion cache entry");
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Writes an expansion. The write is atomic (unique temp file + rename),
    /// so a crash mid-write can't leave a truncated entry behind and two
    /// concurrent fills of the same key (a parallel prewarm) can't tear each
    /// other's writes — last rename wins with identical content.
    pub fn put(&self, key: &str, fixture_type: &FixtureType) -> Result<(), Box<dyn Error>> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.entry_path(key);
        let mut tmp = tempfile::NamedTempFile::new_in(&self.dir)?;
        let entry = CacheEntry::from(fixture_type);
        std::io::Write::write_all(&mut tmp, serde_json::to_string_pretty(&entry)?.as_bytes())?;
        tmp.persist(&path)?;
        Ok(())
    }

    /// The asset store's directory.
    pub fn assets_dir(&self) -> PathBuf {
        self.dir.join(ASSETS_DIR)
    }

    /// The store-relative path of the rig file for an archive and mode:
    /// Reads a rig the cache holds, by the path [`Self::ensure_rig`] gave.
    pub fn load_rig(&self, rel: &str) -> Result<gdtf::RigModel, Box<dyn Error>> {
        let bytes = std::fs::read(self.assets_dir().join(rel))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// `<archive sha256>/rig-<mode sha256 prefix>-v<RIG_VERSION>.json`.
    /// The archive hash keys the directory (its meshes are the archive's,
    /// whatever mode is in use); the mode string as the `.fixture` pins it
    /// and the rig version key the file — two spellings of one mode that
    /// [`gdtf::match_mode`] folds together get two identical rig files,
    /// which is cheap and keeps the path computable without a parse.
    pub fn rig_path(archive_bytes: &[u8], mode: &str) -> String {
        let archive = format!("{:x}", Sha256::digest(archive_bytes));
        let mode = format!("{:x}", Sha256::digest(mode.as_bytes()));
        format!("{archive}/rig-{}-v{RIG_VERSION}.json", &mode[..16])
    }

    /// Makes sure the store holds the rig model for an archive and mode,
    /// with the meshes and thumbnail it names, and returns the rig's
    /// store-relative path with the distillation's warnings. A present rig
    /// file is trusted (the path is content-addressed) and has no warnings
    /// to repeat; otherwise `describe` parses the archive once and
    /// everything is written, meshes first, the rig last.
    pub fn ensure_rig<'a>(
        &self,
        archive_bytes: &[u8],
        mode: &str,
        describe: impl FnOnce() -> Result<&'a Description, Box<dyn Error>>,
    ) -> Result<(String, Vec<String>), Box<dyn Error>> {
        let rel = Self::rig_path(archive_bytes, mode);
        let rig_file = self.assets_dir().join(&rel);
        if rig_file.is_file() {
            return Ok((rel, Vec::new()));
        }
        let description = describe()?;
        let available = gdtf::list_model_files(archive_bytes)?;
        let mut rig = gdtf::distill_rig(description, mode, &available)?;

        // Only the meshes the rig draws are copied out.
        let wanted: std::collections::HashSet<String> = rig
            .nodes
            .iter()
            .filter_map(|n| match &n.shape {
                gdtf::RigShape::Model { file } => file
                    .strip_prefix("models/")
                    .and_then(|f| f.strip_suffix(".glb"))
                    .map(str::to_string),
                _ => None,
            })
            .collect();
        let assets = gdtf::read_assets(archive_bytes, &wanted, description.thumbnail.as_deref())?;

        let dir = rig_file.parent().expect("rig path has a directory");
        let models_dir = dir.join("models");
        std::fs::create_dir_all(&models_dir)?;
        for (stem, bytes) in &assets.models {
            write_atomic(&models_dir.join(format!("{stem}.glb")), bytes)?;
        }
        if let Some((ext, bytes)) = &assets.thumbnail {
            let name = format!("thumbnail.{ext}");
            write_atomic(&dir.join(&name), bytes)?;
            rig.thumbnail = Some(name);
        }
        write_atomic(&rig_file, serde_json::to_string_pretty(&rig)?.as_bytes())?;
        Ok((rel, rig.warnings))
    }

    /// The store-relative path of a venue's scenery file:
    /// `scenery/<mvr sha256>/scene-<origin hash>-v<SCENERY_VERSION>.json`.
    /// The archive hash keys the directory (its meshes); the stage origin,
    /// which the transforms bake in, keys the file.
    pub fn scenery_path(mvr_bytes: &[u8], origin_m: &[f64; 3]) -> String {
        let archive = format!("{:x}", Sha256::digest(mvr_bytes));
        let origin = format!("{:x}", Sha256::digest(format!("{:?}", origin_m).as_bytes()));
        format!(
            "scenery/{archive}/scene-{}-v{SCENERY_VERSION}.json",
            &origin[..16]
        )
    }

    /// Makes sure the store holds a venue's scenery — the scene file and
    /// every glTF mesh it draws — and returns the file's store-relative
    /// path with the distillation's warnings. Meshes in formats the view
    /// cannot draw (`.3ds`) are listed as skipped, not copied.
    pub fn ensure_scenery<'a>(
        &self,
        mvr_bytes: &[u8],
        origin_m: &[f64; 3],
        describe: impl FnOnce() -> Result<&'a Scene, Box<dyn Error>>,
    ) -> Result<(String, Vec<String>), Box<dyn Error>> {
        let rel = Self::scenery_path(mvr_bytes, origin_m);
        let scene_file = self.assets_dir().join(&rel);
        if scene_file.is_file() {
            return Ok((rel, Vec::new()));
        }
        let scene = describe()?;
        let origin_mm = [
            origin_m[0] * 1000.0,
            origin_m[1] * 1000.0,
            origin_m[2] * 1000.0,
        ];
        let entries: std::collections::HashSet<String> =
            mvr::list_entries(mvr_bytes)?.into_iter().collect();

        let mut model = SceneryModel {
            version: SCENERY_VERSION,
            objects: Vec::new(),
            formats: BTreeMap::new(),
            warnings: Vec::new(),
        };
        // Entry name → store file name, so a mesh many objects share is
        // copied once.
        let mut file_of: HashMap<String, String> = HashMap::new();
        let mut taken = std::collections::HashSet::new();
        let mut missing = std::collections::BTreeSet::new();
        let mut ambiguous = std::collections::BTreeSet::new();
        for object in &scene.objects {
            let Some(matrix) = object.matrix else {
                continue;
            };
            let mut out = SceneryObject {
                name: object.name.clone(),
                kind: object.kind.clone(),
                layer: object.layer.clone(),
                transform: scenery_transform(&matrix, &origin_mm),
                meshes: Vec::new(),
                skipped: Vec::new(),
            };
            for mesh in &object.meshes {
                let ext = mesh
                    .file
                    .rsplit_once('.')
                    .map(|(_, e)| e.to_ascii_lowercase())
                    .unwrap_or_default();
                *model.formats.entry(ext.clone()).or_default() += 1;
                if ext != "glb" {
                    out.skipped.push(mesh.file.clone());
                    continue;
                }
                // Entry names are matched as written, then case-folded,
                // then by ASCII skeleton: an archive that stores UTF-8
                // names without the UTF-8 flag reads back as cp437
                // ("B├╝hnenpodest"), and the scene spells it "Bühnenpodest".
                let entry = if entries.contains(&mesh.file) {
                    Some(mesh.file.clone())
                } else {
                    entries
                        .iter()
                        .find(|e| e.eq_ignore_ascii_case(&mesh.file))
                        .or_else(|| {
                            let wanted = ascii_skeleton(&mesh.file);
                            let mut candidates =
                                entries.iter().filter(|e| ascii_skeleton(e) == wanted);
                            match (candidates.next(), candidates.next()) {
                                (Some(one), None) => Some(one),
                                (Some(_), Some(_)) => {
                                    ambiguous.insert(mesh.file.clone());
                                    None
                                }
                                _ => None,
                            }
                        })
                        .cloned()
                };
                let Some(entry) = entry else {
                    if !ambiguous.contains(&mesh.file) {
                        missing.insert(mesh.file.clone());
                    }
                    out.skipped.push(mesh.file.clone());
                    continue;
                };
                let file = match file_of.get(&entry) {
                    Some(file) => file.clone(),
                    None => {
                        let file = format!("models/{}", mesh_file_name(&entry, &mut taken));
                        file_of.insert(entry.clone(), file.clone());
                        file
                    }
                };
                out.meshes.push(SceneryMesh {
                    file,
                    transform: scenery_transform(&mesh.matrix.unwrap_or(mvr::IDENTITY), &[0.0; 3]),
                });
            }
            model.objects.push(out);
        }
        for file in missing {
            model.warnings.push(format!(
                "scenery mesh {file} is referenced but not in the archive"
            ));
        }
        for file in ambiguous {
            model.warnings.push(format!(
                "scenery mesh {file} matches more than one archive entry by name; skipped"
            ));
        }
        let undrawn: usize = model
            .formats
            .iter()
            .filter(|(ext, _)| ext.as_str() != "glb")
            .map(|(_, n)| n)
            .sum();
        if undrawn > 0 {
            model.warnings.push(format!(
                "{undrawn} scenery mesh(es) are in formats the 3D view does not draw ({}); glTF (.glb) is drawn",
                model
                    .formats
                    .iter()
                    .filter(|(ext, _)| ext.as_str() != "glb")
                    .map(|(ext, n)| format!("{n} .{ext}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // Meshes first, the scene file last, under a total budget. A mesh
        // that cannot be read (over its cap, corrupt) or that would take
        // the store past the budget is skipped, and the objects that use
        // it say so; the rest of the scenery is kept.
        let dir = scene_file.parent().expect("scene path has a directory");
        let models_dir = dir.join("models");
        std::fs::create_dir_all(&models_dir)?;
        let mut total: u64 = 0;
        let mut dropped: HashMap<String, String> = HashMap::new();
        let mut ordered: Vec<(&String, &String)> = file_of.iter().collect();
        ordered.sort();
        for (entry, file) in ordered {
            let bytes = match mvr::read_mesh_entry(mvr_bytes, entry) {
                Ok(bytes) => bytes,
                Err(e) => {
                    dropped.insert(file.clone(), format!("{entry}: {e}"));
                    continue;
                }
            };
            if total + bytes.len() as u64 > MAX_SCENERY_BYTES_TOTAL {
                dropped.insert(
                    file.clone(),
                    format!("{entry}: past the {MAX_SCENERY_BYTES_TOTAL}-byte scenery budget"),
                );
                continue;
            }
            total += bytes.len() as u64;
            write_atomic(&dir.join(file), &bytes)?;
        }
        if !dropped.is_empty() {
            for object in &mut model.objects {
                let (kept, lost): (Vec<SceneryMesh>, Vec<SceneryMesh>) = object
                    .meshes
                    .drain(..)
                    .partition(|m| !dropped.contains_key(&m.file));
                object.meshes = kept;
                object.skipped.extend(lost.into_iter().map(|m| m.file));
            }
            let mut reasons: Vec<&String> = dropped.values().collect();
            reasons.sort();
            for reason in reasons {
                model
                    .warnings
                    .push(format!("scenery mesh not stored: {reason}"));
            }
        }
        write_atomic(
            &scene_file,
            serde_json::to_string_pretty(&model)?.as_bytes(),
        )?;
        Ok((rel, model.warnings))
    }

    /// Reads a scenery model back from the store by its store-relative path.
    pub fn scenery(&self, rel: &str) -> Option<SceneryModel> {
        let content = std::fs::read_to_string(self.assets_dir().join(rel)).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Reads a rig model back from the store by its store-relative path.
    pub fn rig(&self, rel: &str) -> Option<RigModel> {
        let content = std::fs::read_to_string(self.assets_dir().join(rel)).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Returns the cached expansion for `key`, filling it via `fill` on a
    /// miss. The fill path is the only place untrusted source data is parsed.
    pub fn get_or_fill(
        &self,
        key: &str,
        fill: impl FnOnce() -> Result<FixtureType, Box<dyn Error>>,
    ) -> Result<FixtureType, Box<dyn Error>> {
        if let Some(cached) = self.get(key) {
            return Ok(cached);
        }
        let fixture_type = fill()?;
        self.put(key, &fixture_type)?;
        Ok(fixture_type)
    }
}

/// Writes a file atomically (unique temp file + rename) so a crash or a
/// concurrent fill can't leave a torn file behind; identical content makes
/// last-rename-wins harmless. Skipped when the file already exists, since
/// the store is content-addressed.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if path.is_file() {
        return Ok(());
    }
    let dir = path.parent().expect("file path has a directory");
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, bytes)?;
    tmp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn sample_fixture_type() -> FixtureType {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("strobe".to_string(), 4);
        crate::lighting::types::FixtureTypeV1 {
            name: "Brick".to_string(),
            channels,
            max_strobe_frequency: Some(25.0),
            min_strobe_frequency: Some(0.4),
            strobe_dmx_offset: Some(7),
        }
        .into()
    }

    #[test]
    fn key_changes_with_each_input() {
        let base = DistillCache::key("Brick", b"archive", "Mode 1", "");
        assert_ne!(base, DistillCache::key("Brick2", b"archive", "Mode 1", ""));
        assert_ne!(base, DistillCache::key("Brick", b"archive2", "Mode 1", ""));
        assert_ne!(base, DistillCache::key("Brick", b"archive", "Mode 2", ""));
        assert_ne!(
            base,
            DistillCache::key("Brick", b"archive", "Mode 1", "overrides")
        );
        // Same inputs, same key.
        assert_eq!(base, DistillCache::key("Brick", b"archive", "Mode 1", ""));
    }

    #[test]
    fn key_separates_fields() {
        // The separator prevents boundary ambiguity: ("ab", "c") != ("a", "bc").
        assert_ne!(
            DistillCache::key("n", b"ab", "c", ""),
            DistillCache::key("n", b"a", "bc", "")
        );
    }

    #[test]
    fn miss_then_fill_then_hit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join("cache"));
        let key = DistillCache::key("Brick", b"archive", "Mode 1", "");

        assert!(cache.get(&key).is_none());

        let mut fills = 0;
        let filled = cache
            .get_or_fill(&key, || {
                fills += 1;
                Ok(sample_fixture_type())
            })
            .unwrap();
        assert_eq!(fills, 1);
        assert_eq!(filled.name(), "Brick");

        // The second call must come from the cache, not the fill closure.
        let cached = cache
            .get_or_fill(&key, || panic!("must not refill a warm cache"))
            .unwrap();
        assert_eq!(cached.name(), "Brick");
        assert_eq!(cached.channels(), filled.channels());
        assert_eq!(cached.strobe_dmx_offset(), Some(7));
        assert_eq!(cached.max_strobe_frequency(), Some(25.0));
        let strobe = cached.channel_defs().get("strobe").unwrap();
        assert_eq!(strobe.functions.len(), 1);
    }

    #[test]
    fn round_trip_preserves_every_field_class() {
        // Source, movement, fine/range channels, and *partial* strobe
        // fields must all survive put→get — the entry rebuilds through the
        // normalizing constructor, so anything it drops is lost silently.
        let mut defs = HashMap::new();
        defs.insert(
            "pan".to_string(),
            ChannelDef {
                offset: 1,
                fine: Some(2),
                range: Some(crate::lighting::types::PhysicalRange {
                    from: -270.0,
                    to: 270.0,
                    unit: crate::lighting::types::PhysicalUnit::Degrees,
                }),
                functions: Vec::new(),
                mirrors: Vec::new(),
            },
        );
        defs.insert("strobe".to_string(), ChannelDef::at(3));
        let mut original = FixtureType::from_parts(
            "Esprite".to_string(),
            defs,
            Some(20.0), // partial: only max is known
            None,
            None,
        );
        original.set_source(GdtfSource {
            path: "library/esprite.gdtf".to_string(),
            mode: "Mode 1".to_string(),
        });
        original.set_movement(MovementLimits {
            max_pan_speed: Some(240.0),
            max_tilt_speed: Some(200.0),
        });

        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().to_path_buf());
        let key = DistillCache::key("Esprite", b"archive", "Mode 1", "overrides");
        cache.put(&key, &original).unwrap();
        let restored = cache.get(&key).expect("cached entry");

        assert_eq!(restored.name(), "Esprite");
        assert_eq!(restored.channel_defs(), original.channel_defs());
        assert_eq!(restored.max_strobe_frequency(), Some(20.0));
        assert_eq!(restored.min_strobe_frequency(), None);
        assert_eq!(restored.strobe_dmx_offset(), None);
        assert_eq!(restored.source(), original.source());
        assert_eq!(restored.movement(), original.movement());
        let pan = restored.channel_defs().get("pan").unwrap();
        assert_eq!(pan.fine, Some(2));
        assert!(pan.range.is_some());
    }

    #[test]
    fn round_trip_preserves_cells() {
        let mut cell_channels = HashMap::new();
        cell_channels.insert("red".to_string(), ChannelDef::at(2));
        cell_channels.insert("green".to_string(), ChannelDef::at(3));
        let cells = vec![
            Cell {
                name: "Pixel 1".to_string(),
                channels: cell_channels.clone(),
                offset: [-0.1, 0.0, 0.0],
            },
            Cell {
                name: "Pixel 2".to_string(),
                channels: cell_channels,
                offset: [0.1, 0.0, 0.0],
            },
        ];
        let mut original = sample_fixture_type();
        original.set_cells(cells.clone());

        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().to_path_buf());
        let key = DistillCache::key("Brick", b"archive", "Mode 1", "");
        cache.put(&key, &original).unwrap();
        let restored = cache.get(&key).expect("cached entry");

        assert_eq!(restored.cells(), cells.as_slice());
    }

    #[test]
    fn a_cache_entry_without_a_cells_field_still_deserializes() {
        // Entries written before cells existed (DISTILLER_VERSION < 3) must
        // not be treated as corrupt — the field defaults to empty.
        let entry = CacheEntry::from(&sample_fixture_type());
        let mut value = serde_json::to_value(&entry).unwrap();
        value.as_object_mut().unwrap().remove("cells");
        let json = serde_json::to_string(&value).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().to_path_buf());
        let key = DistillCache::key("Brick", b"archive", "Mode 1", "");
        std::fs::create_dir_all(cache.dir()).unwrap();
        std::fs::write(cache.dir().join(format!("{key}.json")), json).unwrap();

        let restored = cache.get(&key).expect("still deserializes without cells");
        assert!(restored.cells().is_empty());
    }

    #[test]
    fn corrupt_entry_is_discarded_and_refilled() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().to_path_buf());
        let key = DistillCache::key("Brick", b"archive", "Mode 1", "");

        std::fs::write(dir.path().join(format!("{key}.json")), "not json").unwrap();
        assert!(cache.get(&key).is_none());
        // The corrupt entry is gone, so a fill lands cleanly.
        let filled = cache
            .get_or_fill(&key, || Ok(sample_fixture_type()))
            .unwrap();
        assert_eq!(filled.name(), "Brick");
        assert!(cache.get(&key).is_some());
    }

    #[test]
    fn fill_error_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join("cache"));
        let key = DistillCache::key("Brick", b"archive", "Mode 1", "");

        let result = cache.get_or_fill(&key, || Err("distiller exploded".into()));
        assert!(result.is_err());
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn the_asset_store_holds_the_rig_its_meshes_and_the_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join(".cache"));
        let archive = crate::lighting::gdtf::build_zip(&[
            (
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                    .replace(
                        "Manufacturer=\"mtrack synthetic\"",
                        "Manufacturer=\"m\" Thumbnail=\"thumbnail\"",
                    )
                    .as_bytes(),
            ),
            ("thumbnail.png", b"png".as_slice()),
            ("models/gltf/yoke.glb", b"yoke-mesh".as_slice()),
            ("models/gltf/unused.glb", b"unused".as_slice()),
        ]);
        let description = crate::lighting::gdtf::parse_archive(&archive).unwrap();
        let parses = std::cell::Cell::new(0);
        let describe = || {
            parses.set(parses.get() + 1);
            Ok(&description)
        };
        let (rel, warnings) = cache.ensure_rig(&archive, "Mover 16bit", describe).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(rel.ends_with(&format!("-v{RIG_VERSION}.json")), "{rel}");
        let rig = cache.rig(&rel).unwrap();
        assert_eq!(rig.mode, "Mover 16bit");
        assert_eq!(rig.thumbnail.as_deref(), Some("thumbnail.png"));
        let rig_dir = cache.assets_dir().join(rel.split('/').next().unwrap());
        assert_eq!(
            std::fs::read(rig_dir.join("models/yoke.glb")).unwrap(),
            b"yoke-mesh"
        );
        assert!(
            !rig_dir.join("models/unused.glb").exists(),
            "only meshes the rig draws are copied"
        );
        assert_eq!(
            std::fs::read(rig_dir.join("thumbnail.png")).unwrap(),
            b"png"
        );
        assert_eq!(parses.get(), 1);

        // Present: nothing is parsed again. Another mode of the same
        // archive shares the directory and adds a rig file.
        let (again, _) = cache
            .ensure_rig(&archive, "Mover 16bit", || panic!("must not parse"))
            .unwrap();
        assert_eq!(again, rel);
        let (other, _) = cache
            .ensure_rig(&archive, "8: RGBS", || Ok(&description))
            .unwrap();
        assert_ne!(other, rel);
        assert_eq!(
            other.split('/').next(),
            rel.split('/').next(),
            "same archive, same directory"
        );
        assert!(cache.rig("nope/rig.json").is_none());
    }

    #[test]
    fn a_rig_that_cannot_distill_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join(".cache"));
        let archive = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )]);
        let description = crate::lighting::gdtf::parse_archive(&archive).unwrap();
        let err = cache
            .ensure_rig(&archive, "No Such Mode", || Ok(&description))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no mode matching"), "{err}");
        assert!(!cache.assets_dir().exists());
    }

    #[test]
    fn the_scenery_store_holds_drawable_meshes_and_reports_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join(".cache"));
        let archive = crate::lighting::gdtf::build_zip(&[
            (
                "GeneralSceneDescription.xml",
                crate::lighting::mvr::SYNTHETIC_SCENE.as_bytes(),
            ),
            ("Lid.glb", b"lid-mesh".as_slice()),
            ("deck.glb", b"deck-mesh".as_slice()),
            ("Box.3ds", b"old".as_slice()),
        ]);
        let scene = crate::lighting::mvr::parse_archive(&archive).unwrap();
        let origin = [0.0, -3.5, 0.0];
        let (rel, warnings) = cache
            .ensure_scenery(&archive, &origin, || Ok(&scene))
            .unwrap();
        assert!(
            rel.starts_with("scenery/") && rel.ends_with(&format!("-v{SCENERY_VERSION}.json")),
            "{rel}"
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("1 .3ds"), "{warnings:?}");
        let model = cache.scenery(&rel).unwrap();
        assert_eq!(model.objects.len(), 2);
        let truss = &model.objects[0];
        assert_eq!(truss.kind, "Truss");
        // Re-origined and in meters, the basis kept as it was.
        assert_eq!(truss.transform[1][3], 3.5);
        assert_eq!(truss.transform[2][3], 6.0);
        assert_eq!(truss.transform[0][1], -1.0);
        assert_eq!(truss.skipped, vec!["Box.3ds".to_string()]);
        assert_eq!(truss.meshes.len(), 1);
        assert_eq!(truss.meshes[0].file, "models/Lid.glb");
        assert_eq!(
            truss.meshes[0].transform[2][3], 1.0,
            "relative offset in meters"
        );
        assert_eq!(truss.meshes[0].transform[0][0], 2.0, "symbol scale kept");
        let deck = &model.objects[1];
        assert_eq!(deck.transform[0][0], 7.5);
        assert_eq!(deck.meshes[0].file, "models/deck.glb");
        assert_eq!(model.formats["glb"], 2);
        assert_eq!(model.formats["3ds"], 1);
        let store_dir = cache.assets_dir().join(rel.rsplit_once('/').unwrap().0);
        assert_eq!(
            std::fs::read(store_dir.join("models/Lid.glb")).unwrap(),
            b"lid-mesh"
        );
        assert!(!store_dir.join("models/Box.3ds").exists());

        // Present: no parse. Another origin: another file, same meshes.
        let (again, _) = cache
            .ensure_scenery(&archive, &origin, || panic!("must not parse"))
            .unwrap();
        assert_eq!(again, rel);
        let (other, _) = cache
            .ensure_scenery(&archive, &[0.0; 3], || Ok(&scene))
            .unwrap();
        assert_ne!(other, rel);
        assert_eq!(
            other.rsplit_once('/').unwrap().0,
            rel.rsplit_once('/').unwrap().0
        );
    }

    #[test]
    fn a_cp437_misread_entry_name_still_matches_its_mesh() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join(".cache"));
        // The scene names the mesh in UTF-8; the archive entry is what a
        // cp437 read of the same bytes yields.
        let scene_xml = crate::lighting::mvr::SYNTHETIC_SCENE
            .replace("fileName=\"deck.glb\"", "fileName=\"Bühnenpodest_1.glb\"");
        let archive = crate::lighting::gdtf::build_zip(&[
            ("GeneralSceneDescription.xml", scene_xml.as_bytes()),
            ("B├╝hnenpodest_1.glb", b"deck-mesh".as_slice()),
            ("Lid.glb", b"lid".as_slice()),
        ]);
        let scene = crate::lighting::mvr::parse_archive(&archive).unwrap();
        let (rel, warnings) = cache
            .ensure_scenery(&archive, &[0.0; 3], || Ok(&scene))
            .unwrap();
        assert!(
            !warnings.iter().any(|w| w.contains("not in the archive")),
            "{warnings:?}"
        );
        let model = cache.scenery(&rel).unwrap();
        let deck = model.objects.iter().find(|o| o.name == "Deck").unwrap();
        assert_eq!(deck.meshes.len(), 1);
        assert_eq!(deck.meshes[0].file, "models/B__hnenpodest_1.glb");
        assert_eq!(ascii_skeleton("Bühnenpodest_1.glb"), "bhnenpodest1glb");
    }

    #[test]
    fn an_ambiguous_skeleton_match_is_skipped_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DistillCache::new(dir.path().join(".cache"));
        let scene_xml = crate::lighting::mvr::SYNTHETIC_SCENE
            .replace("fileName=\"deck.glb\"", "fileName=\"Bühne.glb\"");
        // Two entries that both skeleton to "bhneglb" and neither of which
        // matches exactly or case-folded: nothing is guessed.
        let archive = crate::lighting::gdtf::build_zip(&[
            ("GeneralSceneDescription.xml", scene_xml.as_bytes()),
            ("B├╝hne.glb", b"one".as_slice()),
            ("B_hne.glb", b"two".as_slice()),
            ("Lid.glb", b"lid".as_slice()),
        ]);
        let scene = crate::lighting::mvr::parse_archive(&archive).unwrap();
        let (rel, warnings) = cache
            .ensure_scenery(&archive, &[0.0; 3], || Ok(&scene))
            .unwrap();
        let model = cache.scenery(&rel).unwrap();
        let deck = model.objects.iter().find(|o| o.name == "Deck").unwrap();
        assert!(deck.meshes.is_empty(), "{:?}", deck.meshes);
        assert_eq!(deck.skipped, vec!["Bühne.glb".to_string()]);
        assert!(
            warnings.iter().any(|w| w.contains("more than one")),
            "{warnings:?}"
        );
    }

    #[test]
    fn mesh_file_names_are_safe_and_distinct() {
        let mut taken = std::collections::HashSet::new();
        assert_eq!(
            mesh_file_name("Geometrie_ab cd.glb", &mut taken),
            "Geometrie_ab_cd.glb"
        );
        let second = mesh_file_name("Geometrie_ab/cd.glb", &mut taken);
        assert_ne!(second, "Geometrie_ab_cd.glb");
        assert!(
            second.starts_with("cd") || second.starts_with("Geometrie"),
            "{second}"
        );
        assert_eq!(mesh_file_name("../evil.GLB", &mut taken), "evil.glb");
        assert_eq!(mesh_file_name("", &mut taken), "mesh.");
    }
}
