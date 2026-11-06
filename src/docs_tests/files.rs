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
//! The docs' lists of file kinds against the player's: the audio formats
//! the web UI's docs name are the audio extensions the player reads.

use std::collections::BTreeSet;

use crate::songs::AUDIO_EXTENSIONS;
use crate::testutil::markdown::plain;

const WEB_UI_MD: &str = include_str!("../../docs/src/interfaces/web-ui.md");

/// The web UI's "Supported audio formats" line names each audio extension
/// the player reads (`songs::AUDIO_EXTENSIONS`), and no other: `WAV`,
/// `M4A/MP4`, `AIFF (.aiff or .aif)` read as the extensions they name.
#[test]
fn web_ui_audio_formats_are_the_players() {
    let line = WEB_UI_MD
        .lines()
        .find_map(|line| line.strip_prefix("Supported audio formats:"))
        .expect("web-ui.md states the supported audio formats");
    let named: BTreeSet<String> = plain(line)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty() && *word != "or")
        .map(str::to_lowercase)
        .collect();
    let players: BTreeSet<String> = AUDIO_EXTENSIONS.iter().map(|e| e.to_string()).collect();
    assert_eq!(named, players, "{line}");
}
