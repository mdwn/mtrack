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

//! What a file is to the player, by its extension: the one rule.
//!
//! A media or light show file's extension is read in any case (`Bass.WAV`
//! is an audio track, `Show.LIGHT` a light show). A YAML config's (a song,
//! playlist or profile) is read exactly as written, lowercase.

use std::path::Path;

use serde::Serialize;

/// The extensions of the audio files the player reads as tracks, lowercase.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "flac", "mp3", "ogg", "aac", "m4a", "mp4", "aiff", "aif",
];

/// The extensions of the MIDI files the player finds in a song directory
/// (playback, or a `dmx_` light show), lowercase.
pub const MIDI_EXTENSIONS: &[&str] = &["mid"];

/// The extensions of the DSL light show files the player finds in a song
/// directory, lowercase.
pub const LIGHTING_EXTENSIONS: &[&str] = &["light"];

/// The extensions of a YAML config file (a song, playlist or profile), as
/// written: lowercase only.
pub const YAML_EXTENSIONS: &[&str] = &["yaml", "yml"];

/// `path`'s extension, lowercased: what kind of file it is, whatever the
/// case it is written in.
pub fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
}

/// Whether `path`'s extension is one of `extensions` (lowercase), in any
/// case ([`extension_of`]).
pub fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    extension_of(path).is_some_and(|e| extensions.contains(&e.as_str()))
}

/// Whether `path`'s extension is one of `extensions` exactly as written,
/// case and all: the rule for a config file's name ([`is_yaml_config`]),
/// where a media or light show file's is read in any case
/// ([`has_extension`]).
pub fn has_exact_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.contains(&e))
}

/// Whether `path` is named as a YAML config file: `.yaml` or `.yml`,
/// lowercase ([`YAML_EXTENSIONS`]). `song.YAML` is not one.
pub fn is_yaml_config(path: &Path) -> bool {
    has_exact_extension(path, YAML_EXTENSIONS)
}

/// What a file is to the player, by its extension.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(into_wasm_abi))]
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
        if has_extension(path, AUDIO_EXTENSIONS) {
            FileKind::Audio
        } else if has_extension(path, MIDI_EXTENSIONS) {
            FileKind::Midi
        } else if has_extension(path, LIGHTING_EXTENSIONS) {
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
            for name in [
                format!("t.{ext}"),
                format!("t.{}", ext.to_uppercase()),
                format!("t.{}{}", ext[..1].to_uppercase(), &ext[1..]),
            ] {
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
        assert_eq!(FileKind::of(Path::new("a.LIGHT")), FileKind::Lighting);
        assert_eq!(FileKind::of(Path::new("a.Light")), FileKind::Lighting);
        for name in [
            "a.txt",
            "a.yaml",
            "a",
            "a.midi",
            ".wav",
            "wav",
            "a.wav.txt",
            "a.venue",
        ] {
            assert_eq!(FileKind::of(Path::new(name)), FileKind::Other, "{name}");
        }
    }

    #[test]
    fn extension_of_lowercases() {
        assert_eq!(extension_of(Path::new("Bass.WAV")).as_deref(), Some("wav"));
        assert_eq!(
            extension_of(Path::new("dir/a.b.Mid")).as_deref(),
            Some("mid")
        );
        assert_eq!(extension_of(Path::new("noext")), None);
        assert!(has_extension(Path::new("x.FLAC"), AUDIO_EXTENSIONS));
        assert!(!has_extension(Path::new("x.flac"), MIDI_EXTENSIONS));
    }

    #[test]
    fn yaml_configs_are_lowercase_only() {
        for name in ["song.yaml", "song.yml", "Set.yaml", "dir/p.yml"] {
            assert!(is_yaml_config(Path::new(name)), "{name}");
        }
        for name in [
            "song.YAML",
            "song.Yml",
            "song.yaml.bak",
            "song",
            "song.json",
        ] {
            assert!(!is_yaml_config(Path::new(name)), "{name}");
        }
        assert!(has_exact_extension(Path::new("a.yaml"), &["yaml"]));
        assert!(!has_exact_extension(Path::new("a.YAML"), &["yaml"]));
    }
}
