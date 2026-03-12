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

/// Converts a string to kebab-case. Handles spaces, underscores, camelCase,
/// and PascalCase by inserting hyphens at word boundaries and lowercasing.
pub fn to_kebab_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_was_separator = false;

    for (i, &ch) in chars.iter().enumerate() {
        if ch == ' ' || ch == '_' || ch == '-' {
            if !result.is_empty() && !prev_was_separator {
                result.push('-');
            }
            prev_was_separator = true;
        } else if ch.is_uppercase() {
            // Insert hyphen before uppercase if preceded by a lowercase letter/digit,
            // or if preceded by uppercase followed by lowercase (e.g., "XMLParser" -> "xml-parser").
            if !result.is_empty() && !prev_was_separator {
                let prev = chars[i - 1];
                if prev.is_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_uppercase()
                        && chars.get(i + 1).is_some_and(|next| next.is_lowercase()))
                {
                    result.push('-');
                }
            }
            for lc in ch.to_lowercase() {
                result.push(lc);
            }
            prev_was_separator = false;
        } else {
            result.push(ch);
            prev_was_separator = false;
        }
    }

    // Strip trailing hyphen from trailing separators in input.
    while result.ends_with('-') {
        result.pop();
    }

    result
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

    #[test]
    fn kebab_case_spaces() {
        assert_eq!(super::to_kebab_case("Backing Track"), "backing-track");
    }

    #[test]
    fn kebab_case_underscores() {
        assert_eq!(super::to_kebab_case("backing_track"), "backing-track");
    }

    #[test]
    fn kebab_case_camel() {
        assert_eq!(super::to_kebab_case("backingTrack"), "backing-track");
    }

    #[test]
    fn kebab_case_pascal() {
        assert_eq!(super::to_kebab_case("BackingTrack"), "backing-track");
    }

    #[test]
    fn kebab_case_already_kebab() {
        assert_eq!(super::to_kebab_case("backing-track"), "backing-track");
    }

    #[test]
    fn kebab_case_mixed() {
        assert_eq!(
            super::to_kebab_case("My Cool_Song Name"),
            "my-cool-song-name"
        );
    }

    #[test]
    fn kebab_case_consecutive_separators() {
        assert_eq!(super::to_kebab_case("a  b__c--d"), "a-b-c-d");
    }

    #[test]
    fn kebab_case_all_caps() {
        assert_eq!(super::to_kebab_case("LOUD"), "loud");
    }

    #[test]
    fn kebab_case_acronym_then_word() {
        assert_eq!(super::to_kebab_case("XMLParser"), "xml-parser");
    }

    #[test]
    fn kebab_case_digit_before_upper() {
        assert_eq!(super::to_kebab_case("v2Track"), "v2-track");
        assert_eq!(super::to_kebab_case("Part2Guitars"), "part2-guitars");
    }

    #[test]
    fn kebab_case_non_ascii() {
        assert_eq!(super::to_kebab_case("café_backing"), "café-backing");
        assert_eq!(super::to_kebab_case("Élite Track"), "élite-track");
    }

    #[test]
    fn kebab_case_numbers_only() {
        assert_eq!(super::to_kebab_case("123"), "123");
    }

    #[test]
    fn kebab_case_trailing_leading_separators() {
        assert_eq!(super::to_kebab_case("_hello_"), "hello");
        assert_eq!(super::to_kebab_case("  spaced  "), "spaced");
    }
}
