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

//! Verified filesystem paths that are guaranteed to be under a trusted root.
//!
//! `SafePath` can only be constructed through methods that canonicalize the path
//! and verify it is contained within a specified root directory. This prevents
//! path traversal attacks and centralizes the validation logic that was previously
//! duplicated across every API handler.

use std::path::{Path, PathBuf};

/// A canonicalized path that has been verified to reside under a trusted root directory.
///
/// Cannot be constructed directly — use [`SafePath::resolve`], [`SafePath::create_dir`],
/// or [`SafePath::create_dir_nested`] to obtain one.
#[derive(Debug, Clone)]
pub struct SafePath {
    path: PathBuf,
}

impl SafePath {
    /// Returns the verified canonical path.
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Joins a filename to this verified path. Safe for both constant filenames
    /// (e.g. "song.yaml") and user-provided filenames that have been validated
    /// via `SafePath::validate_name()`.
    pub fn join_filename(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// Returns true if the path is a directory.
    pub fn is_dir(&self) -> bool {
        self.path.is_dir()
    }

    /// Resolves an existing path under the given root.
    /// The path must exist and must resolve to a location within the root.
    pub fn resolve(path: &Path, root: &VerifiedRoot) -> Result<Self, SafePathError> {
        let canonical = path.canonicalize().map_err(|_| SafePathError::NotFound)?;
        if !canonical.starts_with(root.as_path()) {
            return Err(SafePathError::OutsideRoot);
        }
        Ok(SafePath { path: canonical })
    }

    /// Creates a single directory under the given root and returns the verified path.
    /// The name must be a single path segment (no slashes). A directory not
    /// there yet takes a new name, so it must pass
    /// [`SafePath::validate_new_name`]; one already there need only be safe.
    pub fn create_dir(
        parent: &SafePath,
        name: &str,
        root: &VerifiedRoot,
    ) -> Result<Self, SafePathError> {
        Self::validate_name(name)?;
        let target = parent.path.join(name);
        if !target.exists() {
            Self::validate_new_name(name)?;
            crate::util::create_dir_all(&target).map_err(SafePathError::Io)?;
        }
        let canonical = target.canonicalize().map_err(|_| SafePathError::NotFound)?;
        if !canonical.starts_with(root.as_path()) {
            // Clean up if a symlink caused escape.
            let _ = std::fs::remove_dir(&target);
            return Err(SafePathError::OutsideRoot);
        }
        Ok(SafePath { path: canonical })
    }

    /// Validates a single path segment (filename or directory name) is safe.
    /// Rejects empty strings, `.`, `..`, and path separators.
    ///
    /// Enough for a name already on disk, to read, save, rename away from or
    /// delete; a name being given to something new must also pass
    /// [`SafePath::validate_new_name`].
    pub fn validate_name(name: &str) -> Result<(), SafePathError> {
        if name.is_empty() || name == "." || name.contains('/') {
            return Err(SafePathError::InvalidName);
        }
        Self::validate_relative_segments(name)
    }

    /// What every relative path under the root is held to: it stays inside
    /// ([`crate::util::escapes_project`], the rule a playlist's paths are held
    /// to as well), with no backslash or NUL in it.
    fn validate_relative_segments(relative: &str) -> Result<(), SafePathError> {
        if relative.is_empty()
            || relative.contains('\\')
            || relative.contains('\0')
            || crate::util::escapes_project(Path::new(relative))
        {
            return Err(SafePathError::InvalidName);
        }
        Ok(())
    }

    /// Validates a name for something new: safe, as [`SafePath::validate_name`]
    /// checks, and not a name that is blank or has space at either end,
    /// which reads as a different name or none in a list; one starting with
    /// a dot, which the song loader skips as hidden, so what was saved
    /// vanishes; or one with control characters. Names Windows reserves
    /// (`CON`) are fine on the systems mtrack runs on.
    ///
    /// Only for a new name: files with names like these from before these
    /// rules stay readable, savable and deletable.
    pub fn validate_new_name(name: &str) -> Result<(), SafePathError> {
        Self::validate_name(name)?;
        let hidden = crate::songs::is_hidden(std::path::Path::new(name));
        if name.trim() != name || hidden || name.chars().any(char::is_control) {
            return Err(SafePathError::InvalidName);
        }
        Ok(())
    }

    /// Validates a name to write to: [`SafePath::validate_new_name`]'s rules
    /// unless `exists`, a file of that name already being there.
    pub fn validate_name_to_write(name: &str, exists: bool) -> Result<(), SafePathError> {
        if exists {
            Self::validate_name(name)
        } else {
            Self::validate_new_name(name)
        }
    }

    /// Validates a relative path under the root without creating anything,
    /// for reading: [`SafePath::validate_relative_entry`]'s path, with a
    /// dangling link refused. Use this for read-only operations where the
    /// directory may not exist.
    pub fn validate_relative(
        relative: &str,
        root: &VerifiedRoot,
    ) -> Result<PathBuf, SafePathError> {
        let entry = Self::validate_relative_entry(relative, root)?;
        if entry.is_dangling_link() {
            return Err(SafePathError::NotFound);
        }
        Ok(entry.into_path())
    }

    /// The entry at a relative path under the root, checked without
    /// creating anything: each segment is checked for traversal (no "..",
    /// "\", null); the nearest existing directory above it must resolve
    /// inside the root, so a linked subdirectory is no way out; and when
    /// the entry resolves (through a link too), what it resolves to must be
    /// inside the root. The path given back is the entry itself, under the
    /// root, not what a link at it points at: deleting it removes the link,
    /// not its target. A link that resolves to nothing is reported, as
    /// writing through it would create whatever it names.
    pub fn validate_relative_entry(
        relative: &str,
        root: &VerifiedRoot,
    ) -> Result<CheckedEntry, SafePathError> {
        Self::validate_relative_segments(relative)?;
        let path = root.as_path().join(relative);
        let is_there = |path: &Path| path.symlink_metadata().is_ok();
        if let Some(ancestor) = path.ancestors().skip(1).find(|a| is_there(a)) {
            SafePath::resolve(ancestor, root)?;
        }
        if path.exists() {
            SafePath::resolve(&path, root)?;
        }
        let dangling = is_there(&path) && !path.exists();
        Ok(CheckedEntry { path, dangling })
    }

    /// Creates a nested directory path (e.g. "Artist/Album/Song") segment by segment,
    /// verifying containment at each step.
    pub fn create_dir_nested(name: &str, root: &VerifiedRoot) -> Result<Self, SafePathError> {
        Self::validate_relative_segments(name)?;
        // Each segment to be created is checked before any is, so a bad name
        // further down ("Artist/.hidden") leaves no new directory behind.
        let mut existing = root.as_path().to_path_buf();
        for segment in name.split('/').filter(|s| !s.is_empty()) {
            existing.push(segment);
            if !existing.exists() {
                Self::validate_new_name(segment)?;
            }
        }
        let mut current = SafePath {
            path: root.as_path().to_path_buf(),
        };
        for segment in name.split('/') {
            if segment.is_empty() {
                continue; // skip double slashes
            }
            current = SafePath::create_dir(&current, segment, root)?;
        }
        Ok(current)
    }
}

impl AsRef<Path> for SafePath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for SafePath {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

/// An entry under a root, checked by [`SafePath::validate_relative_entry`].
#[derive(Debug, Clone)]
pub struct CheckedEntry {
    path: PathBuf,
    dangling: bool,
}

impl CheckedEntry {
    /// The entry's path under the root (a link itself, not its target).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The entry's path under the root.
    pub fn into_path(self) -> PathBuf {
        self.path
    }

