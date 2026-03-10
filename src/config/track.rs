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
use serde::{Deserialize, Serialize};

/// A YAML representation of a track.
#[derive(Deserialize, Clone, Serialize)]
pub struct Track {
    /// The name of the track.
    name: String,
    /// The file associated with the track.
    file: String,
    /// The file channel of the track to use.
    file_channel: Option<u16>,
}

impl Track {
    /// Creates a new track config.
    pub fn new(name: String, file: &str, file_channel: Option<u16>) -> Track {
        Track {
            name,
            file: file.to_string(),
            file_channel,
        }
    }

    /// Gets the name of the track.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the file associated with the track.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Gets the file channel of the track to use.
    pub fn file_channel(&self) -> Option<u16> {
        self.file_channel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, File, FileFormat};

    #[test]
    fn new_with_channel() {
        let t = Track::new("vocals".to_string(), "vocals.wav", Some(1));
        assert_eq!(t.name(), "vocals");
        assert_eq!(t.file(), "vocals.wav");
        assert_eq!(t.file_channel(), Some(1));
    }

    #[test]
    fn new_without_channel() {
        let t = Track::new("drums".to_string(), "drums.wav", None);
        assert_eq!(t.name(), "drums");
        assert_eq!(t.file(), "drums.wav");
        assert_eq!(t.file_channel(), None);
    }

    #[test]
    fn deserialize_with_channel() {
        let yaml = r#"
name: guitar
file: guitar.wav
file_channel: 2
"#;
        let t: Track = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(t.name(), "guitar");
        assert_eq!(t.file(), "guitar.wav");
        assert_eq!(t.file_channel(), Some(2));
    }

    #[test]
    fn deserialize_without_channel() {
        let yaml = r#"
name: bass
file: bass.wav
"#;
        let t: Track = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(t.name(), "bass");
        assert_eq!(t.file(), "bass.wav");
        assert_eq!(t.file_channel(), None);
    }

    #[test]
    fn clone_preserves_all_fields() {
        let t = Track::new("vocals".to_string(), "vocals.wav", Some(3));
        let cloned = t.clone();
        assert_eq!(cloned.name(), "vocals");
        assert_eq!(cloned.file(), "vocals.wav");
        assert_eq!(cloned.file_channel(), Some(3));
    }

    #[test]
    fn serialize_roundtrip() {
        let t = Track::new("keys".to_string(), "keys.wav", Some(5));
        let serialized = crate::util::to_yaml_string(&t).unwrap();
        let deserialized: Track = config::Config::builder()
            .add_source(config::File::from_str(
                &serialized,
                config::FileFormat::Yaml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(deserialized.name(), "keys");
        assert_eq!(deserialized.file(), "keys.wav");
        assert_eq!(deserialized.file_channel(), Some(5));
    }

    #[test]
    fn serialize_roundtrip_no_channel() {
        let t = Track::new("click".to_string(), "click.wav", None);
        let serialized = crate::util::to_yaml_string(&t).unwrap();
        let deserialized: Track = config::Config::builder()
            .add_source(config::File::from_str(
                &serialized,
                config::FileFormat::Yaml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(deserialized.name(), "click");
        assert_eq!(deserialized.file(), "click.wav");
        assert_eq!(deserialized.file_channel(), None);
    }
}
