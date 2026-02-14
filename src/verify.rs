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

use std::collections::{BTreeMap, HashMap};

use crate::songs::{Song, Songs};

/// Severity level for a verification issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// A single verification issue found during checking.
#[derive(Debug, Clone)]
pub struct Issue {
    pub severity: Severity,
    pub category: &'static str,
    pub song_name: String,
    pub message: String,
}

/// Result of verifying a set of songs.
#[derive(Debug, Clone, Default)]
pub struct VerificationReport {
    pub issues: Vec<Issue>,
}

impl VerificationReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// Merge another report into this one.
    pub fn merge(&mut self, other: VerificationReport) {
        self.issues.extend(other.issues);
    }
}

/// Checks a single song's tracks against the provided track mappings.
/// Returns an issue for each track that has no entry in the mappings.
pub fn check_track_mappings(song: &Song, track_mappings: &HashMap<String, Vec<u16>>) -> Vec<Issue> {
    song.tracks()
        .iter()
        .filter(|track| !track_mappings.contains_key(track.name()))
        .map(|track| Issue {
            severity: Severity::Warning,
            category: "track-mappings",
            song_name: song.name().to_string(),
            message: format!("track \"{}\" has no entry in track_mappings", track.name()),
        })
        .collect()
}

/// Checks all songs in a registry against the provided track mappings.
pub fn check_all_track_mappings(
    songs: &Songs,
    track_mappings: &HashMap<String, Vec<u16>>,
) -> VerificationReport {
    let mut report = VerificationReport::default();
    for song in songs.sorted_list() {
        report
            .issues
            .extend(check_track_mappings(&song, track_mappings));
    }
    report
}

/// Logs warnings for any tracks in the song that are missing from the track mappings.
/// Intended to be called right before playback starts.
pub fn warn_unmapped_tracks(song: &Song, track_mappings: &HashMap<String, Vec<u16>>) {
    let unmapped: Vec<&str> = song
        .tracks()
        .iter()
        .filter(|track| !track_mappings.contains_key(track.name()))
        .map(|track| track.name())
        .collect();

    if !unmapped.is_empty() {
        tracing::warn!(
            song = song.name(),
            tracks = ?unmapped,
            "Song has {} track(s) with no track mapping; these tracks will be silent",
            unmapped.len()
        );
    }
}

/// Prints a verification report grouped by song name.
pub fn print_report(report: &VerificationReport, songs: &Songs) {
    if report.is_clean() {
        println!("\u{2705} All {} song(s) passed verification.", songs.len());
        return;
    }

    // Group issues by song name.
    let mut by_song: BTreeMap<&str, Vec<&Issue>> = BTreeMap::new();
    for issue in &report.issues {
        by_song.entry(&issue.song_name).or_default().push(issue);
    }

    let clean_count = songs
        .sorted_list()
        .iter()
        .filter(|song| !by_song.contains_key(song.name()))
        .count();

    for (song_name, issues) in &by_song {
        let has_errors = issues.iter().any(|i| i.severity == Severity::Error);
        let icon = if has_errors {
            "\u{274c}"
        } else {
            "\u{26a0}\u{fe0f} "
        };
        println!("{} {}", icon, song_name);
        for issue in issues {
            let severity_icon = match issue.severity {
                Severity::Warning => "\u{26a0}\u{fe0f} ",
                Severity::Error => "\u{274c}",
            };
            println!(
                "   {} [{}] {}",
                severity_icon, issue.category, issue.message
            );
        }
    }

    if clean_count > 0 {
        println!("\n\u{2705} {} song(s) passed all checks.", clean_count);
    }

    println!(
        "\nSummary: {} issue(s) found across {} song(s).",
        report.issues.len(),
        by_song.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_song(name: &str, track_names: &[&str]) -> Arc<Song> {
        Arc::new(Song::new_for_test(name, track_names))
    }

    fn make_mappings(names: &[&str]) -> HashMap<String, Vec<u16>> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_string(), vec![(i + 1) as u16]))
            .collect()
    }

    #[test]
    fn test_check_track_mappings_all_mapped() {
        let song = make_song("test-song", &["guitar", "bass"]);
        let mappings = make_mappings(&["guitar", "bass"]);
        let issues = check_track_mappings(&song, &mappings);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_check_track_mappings_some_unmapped() {
        let song = make_song("test-song", &["guitar", "bass", "click"]);
        let mappings = make_mappings(&["guitar", "bass"]);
        let issues = check_track_mappings(&song, &mappings);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].song_name, "test-song");
        assert!(issues[0].message.contains("click"));
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].category, "track-mappings");
    }

    #[test]
    fn test_check_track_mappings_none_mapped() {
        let song = make_song("test-song", &["guitar", "bass"]);
        let mappings = HashMap::new();
        let issues = check_track_mappings(&song, &mappings);
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn test_check_track_mappings_empty_song() {
        let song = make_song("empty-song", &[]);
        let mappings = make_mappings(&["guitar"]);
        let issues = check_track_mappings(&song, &mappings);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_check_track_mappings_extra_mappings_ok() {
        let song = make_song("test-song", &["guitar"]);
        let mappings = make_mappings(&["guitar", "bass", "click"]);
        let issues = check_track_mappings(&song, &mappings);
        assert!(issues.is_empty());
    }

    #[test]
    fn test_check_all_track_mappings() {
        let mut songs_map = HashMap::new();
        songs_map.insert(
            "song-a".to_string(),
            make_song("song-a", &["guitar", "bass"]),
        );
        songs_map.insert(
            "song-b".to_string(),
            make_song("song-b", &["guitar", "click"]),
        );
        let songs = Songs::new(songs_map);
        let mappings = make_mappings(&["guitar", "bass"]);
        let report = check_all_track_mappings(&songs, &mappings);
        // song-b has "click" unmapped
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].song_name, "song-b");
    }

    #[test]
    fn test_verification_report_is_clean() {
        let report = VerificationReport::default();
        assert!(report.is_clean());
        assert!(!report.has_errors());
    }

    #[test]
    fn test_verification_report_has_errors() {
        let mut report = VerificationReport::default();
        report.issues.push(Issue {
            severity: Severity::Warning,
            category: "test",
            song_name: "song".to_string(),
            message: "warning".to_string(),
        });
        assert!(!report.has_errors());

        report.issues.push(Issue {
            severity: Severity::Error,
            category: "test",
            song_name: "song".to_string(),
            message: "error".to_string(),
        });
        assert!(report.has_errors());
    }

    #[test]
    fn test_verification_report_merge() {
        let mut report_a = VerificationReport::default();
        report_a.issues.push(Issue {
            severity: Severity::Warning,
            category: "a",
            song_name: "song".to_string(),
            message: "issue a".to_string(),
        });
        let mut report_b = VerificationReport::default();
        report_b.issues.push(Issue {
            severity: Severity::Error,
            category: "b",
            song_name: "song".to_string(),
            message: "issue b".to_string(),
        });
        report_a.merge(report_b);
        assert_eq!(report_a.issues.len(), 2);
        assert!(report_a.has_errors());
    }
}
