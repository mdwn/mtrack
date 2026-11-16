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
//! Songs. Loading one and everything it holds is `mtrack-core`'s; what the
//! player adds is how a song is sent over gRPC.

pub use mtrack_core::songs::*;

use crate::proto::player;

/// A song as gRPC sends it.
pub trait SongProto {
    /// Returns a proto version of the song.
    fn to_proto(&self) -> Result<player::v1::Song, std::io::Error>;
}

impl SongProto for Song {
    fn to_proto(&self) -> Result<player::v1::Song, std::io::Error> {
        let summary = self.summary();
        let duration = match prost_types::Duration::try_from(summary.duration) {
            Ok(duration) => duration,
            Err(e) => return Err(std::io::Error::other(e.to_string())),
        };
        let beat_grid = self.beat_grid().map(|grid| player::v1::BeatGrid {
            beats: grid.beats.clone(),
            measure_starts: grid.measure_starts.iter().map(|&i| i as u32).collect(),
        });

        let sections = summary
            .sections
            .into_iter()
            .map(|s| player::v1::Section {
                name: s.name,
                start_measure: s.start_measure as u32,
                end_measure: s.end_measure as u32,
                start_beat: s.start_beat,
                end_beat: s.end_beat,
                color: s.color,
            })
            .collect();

        Ok(player::v1::Song {
            name: summary.name,
            duration: Some(duration),
            tracks: self.output_track_names(),
            beat_grid,
            sections,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use super::{Song, SongProto};
    use crate::audio::click_analysis::BeatGrid;
    use crate::config::Section;

    fn section(name: &str, start_measure: usize, end_measure: usize) -> Section {
        Section {
            name: name.to_string(),
            start_measure,
            end_measure,
            start_beat: None,
            end_beat: None,
            color: None,
        }
    }

    /// A song of two 4-beat measures (8 beats, measures at beats 0 and 4)
    /// with `sections`.
    fn song_with_beat_grid(sections: Vec<Section>) -> Song {
        Song::new_for_test_with_sections(
            "test",
            &["click"],
            BeatGrid {
                beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
                measure_starts: vec![0, 4],
            },
            sections,
        )
    }

    fn verse_and_chorus() -> Vec<Section> {
        vec![section("verse", 1, 2), section("chorus", 2, 3)]
    }

    #[test]
    fn song_to_proto() {
        let song =
            Song::new_for_test_lasting("Proto Song", &["kick", "snare"], Duration::from_secs(90));
        let proto = song.to_proto().unwrap();
        assert_eq!(proto.name, "Proto Song");
        assert_eq!(proto.tracks, vec!["kick", "snare"]);
        let dur = proto.duration.unwrap();
        assert_eq!(dur.seconds, 90);
        assert_eq!(dur.nanos, 0);
    }

    #[test]
    fn song_to_proto_zero_duration() {
        let song = Song::new_for_test("Zero", &[]);
        let proto = song.to_proto().unwrap();
        assert_eq!(proto.name, "Zero");
        assert!(proto.tracks.is_empty());
    }

    #[test]
    fn to_proto_conversion() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?;
        crate::testutil::write_wav(
            tempdir.path().join("track.wav"),
            vec![vec![1_i32; 4410]],
            44100,
        )?;
        let song_config = crate::config::Song::new(
            "proto test",
            None,
            None,
            None,
            None,
            None,
            vec![crate::config::Track::new(
                "track".to_string(),
                "track.wav",
                Some(1),
            )],
            std::collections::HashMap::new(),
            vec![],
        );
        let song = Song::new(tempdir.path(), &song_config)?;
        let proto = song.to_proto()?;
        assert_eq!(proto.name, "proto test");
        assert_eq!(proto.tracks.len(), 1);
        assert_eq!(proto.tracks[0], "track");
        Ok(())
    }

    #[test]
    fn to_proto_includes_sections() {
        let song = song_with_beat_grid(verse_and_chorus());
        let proto = song.to_proto().unwrap();
        assert_eq!(proto.sections.len(), 2);
        assert_eq!(proto.sections[0].name, "verse");
        assert_eq!(proto.sections[0].start_measure, 1);
        assert_eq!(proto.sections[0].end_measure, 2);
        assert_eq!(proto.sections[1].name, "chorus");
    }

    #[test]
    fn to_proto_carries_section_beat_offsets() {
        // Measures alone do not locate a boundary: the player resolves it via
        // `grid.beat_time(measure, beat)`, so a client given only measures
        // draws the section somewhere the player disagrees with — by up to a
        // whole measure. The offsets have to cross the wire.
        let song = song_with_beat_grid(vec![Section {
            name: "bridge".to_string(),
            start_measure: 2,
            end_measure: 4,
            start_beat: Some(3.0),
            end_beat: Some(2.5),
            color: Some("#ff8800".to_string()),
        }]);

        let proto = song.to_proto().unwrap();
        assert_eq!(proto.sections.len(), 1);
        assert_eq!(proto.sections[0].start_beat, Some(3.0));
        assert_eq!(proto.sections[0].end_beat, Some(2.5));
        assert_eq!(proto.sections[0].color.as_deref(), Some("#ff8800"));
    }

    #[test]
    fn to_proto_leaves_absent_beat_offsets_absent() {
        // Absent means "the measure line", which is not the same claim as
        // beat 1 and must not be materialised into one.
        let song = song_with_beat_grid(verse_and_chorus());
        let proto = song.to_proto().unwrap();
        assert_eq!(proto.sections[0].start_beat, None);
        assert_eq!(proto.sections[0].end_beat, None);
    }
}
