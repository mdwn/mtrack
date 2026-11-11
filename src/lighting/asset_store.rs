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

//! Reading a file of the project's asset store (`lighting/.cache/assets/`,
//! design §16.2): rig models, meshes and thumbnails for the 3D view. The web
//! UI serves them over HTTP and the gRPC service over `GetLightingAsset`;
//! both read them here, so the two hold to the same rules.

use std::path::Path;

use crate::webui::safe_path::{SafePath, SafePathError, VerifiedRoot};

/// The file types the asset store serves, by extension. Everything in the
/// store was written by mtrack itself from a GDTF archive; the allowlist
/// keeps the store from ever becoming a general file server.
const ASSET_TYPES: &[(&str, &str)] = &[
    ("json", "application/json"),
    ("glb", "model/gltf-binary"),
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
];

/// The largest asset served; the store's own caps are lower.
pub const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

/// A file read from the store.
pub struct Asset {
    pub bytes: Vec<u8>,
    /// The media type its extension gives it.
    pub content_type: &'static str,
}

/// Why an asset was not read.
#[derive(Debug)]
pub enum AssetError {
    /// The path's extension is not one the store serves.
    NotAnAsset,
    /// The project has no store yet (no referential fixture type has
    /// expanded).
    NoStore,
    /// The path is not one that stays inside the store.
    Path(SafePathError),
    /// There is no such file, it is not a file, or it is over the cap.
    Unreadable,
}

/// The media type of the store file at `path`, by its extension.
fn content_type(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str())?;
    ASSET_TYPES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(ext))
        .map(|(_, mime)| *mime)
}

/// Reads the asset at `path`, relative to the store of the project at
/// `project`. Blocks on the file system: call it off the async runtime.
pub fn read_asset(project: &Path, path: &str) -> Result<Asset, AssetError> {
    let content_type = content_type(path).ok_or(AssetError::NotAnAsset)?;
    let store = project
        .join("lighting")
        .join(".cache")
        .join(super::distill::ASSETS_DIR);
    let root = VerifiedRoot::new(&store).map_err(|_| AssetError::NoStore)?;
    let file = SafePath::validate_relative(path, &root).map_err(AssetError::Path)?;

    let unreadable = |_| AssetError::Unreadable;
    let meta = std::fs::metadata(&file).map_err(unreadable)?;
    if !meta.is_file() || meta.len() > MAX_ASSET_BYTES {
        return Err(AssetError::Unreadable);
    }
    let bytes = std::fs::read(&file).map_err(unreadable)?;
    Ok(Asset {
        bytes,
        content_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("lighting/.cache/assets/abc");
        std::fs::create_dir_all(store.join("models/dir.json")).unwrap();
        std::fs::write(store.join("rig.json"), "{}").unwrap();
        std::fs::write(store.join("notes.txt"), "no").unwrap();
        std::fs::write(dir.path().join("lighting/secret.json"), "{}").unwrap();
        dir
    }

    #[test]
    fn a_file_of_the_store_is_read_with_its_type() {
        let dir = project();
        let asset = read_asset(dir.path(), "abc/rig.json").unwrap();
        assert_eq!(asset.bytes, b"{}");
        assert_eq!(asset.content_type, "application/json");
        assert_eq!(content_type("A/B.GLB"), Some("model/gltf-binary"));
    }

    #[test]
    fn what_the_store_does_not_serve_is_refused() {
        let dir = project();
        assert!(matches!(
            read_asset(dir.path(), "abc/notes.txt"),
            Err(AssetError::NotAnAsset)
        ));
        for missing in [
            "abc/models/head.glb",
            "abc/models/dir.json",
            "abc/models/x.json",
        ] {
            assert!(
                matches!(read_asset(dir.path(), missing), Err(AssetError::Unreadable)),
                "{missing}"
            );
        }
        for escape in ["../secret.json", "abc/../../secret.json", "/etc/x.json"] {
            assert!(
                matches!(read_asset(dir.path(), escape), Err(AssetError::Path(_))),
                "{escape}"
            );
        }
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_asset(empty.path(), "abc/rig.json"),
            Err(AssetError::NoStore)
        ));
    }

    #[test]
    fn a_file_over_the_cap_is_refused() {
        let dir = project();
        let big = dir.path().join("lighting/.cache/assets/abc/big.glb");
        let file = std::fs::File::create(big).unwrap();
        file.set_len(MAX_ASSET_BYTES + 1).unwrap();
        assert!(matches!(
            read_asset(dir.path(), "abc/big.glb"),
            Err(AssetError::Unreadable)
        ));
    }
}
