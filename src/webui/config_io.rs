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

    let player = config::Player::deserialize(tmp.path()).map_err(|e| vec![format!("{}", e)])?;
    player.validate()?;

    Ok(())
}

/// Validates a light show DSL string by attempting to parse it.
pub fn validate_light_show(content: &str) -> Result<(), Vec<String>> {
    parse_light_shows(content).map_err(|e| vec![format!("{}", e)])?;
    Ok(())
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
