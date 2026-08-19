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
//! Only the description is ever read — nothing is extracted to disk, so
//! zip-slip has no surface here. What remains is decompression abuse, capped
//! hard: a bounded entry count, a bounded decompressed size for the
//! description, and a strict read that refuses an entry lying about its
//! size. Asset entries (models, thumbnails) are ignored entirely until the
//! phase that consumes them.

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

/// Reads `description.xml` out of a GDTF archive held in memory.
pub fn read_description_xml(bytes: &[u8]) -> Result<String, GdtfError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| GdtfError::new(format!("not a readable GDTF archive: {e}")))?;

    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(GdtfError::new(format!(
            "archive has {} entries; refusing more than {MAX_ARCHIVE_ENTRIES}",
            archive.len()
        )));
    }

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
    pub(in super::super) fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
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
    fn oversized_description_is_rejected_by_claimed_size() {
        // A genuinely huge (but honest) entry is refused before decompression.
        let big = vec![b'a'; (MAX_DESCRIPTION_BYTES + 1) as usize];
        let bytes = build_zip(&[("description.xml", big.as_slice())]);
        let err = read_description_xml(&bytes).unwrap_err().to_string();
        assert!(err.contains("refusing"), "{err}");
    }
}
