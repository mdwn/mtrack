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

use std::io::Write;
use std::path::Path;

use crate::config;
use crate::lighting::parser::parse_light_shows;

/// Atomically writes content to a file by writing to a temporary file first,
/// then renaming it into place.
pub fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Cannot determine parent directory of {}", path.display()))?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("Failed to create temp file in {}: {}", parent.display(), e))?;

    tmp.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {}", e))?;

    tmp.persist(path)
        .map_err(|e| format!("Failed to rename temp file to {}: {}", path.display(), e))?;

    Ok(())
}

/// Validates a player config YAML string by attempting to deserialize it.
pub fn validate_player_config(yaml: &str) -> Result<(), Vec<String>> {
    // Write to a temp file so config::Player::deserialize can read it
    let tmp = tempfile::Builder::new()
        .suffix(".yaml")
        .tempfile()
        .map_err(|e| vec![format!("Failed to create temp file: {}", e)])?;

    std::fs::write(tmp.path(), yaml)
        .map_err(|e| vec![format!("Failed to write temp file: {}", e)])?;

    config::Player::deserialize(tmp.path()).map_err(|e| vec![format!("{}", e)])?;

    Ok(())
}

/// Validates a playlist config YAML string by attempting to deserialize it.
pub fn validate_playlist(yaml: &str) -> Result<(), Vec<String>> {
    let tmp = tempfile::Builder::new()
        .suffix(".yaml")
        .tempfile()
        .map_err(|e| vec![format!("Failed to create temp file: {}", e)])?;

    std::fs::write(tmp.path(), yaml)
        .map_err(|e| vec![format!("Failed to write temp file: {}", e)])?;

    config::Playlist::deserialize(tmp.path()).map_err(|e| vec![format!("{}", e)])?;

    Ok(())
}

/// Validates a light show DSL string by attempting to parse it.
pub fn validate_light_show(content: &str) -> Result<(), Vec<String>> {
    parse_light_shows(content).map_err(|e| vec![format!("{}", e)])?;
    Ok(())
}

/// Validates that a resolved path is under the given base directory.
/// Returns the canonical path if valid, or an error if the path would escape.
pub fn validate_path_within(base: &Path, requested: &Path) -> Result<std::path::PathBuf, String> {
    let canonical = requested
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path {}: {}", requested.display(), e))?;

    let canonical_base = base
        .canonicalize()
        .map_err(|e| format!("Cannot resolve base path {}: {}", base.display(), e))?;

    if !canonical.starts_with(&canonical_base) {
        return Err("Path outside allowed directory".to_string());
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, "hello world").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_atomic_write_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn test_validate_light_show_valid() {
        let content = r#"
show "test" {
    @00:00.000
    lights: static color: "red"
}
"#;
        assert!(validate_light_show(content).is_ok());
    }

    #[test]
    fn test_validate_light_show_invalid() {
        let content = "this is not valid DSL content {{{";
        assert!(validate_light_show(content).is_err());
    }

    #[test]
    fn test_validate_playlist_valid() {
        let yaml = "songs:\n  - song1\n  - song2\n";
        assert!(validate_playlist(yaml).is_ok());
    }

    #[test]
    fn test_validate_path_within() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = sub.join("test.txt");
        std::fs::write(&file, "test").unwrap();

        // Valid path within base
        assert!(validate_path_within(dir.path(), &file).is_ok());

        // Path traversal attempt
        let bad_path = dir
            .path()
            .join("sub")
            .join("..")
            .join("..")
            .join("etc")
            .join("passwd");
        assert!(validate_path_within(dir.path(), &bad_path).is_err());
    }

    #[test]
    fn test_validate_path_within_outside_base() {
        // Create two separate tempdirs so the canonical path exists but is
        // not under the base directory.
        let base_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let outside_file = other_dir.path().join("secret.txt");
        std::fs::write(&outside_file, "secret").unwrap();

        let result = validate_path_within(base_dir.path(), &outside_file);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Path outside allowed directory");
    }

    #[test]
    fn test_validate_player_config_valid() {
        let yaml = "songs: songs\n";
        assert!(validate_player_config(yaml).is_ok());
    }

    #[test]
    fn test_validate_player_config_invalid() {
        let yaml = "this is not valid yaml: [[[";
        assert!(validate_player_config(yaml).is_err());
    }
}
