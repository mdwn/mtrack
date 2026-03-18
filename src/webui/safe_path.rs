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
    /// The name must be a single path segment (no slashes).
    pub fn create_dir(
        parent: &SafePath,
        name: &str,
        root: &VerifiedRoot,
    ) -> Result<Self, SafePathError> {
        if name.is_empty()
            || name.contains("..")
            || name.contains('/')
            || name.contains('\\')
            || name.contains('\0')
        {
            return Err(SafePathError::InvalidName);
        }
        let target = parent.path.join(name);
        if !target.exists() {
            std::fs::create_dir(&target).map_err(SafePathError::Io)?;
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
    /// Rejects empty strings, "..", and path separators.
    pub fn validate_name(name: &str) -> Result<(), SafePathError> {
        if name.is_empty()
            || name == "."
            || name.contains("..")
            || name.contains('/')
            || name.contains('\\')
            || name.contains('\0')
        {
            return Err(SafePathError::InvalidName);
        }
        Ok(())
    }

    /// Validates a relative path under the root without creating anything.
    /// If the full path exists, it's resolved and verified. If it doesn't exist,
    /// each segment is checked for traversal (no "..", "\", null) and the
    /// constructed path is returned without filesystem modification.
    /// Use this for read-only operations where the directory may not exist.
    pub fn validate_relative(
        relative: &str,
        root: &VerifiedRoot,
    ) -> Result<PathBuf, SafePathError> {
        if relative.is_empty()
            || relative.contains("..")
            || relative.contains('\\')
            || relative.contains('\0')
        {
            return Err(SafePathError::InvalidName);
        }
        if std::path::Path::new(relative).is_absolute() {
            return Err(SafePathError::InvalidName);
        }
        let resolved = root.as_path().join(relative);
        if resolved.exists() {
            let safe = SafePath::resolve(&resolved, root)?;
            Ok(safe.path)
        } else {
            Ok(resolved)
        }
    }

    /// Creates a nested directory path (e.g. "Artist/Album/Song") segment by segment,
    /// verifying containment at each step.
    pub fn create_dir_nested(name: &str, root: &VerifiedRoot) -> Result<Self, SafePathError> {
        if name.is_empty() || name.contains("..") || name.contains('\\') || name.contains('\0') {
            return Err(SafePathError::InvalidName);
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

    #[test]
    fn validate_relative_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = VerifiedRoot::new(dir.path()).unwrap();
        assert!(SafePath::validate_relative("../escape", &root).is_err());
        assert!(SafePath::validate_relative("/absolute", &root).is_err());
    }
}