    /// Whether the entry is a link that resolves to nothing.
    pub fn is_dangling_link(&self) -> bool {
        self.dangling
    }
}

/// A canonicalized root directory used as the trust anchor for path verification.
#[derive(Debug, Clone)]
pub struct VerifiedRoot {
    path: PathBuf,
}

impl VerifiedRoot {
    /// Creates a VerifiedRoot by canonicalizing the given path.
    pub fn new(path: &Path) -> Result<Self, SafePathError> {
        let canonical = path
            .canonicalize()
            .map_err(|_| SafePathError::RootNotFound)?;
        Ok(VerifiedRoot { path: canonical })
    }

    /// Returns the canonical root path.
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Convenience: resolve an existing path relative to this root.
    pub fn resolve(&self, relative: &str) -> Result<SafePath, SafePathError> {
        SafePath::resolve(&self.path.join(relative), self)
    }

    /// Convenience: create a SafePath pointing at the root itself.
    pub fn as_safe_path(&self) -> SafePath {
        SafePath {
            path: self.path.clone(),
        }
    }
}

/// Errors from SafePath operations.
#[derive(Debug)]
pub enum SafePathError {
    /// The path does not exist.
    NotFound,
    /// The root directory could not be resolved.
    RootNotFound,
    /// The resolved path is outside the trusted root.
    OutsideRoot,
    /// The provided name contains invalid characters.
    InvalidName,
    /// A filesystem operation failed.
    Io(std::io::Error),
}

impl std::fmt::Display for SafePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafePathError::NotFound => write!(f, "Path not found"),
            SafePathError::RootNotFound => write!(f, "Root directory not found"),
            SafePathError::OutsideRoot => write!(f, "Path is outside the allowed directory"),
            SafePathError::InvalidName => write!(f, "Invalid path name"),
            SafePathError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for SafePathError {}

impl SafePathError {
    /// Convert to an axum response with appropriate HTTP status code.
    pub fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use axum::Json;
        let (status, msg) = match &self {
            SafePathError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            SafePathError::RootNotFound => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            SafePathError::OutsideRoot => (StatusCode::FORBIDDEN, self.to_string()),
            SafePathError::InvalidName => (StatusCode::BAD_REQUEST, self.to_string()),
            SafePathError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, Json(serde_json::json!({"error": msg}))).into_response()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn validate_new_name_refuses_names_that_read_wrong_or_vanish() {
        // #578: names like these were saved, then shown blank, as another
        // name, or (a hidden song directory) not loaded at all.
        for name in [" ", "  ", " lead", "trail ", ".hidden", "tab\there", "nl\n"] {
            assert!(SafePath::validate_new_name(name).is_err(), "{name:?}");
            assert!(
                SafePath::validate_name_to_write(name, false).is_err(),
                "{name:?}"
            );
        }
        for name in ["Battle Scarred", "show.light", "CON", "a_b-c (2)"] {
            assert!(SafePath::validate_new_name(name).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn a_name_already_on_disk_needs_only_be_safe() {
        // Files named before the rules for new names stay usable: saved,
        // renamed away from, deleted.
        for name in [" ", "trail ", ".warmup", "tab\there"] {
            assert!(SafePath::validate_name(name).is_ok(), "{name:?}");
            assert!(
                SafePath::validate_name_to_write(name, true).is_ok(),
                "{name:?}"
            );
        }
        for name in ["", ".", "..", "a/b", "a\\b", "a\0b", "../x"] {
            assert!(
                SafePath::validate_name_to_write(name, true).is_err(),
                "{name:?}"
            );
        }
    }

    use super::*;

    #[test]
    fn resolve_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("child");
        std::fs::create_dir(&sub).unwrap();

        let root = VerifiedRoot::new(dir.path()).unwrap();
        let safe = SafePath::resolve(&sub, &root).unwrap();
        assert!(safe.is_dir());
    }

    #[test]
    fn resolve_outside_root_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();

        let root = VerifiedRoot::new(dir.path()).unwrap();
        let result = SafePath::resolve(other.path(), &root);
        assert!(result.is_err());
    }

    #[test]
    fn create_dir_single_segment() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        let parent = root.as_safe_path();

        let safe = SafePath::create_dir(&parent, "newsong", &root).unwrap();
        assert!(safe.is_dir());
        assert!(safe.as_path().ends_with("newsong"));
    }

