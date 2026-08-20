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

//! Hardened access to an MVR archive's contents.
//!
//! Two things are ever read: `GeneralSceneDescription.xml`, and — at import
//! time — embedded `.gdtf` archives, each of which then goes through the
//! GDTF layer's own caps. Nothing is extracted to disk. MVR archives run
//! larger than GDTF (several embedded fixtures plus scene assets), so the
//! whole-archive cap is wider.

use std::io::{Cursor, Read};

use zip::ZipArchive;

use super::MvrError;

/// The largest whole archive accepted. Real venue files run tens to a few
/// hundred MB depending on embedded models.
const MAX_ARCHIVE_BYTES: usize = 512 * 1024 * 1024;

/// The most entries a plausible MVR holds.
const MAX_ARCHIVE_ENTRIES: usize = 8192;

/// The largest decompressed scene description accepted.
const MAX_SCENE_BYTES: u64 = 64 * 1024 * 1024;

/// The largest embedded GDTF accepted — the GDTF layer's own whole-archive
/// cap, applied here before the bytes are handed to it.
const MAX_EMBEDDED_GDTF_BYTES: u64 = 256 * 1024 * 1024;

fn open(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, MvrError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(MvrError::new(format!(
            "archive is {} bytes; refusing more than {MAX_ARCHIVE_BYTES}",
            bytes.len()
        )));
    }
    let archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| MvrError::new(format!("not a readable MVR archive: {e}")))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(MvrError::new(format!(
            "archive has {} entries; refusing more than {MAX_ARCHIVE_ENTRIES}",
            archive.len()
        )));
    }
    Ok(archive)
}

/// Reads a named entry with a hard decompression cap, refusing entries that
/// lie about their size.
fn read_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    cap: u64,
) -> Result<Vec<u8>, MvrError> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| MvrError::new(format!("archive has no {name} ({e})")))?;
    if entry.size() > cap {
        return Err(MvrError::new(format!(
            "{name} claims {} bytes; refusing more than {cap}",
            entry.size()
        )));
    }
    let mut content = Vec::new();
    entry
        .by_ref()
        .take(cap + 1)
        .read_to_end(&mut content)
        .map_err(|e| MvrError::new(format!("failed to read {name}: {e}")))?;
    if content.len() as u64 > cap {
        return Err(MvrError::new(format!(
            "{name} decompressed past the {cap}-byte cap"
        )));
    }
    Ok(content)
}

/// Reads `GeneralSceneDescription.xml` out of an MVR archive held in memory.
pub fn read_scene_xml(bytes: &[u8]) -> Result<String, MvrError> {
    let mut archive = open(bytes)?;
    let content = read_entry(&mut archive, "GeneralSceneDescription.xml", MAX_SCENE_BYTES)
        .map_err(|e| MvrError::new(format!("{e}; is this an MVR file?")))?;
    let content = if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
        content[3..].to_vec()
    } else {
        content
    };
    String::from_utf8(content)
        .map_err(|_| MvrError::new("GeneralSceneDescription.xml is not valid UTF-8"))
}

/// Lists the embedded `.gdtf` entry names, sorted.
pub fn list_gdtf_entries(bytes: &[u8]) -> Result<Vec<String>, MvrError> {
    let archive = open(bytes)?;
    let mut names: Vec<String> = archive
        .file_names()
        .filter(|name| name.to_ascii_lowercase().ends_with(".gdtf"))
        .map(|name| name.to_string())
        .collect();
    names.sort();
    Ok(names)
}

/// Reads one embedded GDTF archive by entry name. The returned bytes go
/// through the GDTF layer, which applies its own caps again.
pub fn read_gdtf_entry(bytes: &[u8], name: &str) -> Result<Vec<u8>, MvrError> {
    let mut archive = open(bytes)?;
    read_entry(&mut archive, name, MAX_EMBEDDED_GDTF_BYTES)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn reads_scene_and_lists_gdtfs() {
        let bytes = crate::lighting::gdtf::build_zip(&[
            (
                "GeneralSceneDescription.xml",
                b"<GeneralSceneDescription/>".as_slice(),
            ),
            ("fixtures/PB15.gdtf", b"gdtf bytes".as_slice()),
            ("fixtures/Mover.GDTF", b"more".as_slice()),
            ("scenery/truss.3ds", b"not a fixture".as_slice()),
        ]);
        assert_eq!(
            read_scene_xml(&bytes).unwrap(),
            "<GeneralSceneDescription/>"
        );
        assert_eq!(
            list_gdtf_entries(&bytes).unwrap(),
            vec![
                "fixtures/Mover.GDTF".to_string(),
                "fixtures/PB15.gdtf".to_string()
            ]
        );
        assert_eq!(
            read_gdtf_entry(&bytes, "fixtures/PB15.gdtf").unwrap(),
            b"gdtf bytes"
        );
    }

    #[test]
    fn a_zip_without_a_scene_is_a_clear_error() {
        let bytes = crate::lighting::gdtf::build_zip(&[("x.txt", b"hi".as_slice())]);
        let err = read_scene_xml(&bytes).unwrap_err().to_string();
        assert!(err.contains("is this an MVR file?"), "{err}");
    }

    #[test]
    fn garbage_is_not_an_archive() {
        let err = read_scene_xml(b"junk").unwrap_err().to_string();
        assert!(err.contains("not a readable MVR archive"), "{err}");
    }

    #[test]
    fn missing_gdtf_entry_is_a_clear_error() {
        let bytes = crate::lighting::gdtf::build_zip(&[(
            "GeneralSceneDescription.xml",
            b"<x/>".as_slice(),
        )]);
        let err = read_gdtf_entry(&bytes, "nope.gdtf")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no nope.gdtf"), "{err}");
    }
}
