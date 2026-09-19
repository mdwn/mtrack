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

//! Hardened access to a GDTF archive's `description.xml`.
//!
//! Nothing is extracted by the archive's own names — the asset cache writes
//! model files under names it derives, so zip-slip has no surface here.
//! What remains is decompression abuse, capped hard: a bounded entry count,
//! a bounded decompressed size per entry and for all assets together, and a
//! strict read that refuses an entry lying about its size.

use std::collections::HashSet;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use super::GdtfError;

/// The most entries a plausible GDTF archive holds. Real manufacturer files
/// carry a description, thumbnails, and a models tree — tens of entries;
/// wheel-heavy fixtures a few hundred.
const MAX_ARCHIVE_ENTRIES: usize = 4096;

/// The largest decompressed `description.xml` accepted. Real descriptions
/// run tens of KB to a few MB (the Astera PixelBrick's is 315 KB); the cap
/// leaves an order of magnitude of headroom while keeping a zip bomb's
/// petabyte claims un-decompressed.
const MAX_DESCRIPTION_BYTES: u64 = 64 * 1024 * 1024;

/// The largest whole archive accepted. Real manufacturer files run 1–10 MB
/// (description + thumbnails + 3D models); the cap bounds the central
/// directory parse and everything after it.
const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

/// The largest mesh accepted. Manufacturer glTF models run tens of KB to a
/// few MB.
const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024;

/// The most bytes of assets (meshes and thumbnail) read out of one archive.
const MAX_ASSET_BYTES_TOTAL: u64 = 64 * 1024 * 1024;

/// The archive directory that holds glTF meshes.
const MODELS_DIR: &str = "models/gltf/";

/// Opens an archive with the entry-count and size caps applied.
fn open(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, GdtfError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(GdtfError::new(format!(
            "archive is {} bytes; refusing more than {MAX_ARCHIVE_BYTES}",
            bytes.len()
        )));
    }
    let archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| GdtfError::new(format!("not a readable GDTF archive: {e}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(GdtfError::new(format!(
            "archive has {} entries; refusing more than {MAX_ARCHIVE_ENTRIES}",
            archive.len()
        )));
    }
    Ok(archive)
}

/// Reads one entry through a hard limit; an entry lying about its size
/// can't decompress past the cap either.
fn read_capped(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    cap: u64,
) -> Result<Vec<u8>, GdtfError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| GdtfError::new(format!("archive has no {name} ({e})")))?;
    if entry.size() > cap {
        return Err(GdtfError::new(format!(
            "{name} claims {} bytes; refusing more than {cap}",
            entry.size()
        )));
    }
    let mut content = Vec::new();
    entry
        .by_ref()
        .take(cap + 1)
        .read_to_end(&mut content)
        .map_err(|e| GdtfError::new(format!("failed to read {name}: {e}")))?;
    if content.len() as u64 > cap {
        return Err(GdtfError::new(format!(
            "{name} decompressed past the {cap}-byte cap"
        )));
    }
    Ok(content)
}

/// The mesh stems the archive carries under `models/gltf/`, lowercased —
/// what a model table's `File` can resolve to.
pub fn list_model_files(bytes: &[u8]) -> Result<HashSet<String>, GdtfError> {
    let archive = open(bytes)?;
    Ok(archive
        .file_names()
        .filter_map(|name| {
            let rest = name.strip_prefix(MODELS_DIR)?;
            let stem = rest.strip_suffix(".glb")?;
            (!stem.is_empty() && !stem.contains('/')).then(|| stem.to_ascii_lowercase())
        })
        .collect())
}

/// The assets read out of an archive: meshes by lowercased stem, and the
/// thumbnail by extension.
#[derive(Default)]
pub struct Assets {
    /// `(stem, glb bytes)`.
    pub models: Vec<(String, Vec<u8>)>,
    /// `("png" | "svg", bytes)`.
    pub thumbnail: Option<(String, Vec<u8>)>,
}

/// Reads the meshes named by `stems` (lowercased) and the thumbnail named
/// by the fixture type, if any, under the per-asset and total caps. A stem
/// the archive lacks is skipped; a mesh over the cap is an error, since a
/// rig that names it would draw nothing.
pub fn read_assets(
    bytes: &[u8],
    stems: &HashSet<String>,
    thumbnail: Option<&str>,
) -> Result<Assets, GdtfError> {
    let mut archive = open(bytes)?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    let mut assets = Assets::default();
    let mut total: u64 = 0;
    let mut budget = |len: usize| -> Result<(), GdtfError> {
        total += len as u64;
        if total > MAX_ASSET_BYTES_TOTAL {
            return Err(GdtfError::new(format!(
                "archive assets exceed {MAX_ASSET_BYTES_TOTAL} bytes together"
            )));
        }
        Ok(())
    };
    for name in &names {
        let Some(stem) = name
            .strip_prefix(MODELS_DIR)
            .and_then(|rest| rest.strip_suffix(".glb"))
        else {
            continue;
        };
        let lowered = stem.to_ascii_lowercase();
        if !stems.contains(&lowered) || assets.models.iter().any(|(s, _)| *s == lowered) {
            continue;
        }
        let content = read_capped(&mut archive, name, MAX_MODEL_BYTES)?;
        budget(content.len())?;
        assets.models.push((lowered, content));
    }
    if let Some(thumbnail) = thumbnail {
        for ext in ["png", "svg"] {
            let wanted = format!("{thumbnail}.{ext}");
            if let Some(name) = names.iter().find(|n| n.eq_ignore_ascii_case(&wanted)) {
                let content = read_capped(&mut archive, name, MAX_MODEL_BYTES)?;
                budget(content.len())?;
                assets.thumbnail = Some((ext.to_string(), content));
                break;
            }
        }
    }
    Ok(assets)
}

