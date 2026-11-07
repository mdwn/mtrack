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

use std::path::Path;

use crate::config;
use crate::util;

/// Writes content to a config file, preserving its ownership and leaving a
/// recoverable copy if the write is interrupted. See [`crate::util::write_file`],
/// which this wraps for callers that report errors as strings.
pub fn staged_write(path: &Path, content: &str) -> Result<(), String> {
    write(path, content, false).map_err(|e| write_message(path, &e))
}

/// Writes `path` as [`staged_write`] does; with `new`, only creates it
/// (`util::write_new_file`): a file already there fails with
/// `ErrorKind::AlreadyExists` and is left as it was.
pub fn write(path: &Path, content: &str, new: bool) -> std::io::Result<()> {
    if new {
        util::write_new_file(path, content.as_bytes())
    } else {
        util::write_file(path, content.as_bytes())
    }
}

/// What a failed write of `path` says, with the hint for fixing it.
pub fn write_message(path: &Path, e: &std::io::Error) -> String {
    util::write_file_message(path, e)
}

/// Validates a player config YAML string by attempting to deserialize it.
pub fn validate_player_config(yaml: &str) -> Result<(), Vec<String>> {
    let player = config::Player::deserialize_from_str(yaml).map_err(|e| vec![format!("{}", e)])?;
    player.validate()?;

    Ok(())
}

/// Validates a light show DSL string by attempting to parse it.
///
/// `tempo` is the map the show will be loaded with — a song's `tempo:` block,
/// or one derived from its click track. Musical timing resolves at parse time,
/// so validating a tempo-inheriting show without it rejects content the player
/// loads without complaint. `path` is where the content is (or will be)
/// saved, which its `include`s resolve against; `root`, when given, is the
/// folder they must stay inside.
pub fn validate_light_show(
    content: &str,
    path: Option<&Path>,
    root: Option<&Path>,
    tempo: Option<&crate::tempo::TempoMap>,
) -> Result<(), Vec<String>> {
    crate::lighting::parser::parse_light_file_within(content, path, root, tempo)
        .map_err(|e| vec![format!("{}", e)])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_staged_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        staged_write(&path, "hello world").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_staged_write_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        staged_write(&path, "first").unwrap();
        staged_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn test_validate_light_show_valid() {
        let content = r#"
show "test" {
    @00:00.000
    lights: static color: "red", duration: 5s
}
"#;
        assert!(validate_light_show(content, None, None, None).is_ok());
    }

    #[test]
    fn test_validate_light_show_invalid() {
        let content = "this is not valid DSL content {{{";
        assert!(validate_light_show(content, None, None, None).is_err());
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