    #[test]
    fn create_dir_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        let parent = root.as_safe_path();

        assert!(SafePath::create_dir(&parent, "..", &root).is_err());
        assert!(SafePath::create_dir(&parent, "a/b", &root).is_err());
        assert!(SafePath::create_dir(&parent, "", &root).is_err());
    }

    #[test]
    fn create_dir_nested_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();

        let safe = SafePath::create_dir_nested("Artist/Album/Song", &root).unwrap();
        assert!(safe.is_dir());
        assert!(safe.as_path().ends_with("Song"));
        assert!(dir.path().join("Artist/Album/Song").exists());
    }

    #[test]
    fn create_dir_refuses_a_new_name_the_rules_refuse() {
        // #578: the web UI made song directories by names no new song may
        // have, and the song loader then skipped a hidden one.
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        let parent = root.as_safe_path();
        for name in [".hidden", " lead", "trail "] {
            assert!(
                SafePath::create_dir(&parent, name, &root).is_err(),
                "{name:?}"
            );
            assert!(!dir.path().join(name).exists(), "{name:?} created");
        }
        // One already there, from before the rules, is still used.
        std::fs::create_dir(dir.path().join(".warmup")).unwrap();
        assert!(SafePath::create_dir(&parent, ".warmup", &root).is_ok());
    }