/// Reads `description.xml` out of a GDTF archive held in memory.
pub fn read_description_xml(bytes: &[u8]) -> Result<String, GdtfError> {
    let mut archive = open(bytes)?;

    let mut entry = archive.by_name("description.xml").map_err(|e| {
        GdtfError::new(format!(
            "archive has no description.xml ({e}); is this a GDTF file?"
        ))
    })?;

    if entry.size() > MAX_DESCRIPTION_BYTES {
        return Err(GdtfError::new(format!(
            "description.xml claims {} bytes; refusing more than {MAX_DESCRIPTION_BYTES}",
            entry.size()
        )));
    }

    // The claimed size is untrusted; read through a hard limit so an entry
    // lying about its size can't decompress past the cap either.
    let mut content = Vec::new();
    entry
        .by_ref()
        .take(MAX_DESCRIPTION_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|e| GdtfError::new(format!("failed to read description.xml: {e}")))?;
    if content.len() as u64 > MAX_DESCRIPTION_BYTES {
        return Err(GdtfError::new(format!(
            "description.xml decompressed past the {MAX_DESCRIPTION_BYTES}-byte cap"
        )));
    }

    // GDTF descriptions are UTF-8. Some tools emit a BOM; strip it rather
    // than letting it poison the first tag name.
    let content = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        content[3..].to_vec()
    } else {
        content
    };
    String::from_utf8(content).map_err(|_| GdtfError::new("description.xml is not valid UTF-8"))
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::Write;

    use super::*;

    /// Builds an in-memory zip with the given entries.
    pub(crate) fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, content) in entries {
                writer.start_file(*name, options).unwrap();
                writer.write_all(content).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn reads_description_xml() {
        let bytes = build_zip(&[
            ("description.xml", b"<GDTF/>".as_slice()),
            ("thumbnail.png", b"not xml".as_slice()),
        ]);
        assert_eq!(read_description_xml(&bytes).unwrap(), "<GDTF/>");
    }

    #[test]
    fn strips_utf8_bom() {
        let bytes = build_zip(&[("description.xml", b"\xEF\xBB\xBF<GDTF/>".as_slice())]);
        assert_eq!(read_description_xml(&bytes).unwrap(), "<GDTF/>");
    }

    #[test]
    fn missing_description_is_a_clear_error() {
        let bytes = build_zip(&[("thumbnail.png", b"png".as_slice())]);
        let err = read_description_xml(&bytes).unwrap_err().to_string();
        assert!(err.contains("no description.xml"), "{err}");
    }

    #[test]
    fn garbage_is_not_an_archive() {
        let err = read_description_xml(b"not a zip").unwrap_err().to_string();
        assert!(err.contains("not a readable GDTF archive"), "{err}");
    }

    #[test]
    fn non_utf8_description_is_rejected() {
        let bytes = build_zip(&[("description.xml", &[0xFF, 0xFE, 0x00][..])]);
        let err = read_description_xml(&bytes).unwrap_err().to_string();
        assert!(err.contains("not valid UTF-8"), "{err}");
    }

    #[test]
    fn assets_are_listed_and_read_by_stem_with_the_thumbnail() {
        let bytes = build_zip(&[
            ("description.xml", b"<GDTF/>".as_slice()),
            ("thumbnail.png", b"png".as_slice()),
            ("thumbnail.svg", b"svg".as_slice()),
            ("models/gltf/Base.glb", b"base-mesh".as_slice()),
            ("models/gltf/yoke.glb", b"yoke-mesh".as_slice()),
            ("models/3ds/yoke.3ds", b"old".as_slice()),
            ("models/gltf_low/yoke.glb", b"low".as_slice()),
        ]);
        let files = list_model_files(&bytes).unwrap();
        assert_eq!(
            files,
            ["base".to_string(), "yoke".to_string()]
                .into_iter()
                .collect()
        );
        let wanted: HashSet<String> = ["base".to_string(), "missing".to_string()]
            .into_iter()
            .collect();
        let assets = read_assets(&bytes, &wanted, Some("thumbnail")).unwrap();
        assert_eq!(
            assets.models,
            vec![("base".to_string(), b"base-mesh".to_vec())]
        );
        assert_eq!(
            assets.thumbnail,
            Some(("png".to_string(), b"png".to_vec())),
            "png is preferred over svg"
        );
        let none = read_assets(&bytes, &HashSet::new(), None).unwrap();
        assert!(none.models.is_empty());
        assert!(none.thumbnail.is_none());
    }

    #[test]
    fn oversized_description_is_rejected_by_claimed_size() {
        // A genuinely huge (but honest) entry is refused before decompression.
        let big = vec![b'a'; (MAX_DESCRIPTION_BYTES + 1) as usize];
        let bytes = build_zip(&[("description.xml", big.as_slice())]);
        let err = read_description_xml(&bytes).unwrap_err().to_string();
        assert!(err.contains("refusing"), "{err}");
    }
}
