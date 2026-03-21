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

//! Disk-backed cache for computed song data (waveform peaks, etc.).
//!
//! Stores a `.mtrack-cache.json` file in each song's directory. Entries are
//! keyed by audio filename and channel, with mtime+size used to detect when
//! source files have changed.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::webui::config_io::atomic_write;
use serde::{Deserialize, Serialize};

const CACHE_VERSION: u32 = 1;
const CACHE_FILENAME: &str = ".mtrack-cache.json";

#[derive(Serialize, Deserialize)]
struct SongCache {
    version: u32,
    tracks: HashMap<String, FileCacheEntry>,
}

#[derive(Serialize, Deserialize)]
struct FileCacheEntry {
    mtime_secs: u64,
    mtime_nanos: u32,
    size: u64,
    channels: HashMap<String, ChannelCache>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChannelCache {
    peaks: Vec<f32>,
}

/// Metadata from the filesystem used for cache invalidation.
struct FileMeta {
    mtime_secs: u64,
    mtime_nanos: u32,
    size: u64,
}

fn get_file_meta(path: &Path) -> Option<FileMeta> {
    let metadata = fs::metadata(path).ok()?;
    let mtime = metadata.modified().ok()?;
    let since_epoch = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(FileMeta {
        mtime_secs: since_epoch.as_secs(),
        mtime_nanos: since_epoch.subsec_nanos(),
        size: metadata.len(),
    })
}

fn meta_matches(entry: &FileCacheEntry, meta: &FileMeta) -> bool {
    entry.mtime_secs == meta.mtime_secs
        && entry.mtime_nanos == meta.mtime_nanos
        && entry.size == meta.size
}

fn filename_key(file: &Path) -> Option<String> {
    file.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

fn read_cache(song_dir: &Path) -> Option<SongCache> {
    let cache_path = song_dir.join(CACHE_FILENAME);
    let content = fs::read_to_string(&cache_path).ok()?;
    let cache: SongCache = serde_json::from_str(&content).ok()?;
    if cache.version != CACHE_VERSION {
        return None;
    }
    Some(cache)
}

/// Returns true if the song directory looks valid for caching (non-empty, exists).
fn is_valid_cache_dir(song_dir: &Path) -> bool {
    !song_dir.as_os_str().is_empty() && song_dir.is_dir()
}

/// Load cached peaks for a song's tracks. Returns a map of track_name to peaks
/// for tracks where the cache is valid (source file unchanged).
///
/// `tracks` is a slice of `(track_name, file_path, file_channel)`.
pub fn load_cached_peaks(
    song_dir: &Path,
    tracks: &[(String, PathBuf, u16)],
) -> HashMap<String, Vec<f32>> {
    let mut result = HashMap::new();

    if !is_valid_cache_dir(song_dir) {
        return result;
    }

    let cache = match read_cache(song_dir) {
        Some(c) => c,
        None => return result,
    };

    for (track_name, file, channel) in tracks {
        let key = match filename_key(file) {
            Some(k) => k,
            None => continue,
        };

        let entry = match cache.tracks.get(&key) {
            Some(e) => e,
            None => continue,
        };

        let meta = match get_file_meta(file) {
            Some(m) => m,
            None => continue,
        };

        if !meta_matches(entry, &meta) {
            continue;
        }

        let channel_key = channel.to_string();
        if let Some(ch_cache) = entry.channels.get(&channel_key) {
            result.insert(track_name.clone(), ch_cache.peaks.clone());
        }
    }

    result
}

/// Save computed peaks to the song's cache file. Merges with any existing
/// cached data for other tracks/channels.
///
/// `peaks` is a slice of `(track_name, file_path, file_channel, peak_data)`.
pub fn save_peaks(
    song_dir: &Path,
    peaks: &[(String, PathBuf, u16, Vec<f32>)],
) -> Result<(), String> {
    if !is_valid_cache_dir(song_dir) {
        return Ok(());
    }

    let mut cache = read_cache(song_dir).unwrap_or(SongCache {
        version: CACHE_VERSION,
        tracks: HashMap::new(),
    });

    for (_track_name, file, channel, peak_data) in peaks {
        let key = match filename_key(file) {
            Some(k) => k,
            None => continue,
        };

        let meta = match get_file_meta(file) {
            Some(m) => m,
            None => continue,
        };

        let entry = cache.tracks.entry(key).or_insert_with(|| FileCacheEntry {
            mtime_secs: meta.mtime_secs,
            mtime_nanos: meta.mtime_nanos,
            size: meta.size,
            channels: HashMap::new(),
        });

        // Update metadata in case the file was just recomputed after a change.
        entry.mtime_secs = meta.mtime_secs;
        entry.mtime_nanos = meta.mtime_nanos;
        entry.size = meta.size;

        let channel_key = channel.to_string();
        entry.channels.insert(
            channel_key,
            ChannelCache {
                peaks: peak_data.clone(),
            },
        );
    }

    cache.version = CACHE_VERSION;

    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| format!("Failed to serialize song cache: {}", e))?;

    let cache_path = song_dir.join(CACHE_FILENAME);
    atomic_write(&cache_path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_audio_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn load_returns_empty_when_no_cache_file() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "click.wav", b"audio data");
        let tracks = vec![("click".to_string(), file, 1u16)];

        let result = load_cached_peaks(dir.path(), &tracks);
        assert!(result.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "click.wav", b"audio data");
        let peaks = vec![0.1, 0.5, 1.0, 0.3];

        save_peaks(
            dir.path(),
            &[("click".to_string(), file.clone(), 1u16, peaks.clone())],
        )
        .unwrap();

        let tracks = vec![("click".to_string(), file, 1u16)];
        let result = load_cached_peaks(dir.path(), &tracks);
        assert_eq!(result.get("click").unwrap(), &peaks);
    }

