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
use indexmap::IndexMap;

use serde::{Deserialize, Serialize};

/// The mappings of tracks to output channels.
///
/// Uses `IndexMap` so insertion order is preserved through serialize/deserialize
/// round-trips — important for the web UI config editor.
#[derive(Deserialize, Serialize, Clone)]
pub struct TrackMappings {
    // The individual track mappings.
    #[serde(flatten)]
    pub track_mappings: IndexMap<String, Vec<u16>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, File, FileFormat};

    #[test]
    fn deserialize_basic() {
        let yaml = r#"
vocals: [1, 2]
drums: [3, 4]
"#;
        let tm: TrackMappings = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(tm.track_mappings.len(), 2);
        assert_eq!(tm.track_mappings["vocals"], vec![1, 2]);
        assert_eq!(tm.track_mappings["drums"], vec![3, 4]);
    }

    #[test]
    fn deserialize_single_channel() {
        let yaml = r#"
click: [5]
"#;
        let tm: TrackMappings = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(tm.track_mappings["click"], vec![5]);
    }

    #[test]
    fn deserialize_empty() {
        let yaml = "{}";
        let tm: TrackMappings = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert!(tm.track_mappings.is_empty());
    }

    #[test]
    fn deserialize_many_channels() {
        let yaml = r#"
main: [1, 2, 3, 4, 5, 6, 7, 8]
"#;
        let tm: TrackMappings = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(tm.track_mappings["main"], vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn deserialize_multiple_tracks() {
        let yaml = r#"
vocals: [1, 2]
drums: [3, 4]
bass: [5, 6]
keys: [7, 8]
click: [9]
cue: [10]
"#;
        let tm: TrackMappings = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(tm.track_mappings.len(), 6);
        assert_eq!(tm.track_mappings["bass"], vec![5, 6]);
        assert_eq!(tm.track_mappings["click"], vec![9]);
    }

    #[test]
    fn serialize_roundtrip() {
        let mut track_mappings = IndexMap::new();
        track_mappings.insert("vocals".to_string(), vec![1u16, 2]);
        track_mappings.insert("drums".to_string(), vec![3u16, 4]);
        let tm = TrackMappings { track_mappings };

        let serialized = crate::util::to_yaml_string(&tm).unwrap();
        let deserialized: TrackMappings = config::Config::builder()
            .add_source(config::File::from_str(
                &serialized,
                config::FileFormat::Yaml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(deserialized.track_mappings["vocals"], vec![1u16, 2]);
        assert_eq!(deserialized.track_mappings["drums"], vec![3u16, 4]);
    }
}
