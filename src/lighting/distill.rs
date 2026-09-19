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

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::gdtf::{self, Description, RigModel, RIG_VERSION};
use super::types::{ChannelDef, FixtureType, GdtfSource, MovementLimits};

/// The asset store's directory under the cache.
pub const ASSETS_DIR: &str = "assets";

/// Bumped whenever the distiller's output for the same source can change.
/// Part of the cache key, so an upgrade regenerates every expansion.
pub const DISTILLER_VERSION: u32 = 2;

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
    /// `<archive sha256>/rig-<mode sha256 prefix>-v<RIG_VERSION>.json`.
    /// The archive hash keys the directory (its meshes are the archive's,
    /// whatever mode is in use); the mode and rig version key the file.
    pub fn rig_path(archive_bytes: &[u8], mode: &str) -> String {
        let archive = format!("{:x}", Sha256::digest(archive_bytes));
        let mode = format!("{:x}", Sha256::digest(mode.as_bytes()));
        format!("{archive}/rig-{}-v{RIG_VERSION}.json", &mode[..16])
    }

    /// Makes sure the store holds the rig model for an archive and mode,
    /// with the meshes and thumbnail it names, and returns the rig's
    /// store-relative path. A present rig file is trusted (the path is
    /// content-addressed); otherwise `describe` parses the archive once
    /// and everything is written, meshes first, the rig last.
    pub fn ensure_rig<'a>(
        &self,
        archive_bytes: &[u8],
        mode: &str,
        describe: impl FnOnce() -> Result<&'a Description, Box<dyn Error>>,
    ) -> Result<String, Box<dyn Error>> {
        let rel = Self::rig_path(archive_bytes, mode);
        let rig_file = self.assets_dir().join(&rel);
        if rig_file.is_file() {
            return Ok(rel);
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
        Ok(rel)
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
        let rel = cache.ensure_rig(&archive, "Mover 16bit", describe).unwrap();
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
        let again = cache
            .ensure_rig(&archive, "Mover 16bit", || panic!("must not parse"))
            .unwrap();
        assert_eq!(again, rel);
        let other = cache
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
}