    #[test]
    fn cache_invalidated_when_file_changes() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "click.wav", b"audio data");
        let peaks = vec![0.1, 0.5, 1.0];

        save_peaks(
            dir.path(),
            &[("click".to_string(), file.clone(), 1u16, peaks)],
        )
        .unwrap();

        // Modify the file (change size).
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&file, b"modified audio data that is longer").unwrap();

        let tracks = vec![("click".to_string(), file, 1u16)];
        let result = load_cached_peaks(dir.path(), &tracks);
        assert!(result.is_empty());
    }

    #[test]
    fn corrupt_cache_file_returns_empty() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "click.wav", b"audio data");
        fs::write(dir.path().join(CACHE_FILENAME), "not valid json{{{").unwrap();

        let tracks = vec![("click".to_string(), file, 1u16)];
        let result = load_cached_peaks(dir.path(), &tracks);
        assert!(result.is_empty());
    }

    #[test]
    fn version_mismatch_returns_empty() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "click.wav", b"audio data");
        let json = r#"{"version": 999, "tracks": {}}"#;
        fs::write(dir.path().join(CACHE_FILENAME), json).unwrap();

        let tracks = vec![("click".to_string(), file, 1u16)];
        let result = load_cached_peaks(dir.path(), &tracks);
        assert!(result.is_empty());
    }

    #[test]
    fn save_merges_with_existing_cache() {
        let dir = TempDir::new().unwrap();
        let file1 = create_test_audio_file(dir.path(), "click.wav", b"click data");
        let file2 = create_test_audio_file(dir.path(), "backing.flac", b"backing data");

        // Save first track.
        save_peaks(
            dir.path(),
            &[("click".to_string(), file1.clone(), 1u16, vec![0.1, 0.2])],
        )
        .unwrap();

        // Save second track.
        save_peaks(
            dir.path(),
            &[("backing".to_string(), file2.clone(), 1u16, vec![0.5, 0.6])],
        )
        .unwrap();

        // Both should be loadable.
        let tracks = vec![
            ("click".to_string(), file1, 1u16),
            ("backing".to_string(), file2, 1u16),
        ];
        let result = load_cached_peaks(dir.path(), &tracks);
        assert_eq!(result.get("click").unwrap(), &vec![0.1, 0.2]);
        assert_eq!(result.get("backing").unwrap(), &vec![0.5, 0.6]);
    }

    #[test]
    fn multiple_channels_same_file() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "stereo.wav", b"stereo data");

        save_peaks(
            dir.path(),
            &[
                ("stereo-l".to_string(), file.clone(), 1u16, vec![0.1, 0.2]),
                ("stereo-r".to_string(), file.clone(), 2u16, vec![0.8, 0.9]),
            ],
        )
        .unwrap();

        let tracks = vec![
            ("stereo-l".to_string(), file.clone(), 1u16),
            ("stereo-r".to_string(), file, 2u16),
        ];
        let result = load_cached_peaks(dir.path(), &tracks);
        assert_eq!(result.get("stereo-l").unwrap(), &vec![0.1, 0.2]);
        assert_eq!(result.get("stereo-r").unwrap(), &vec![0.8, 0.9]);
    }

    #[test]
    fn missing_audio_file_skipped() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("missing.wav");
        let tracks = vec![("missing".to_string(), nonexistent, 1u16)];

        let result = load_cached_peaks(dir.path(), &tracks);
        assert!(result.is_empty());
    }

    #[test]
    fn cache_file_created_with_pretty_json() {
        let dir = TempDir::new().unwrap();
        let file = create_test_audio_file(dir.path(), "click.wav", b"audio data");

        save_peaks(dir.path(), &[("click".to_string(), file, 1u16, vec![0.5])]).unwrap();

        let content = fs::read_to_string(dir.path().join(CACHE_FILENAME)).unwrap();
        assert!(content.contains('\n'));
        assert!(content.contains("\"version\": 1"));
    }
}