    #[test]
    fn create_dir_nested_creates_nothing_for_a_bad_name_further_down() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        assert!(SafePath::create_dir_nested("Artist/.hidden", &root).is_err());
        assert!(!dir.path().join("Artist").exists());
        // Under a directory from before the rules, a good name is fine.
        std::fs::create_dir(dir.path().join(".old")).unwrap();
        assert!(SafePath::create_dir_nested(".old/Song", &root).is_ok());
    }

    #[test]
    fn create_dir_nested_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();

        assert!(SafePath::create_dir_nested("Artist/../../../etc", &root).is_err());
    }

    #[test]
    fn join_filename() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        let safe = root.as_safe_path();

        let joined = safe.join_filename("song.yaml");
        assert!(joined.ends_with("song.yaml"));
    }

    #[test]
    fn resolve_relative() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("songs");
        std::fs::create_dir(&sub).unwrap();

        let root = VerifiedRoot::new(dir.path()).unwrap();
        let safe = root.resolve("songs").unwrap();
        assert!(safe.is_dir());
    }

    #[test]
    fn validate_relative_existing() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("fixtures");
        std::fs::create_dir(&sub).unwrap();

        let root = VerifiedRoot::new(dir.path()).unwrap();
        let path = SafePath::validate_relative("fixtures", &root).unwrap();
        assert!(path.is_dir());
    }

    #[test]
    fn validate_relative_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        let path = SafePath::validate_relative("does_not_exist", &root).unwrap();
        // Returns the path without creating it
        assert!(!path.exists());
        assert!(path.ends_with("does_not_exist"));
    }

    /// A subdirectory linked out of the root is no way out, for a file
    /// there or one not there yet; nor is a dangling link.
    #[cfg(unix)]
    #[test]
    fn validate_relative_refuses_a_linked_way_out() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("there.venue"), "").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("out")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("gone"), dir.path().join("dangling"))
            .unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        for relative in ["out/there.venue", "out/new.venue", "dangling"] {
            assert!(
                SafePath::validate_relative(relative, &root).is_err(),
                "{relative}"
            );
        }
        std::fs::create_dir(dir.path().join("in")).unwrap();
        assert!(SafePath::validate_relative("in/new.venue", &root).is_ok());
    }

    /// The entry is the link itself, not what it points at, and a link to
    /// nothing inside the root is reported, not refused.
    #[cfg(unix)]
    #[test]
    fn validate_relative_entry_is_the_link_not_its_target() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.venue"), "").unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("main.venue"),
            dir.path().join("alias.venue"),
        )
        .unwrap();
        std::os::unix::fs::symlink(dir.path().join("gone.venue"), dir.path().join("dead.venue"))
            .unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();

        let alias = SafePath::validate_relative_entry("alias.venue", &root).unwrap();
        assert_eq!(alias.path(), root.as_path().join("alias.venue"));
        assert!(!alias.is_dangling_link());
        let dead = SafePath::validate_relative_entry("dead.venue", &root).unwrap();
        assert!(dead.is_dangling_link());
        assert!(SafePath::validate_relative("dead.venue", &root).is_err());
    }

    #[test]
    fn validate_relative_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        assert!(SafePath::validate_relative("../escape", &root).is_err());
        assert!(SafePath::validate_relative("/absolute", &root).is_err());
    }

    /// `..` is an escape as a path component, not as two characters in a
    /// name: the rule a playlist's paths are held to.
    #[test]
    fn a_dot_dot_inside_a_name_is_no_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        assert!(SafePath::validate_name("v1..2").is_ok());
        assert!(SafePath::validate_name("..").is_err());
        assert!(SafePath::validate_relative("v1..2/show.light", &root).is_ok());
        assert!(SafePath::validate_relative("v1/../show.light", &root).is_err());
        assert!(SafePath::create_dir_nested("Artist/v1..2", &root).is_ok());
        assert!(SafePath::create_dir_nested("Artist/..", &root).is_err());
    }

    /// `create_dir` holds a name to `validate_name`, so `.` names nothing.
    #[test]
    fn create_dir_rejects_the_current_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        let parent = root.as_safe_path();
        assert!(SafePath::create_dir(&parent, ".", &root).is_err());
    }
}
