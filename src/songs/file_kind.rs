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

//! What a file in a song directory is to the player, by its extension. The
//! extension's case does not matter: `Bass.WAV` is an audio track.

use std::path::Path;

use serde::Serialize;

/// The extensions of the audio files the player reads as tracks, lowercase.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "flac", "mp3", "ogg", "aac", "m4a", "mp4", "aiff", "aif",
];

/// The extensions of the MIDI files the player finds in a song directory
/// (playback, or a `dmx_` light show), lowercase.
pub const MIDI_EXTENSIONS: &[&str] = &["mid"];

/// What a file is to the player, by its extension.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Directory,
    Audio,
    Midi,
    Lighting,
    Other,
}

impl FileKind {
    /// The kind of the file at `path`, by its extension in any case. Never
    /// [`FileKind::Directory`]: the extension alone cannot say that.
    pub fn of(path: &Path) -> FileKind {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            FileKind::Audio
        } else if MIDI_EXTENSIONS.contains(&ext.as_str()) {
            FileKind::Midi
        } else if ext == "light" {
            FileKind::Lighting
        } else {
            FileKind::Other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_extensions_in_any_case() {
        for ext in AUDIO_EXTENSIONS {
            for name in [format!("t.{ext}"), format!("t.{}", ext.to_uppercase())] {
                assert_eq!(FileKind::of(Path::new(&name)), FileKind::Audio, "{name}");
            }
        }
        assert_eq!(FileKind::of(Path::new("Bass.WAV")), FileKind::Audio);
    }

    #[test]
    fn midi_lighting_and_other() {
        assert_eq!(FileKind::of(Path::new("a.mid")), FileKind::Midi);
        assert_eq!(FileKind::of(Path::new("a.MID")), FileKind::Midi);
        assert_eq!(FileKind::of(Path::new("a.light")), FileKind::Lighting);
        assert_eq!(FileKind::of(Path::new("a.Light")), FileKind::Lighting);
        for name in ["a.txt", "a.yaml", "a", "a.midi"] {
            assert_eq!(FileKind::of(Path::new(name)), FileKind::Other, "{name}");
        }
    }
}
