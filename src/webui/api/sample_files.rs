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

//! Sample files added to the project: the plain function behind
//! `ConfigService.AddSampleFile`, and the checks `BeginUpload` makes of the
//! name as the upload is announced.

use super::super::safe_path::SafePath;
use super::songs_api::{blocking_io, SongError};
use crate::songs as songs_crate;

/// Validates that a filename has a supported audio extension (for sample uploads).
fn validate_sample_filename(filename: &str) -> Result<(), SongError> {
    if SafePath::validate_name(filename).is_err() {
        return Err(SongError::Invalid("Invalid filename".to_string()));
    }

    let path = std::path::Path::new(filename);
    if songs_crate::FileKind::of(path) != songs_crate::FileKind::Audio {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        return Err(SongError::Invalid(format!(
            "Unsupported audio file type: .{ext}"
        )));
    }

    Ok(())
}

/// The canonical project root, from which the samples directory is built, so
/// that all filesystem operations use a verified base.
///
/// `project_dir_of`, not `parent()`: the latter answers Some("") for a bare
/// filename, so the fallback never fires and the empty path fails to
/// canonicalize -- a 500 for every sample upload whenever mtrack was started
/// as `mtrack start mtrack.yaml`.
fn canonical_project_root(config_path: &std::path::Path) -> Result<std::path::PathBuf, SongError> {
    crate::util::project_dir_of(config_path)
        .canonicalize()
        .map_err(|e| SongError::Internal(format!("Failed to resolve project root: {e}")))
}

/// The file a sample upload named `filename` would write: `samples/` in the
/// project, made first when `make_dir`. A new sample's name must also pass
/// the rules for new names; one already there is replaced under its own.
fn sample_target(
    config_path: &std::path::Path,
    filename: &str,
    make_dir: bool,
) -> Result<std::path::PathBuf, SongError> {
    validate_sample_filename(filename)?;
    let root_canonical = canonical_project_root(config_path)?;
    let samples_dir = root_canonical.join("samples");

    if make_dir && !samples_dir.exists() {
        std::fs::create_dir_all(&samples_dir)
            .map_err(|e| SongError::Internal(format!("Failed to create samples directory: {e}")))?;
    }

    let file_path = samples_dir.join(filename);
    let refused = !file_path.exists() && SafePath::validate_new_name(filename).is_err();
    if refused || !file_path.starts_with(&root_canonical) {
        return Err(SongError::Invalid("Invalid filename".to_string()));
    }
    Ok(file_path)
}

/// What a sample upload announces: the name, held to the rules
/// [`add_sample_file`] holds it to, with nothing made.
pub(crate) fn check_sample_upload(
    config_path: &std::path::Path,
    filename: &str,
) -> Result<(), SongError> {
    sample_target(config_path, filename, false).map(|_| ())
}

/// A sample file stored.
pub(crate) struct SampleAdded {
    pub(crate) file: String,
    /// Relative to the project, as a sample definition names it.
    pub(crate) path: String,
}

/// Stores the staged `file` as a sample in the `samples/` directory next to
/// the config file, which is made if it is not there. The staged file is
/// deleted whether this succeeds or not.
pub(crate) async fn add_sample_file(
    config_path: &std::path::Path,
    file: crate::webui::transfers::Claimed,
) -> Result<SampleAdded, SongError> {
    let filename = file.file_name.clone();
    let file_path = sample_target(config_path, &filename, true)?;
    blocking_io("write file", move || file.write_to(&file_path)).await?;
    Ok(SampleAdded {
        path: format!("samples/{filename}"),
        file: filename,
    })
}
