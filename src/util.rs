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
use std::time::Duration;

use serde::Serialize;
use yaml_rust2::{Yaml, YamlEmitter};

/// Extracts a displayable file name from a path, returning a fallback if the name is unreadable.
pub fn filename_display(path: &Path) -> &str {
    path.file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("unreadable file name")
}

/// Outputs the given duration in a minutes:seconds format.
pub fn duration_minutes_seconds(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    let secs = duration.as_secs() - minutes * 60;
    format!("{}:{:02}", minutes, secs)
}

/// Serializes a value to a YAML string using serde_json as an intermediary and yaml-rust2 for
/// emission.
pub fn to_yaml_string<T: Serialize>(value: &T) -> Result<String, Box<dyn std::error::Error>> {
    let json_value = serde_json::to_value(value)?;
    let yaml = json_to_yaml(&json_value);
    let mut out = String::new();
    let mut emitter = YamlEmitter::new(&mut out);
    emitter.dump(&yaml)?;
    Ok(out)
}

fn json_to_yaml(value: &serde_json::Value) -> Yaml {
    match value {
        serde_json::Value::Null => Yaml::Null,
        serde_json::Value::Bool(b) => Yaml::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Yaml::Integer(i)
            } else {
                Yaml::Real(n.to_string())
            }
        }
        serde_json::Value::String(s) => Yaml::String(s.clone()),
        serde_json::Value::Array(arr) => Yaml::Array(arr.iter().map(json_to_yaml).collect()),
        serde_json::Value::Object(obj) => {
            let mut hash = yaml_rust2::yaml::Hash::new();
            for (k, v) in obj {
                hash.insert(Yaml::String(k.clone()), json_to_yaml(v));
            }
            Yaml::Hash(hash)
        }
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use crate::util::duration_minutes_seconds;

    #[test]
    fn test_duration_minutes_strings() {
        assert_eq!("0:00", duration_minutes_seconds(Duration::new(0, 0)));
        assert_eq!("0:05", duration_minutes_seconds(Duration::new(5, 0)));
        assert_eq!("0:55", duration_minutes_seconds(Duration::new(55, 0)));
        assert_eq!("1:00", duration_minutes_seconds(Duration::new(60, 0)));
        assert_eq!("2:05", duration_minutes_seconds(Duration::new(125, 0)));
        assert_eq!("60:06", duration_minutes_seconds(Duration::new(3606, 0)));
    }

    #[test]
    fn filename_display_normal() {
        use std::path::Path;
        assert_eq!(
            super::filename_display(Path::new("/home/user/song.wav")),
            "song.wav"
        );
    }

    #[test]
    fn filename_display_no_extension() {
        use std::path::Path;
        assert_eq!(
            super::filename_display(Path::new("/home/user/readme")),
            "readme"
        );
    }

    #[test]
    fn filename_display_just_filename() {
        use std::path::Path;
        assert_eq!(super::filename_display(Path::new("track.wav")), "track.wav");
    }

    #[test]
    fn filename_display_root_path() {
        use std::path::Path;
        // "/" has no file_name component
        assert_eq!(
            super::filename_display(Path::new("/")),
            "unreadable file name"
        );
    }

    #[test]
    fn filename_display_empty_path() {
        use std::path::Path;
        assert_eq!(
            super::filename_display(Path::new("")),
            "unreadable file name"
        );
    }
}
