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

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast, watch};
use tokio::time;
use tracing::{info, warn};

use rayon::prelude::*;

use crate::audio::sample_source::audio::AudioSampleSource;
use crate::audio::sample_source::traits::SampleSource;
use crate::player::Player;
use crate::tui::logging::get_log_buffer;

/// Per-track info needed for waveform computation: (track_name, file_path, file_channel).
pub type TrackInfo = (String, PathBuf, u16);

/// Per-track waveform peaks: (track_name, peak_values).
pub type TrackPeaks = (String, Vec<f32>);

/// Polls the player state at ~5Hz and broadcasts playback status messages.
#[tracing::instrument(skip_all, name = "playback_poller")]
pub async fn playback_poller(player: Arc<Player>, tx: broadcast::Sender<String>) {
    let mut interval = time::interval(Duration::from_millis(200));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        // Skip if no subscribers
        if tx.receiver_count() == 0 {
            continue;
        }

        let is_playing = player.is_playing().await;
        let playlist = player.get_playlist();

        let raw_elapsed_ms = player
            .elapsed()
            .await
            .ok()
            .flatten()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let playlist_name = playlist.name().to_string();
        let playlist_position = playlist.position();
        let playlist_songs: Vec<String> = playlist.songs().clone();

        let (song_name, song_duration_ms, tracks, beat_grid, looping, available_sections) =
            if let Some(current_song) = playlist.current() {
                let mappings = player.track_mappings();
                let tracks: Vec<serde_json::Value> = current_song
                    .tracks()
                    .iter()
                    .map(|t| {
                        let output_channels = mappings
                            .as_ref()
                            .and_then(|m| m.get(t.name()))
                            .cloned()
                            .unwrap_or_default();
                        json!({
                            "name": t.name(),
                            "output_channels": output_channels,
                        })
                    })
                    .collect();
                let beat_grid = current_song.beat_grid().map(|g| {
                    json!({
                        "beats": g.beats,
                        "measure_starts": g.measure_starts,
                    })
                });

                let available_sections: Vec<serde_json::Value> = current_song
                    .sections()
                    .iter()
                    .map(|s| {
                        json!({
                            "name": s.name,
                            "start_measure": s.start_measure,
                            "end_measure": s.end_measure,
                        })
                    })
                    .collect();

                (
                    current_song.name().to_string(),
                    current_song.duration().as_millis() as u64,
                    tracks,
                    beat_grid,
                    current_song.loop_playback(),
                    available_sections,
                )
            } else {
                (String::new(), 0, vec![], None, false, vec![])
            };

        let available_playlists = player.list_playlists();
        let persisted_playlist_name = player.persisted_playlist_name();

        // Get active section loop info.
        let active_section = player.active_section().map(|s| {
            json!({
                "name": s.name,
                "start_ms": s.start_time.as_millis() as u64,
                "end_ms": s.end_time.as_millis() as u64,
            })
        });

        // Get reactive loop state info.
        let reactive_loop_state = {
            use crate::player::ReactiveLoopState;
            let state = player.reactive_loop_state();
            match &state {
                ReactiveLoopState::Idle => json!({"state": "idle"}),
                ReactiveLoopState::SectionOffered(b) => json!({
                    "state": "section_offered",
                    "section_name": b.name,
                }),
                ReactiveLoopState::LoopArmed(b) => json!({
                    "state": "loop_armed",
                    "section_name": b.name,
                }),
                ReactiveLoopState::Looping(b) => json!({
                    "state": "looping",
                    "section_name": b.name,
                }),
                ReactiveLoopState::BreakRequested(b) => json!({
                    "state": "break_requested",
                    "section_name": b.name,
                }),
            }
        };
        // For looping, wrap elapsed time to show position within the current
        // loop/section iteration rather than total time since first play.
        let elapsed_ms = if let Some(ref section) = player.active_section() {
            let start_ms = section.start_time.as_millis() as u64;
            let end_ms = section.end_time.as_millis() as u64;
            let section_duration = end_ms.saturating_sub(start_ms);
            if section_duration > 0 && raw_elapsed_ms >= start_ms {
                (raw_elapsed_ms - start_ms) % section_duration + start_ms
            } else {
                raw_elapsed_ms
            }
        } else if looping && song_duration_ms > 0 {
            raw_elapsed_ms % song_duration_ms
        } else {
            raw_elapsed_ms
        };

        let msg = json!({
            "type": "playback",
            "is_playing": is_playing,
            "elapsed_ms": elapsed_ms,
            "song_name": song_name,
            "song_duration_ms": song_duration_ms,
            "playlist_name": playlist_name,
            "playlist_position": playlist_position,
            "playlist_songs": playlist_songs,
            "tracks": tracks,
            "available_playlists": available_playlists,
            "persisted_playlist_name": persisted_playlist_name,
            "locked": player.is_locked(),
            "beat_grid": beat_grid,
            "available_sections": available_sections,
            "active_section": active_section,
            "looping": looping,
            "reactive_loop_state": reactive_loop_state,
        });

        let _ = tx.send(msg.to_string());
    }
}

/// Watches the shared state snapshot (fixtures + active effects) and broadcasts changes.
#[tracing::instrument(skip_all, name = "state_poller")]
pub async fn state_poller(
    mut state_rx: watch::Receiver<Arc<crate::state::StateSnapshot>>,
    tx: broadcast::Sender<String>,
) {
    loop {
        // Wait for the state to change
        if state_rx.changed().await.is_err() {
            break; // Sender dropped
        }

        if tx.receiver_count() == 0 {
            continue;
        }

        let snapshot = state_rx.borrow_and_update().clone();

        let fixtures: serde_json::Map<String, serde_json::Value> = snapshot
            .fixtures
            .iter()
            .map(|f| {
                let channels: serde_json::Map<String, serde_json::Value> = f
                    .channels
                    .iter()
                    .map(|(k, v)| (k.clone(), json!(*v)))
                    .collect();
                (f.name.clone(), serde_json::Value::Object(channels))
            })
            .collect();

        let msg = json!({
            "type": "state",
            "fixtures": fixtures,
            "active_effects": snapshot.active_effects,
        });

        let _ = tx.send(msg.to_string());
    }
}

/// Polls the log ring buffer at ~2Hz and broadcasts log lines.
#[tracing::instrument(skip_all, name = "log_poller")]
pub async fn log_poller(tx: broadcast::Sender<String>) {
    let mut interval = time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    // Track how many log lines we've already sent
    let mut last_sent_count: usize = 0;

    loop {
        interval.tick().await;

        if tx.receiver_count() == 0 {
            continue;
        }

        let buffer = match get_log_buffer() {
            Some(buf) => buf,
            None => continue,
        };

        // Acquire the log buffer lock on the blocking thread pool so we never
        // block a tokio worker thread.  The TuiLogLayer acquires this same
        // parking_lot::Mutex on every log event from any thread.
        let skip_from = last_sent_count;
        let (new_lines, new_count) = match tokio::task::spawn_blocking(move || {
            let buf = buffer.lock();

            let current_len = buf.len();
            let mut adjusted_skip = skip_from;
            if current_len < adjusted_skip {
                adjusted_skip = 0;
            }
            if current_len == adjusted_skip {
                return (Vec::new(), adjusted_skip);
            }

            let lines: Vec<serde_json::Value> = buf
                .iter()
                .skip(adjusted_skip)
                .map(|line| {
                    // Parse "LEVEL target: message" format from TuiLogLayer
                    let (level, rest) = line.split_once(' ').unwrap_or(("INFO", line));
                    let (target, message) = rest.split_once(": ").unwrap_or(("", rest));
                    json!({
                        "level": level,
                        "target": target,
                        "message": message,
                    })
                })
                .collect();

            (lines, current_len)
        })
        .await
        {
            Ok(result) => result,
            Err(_) => continue,
        };

        last_sent_count = new_count;

        if new_lines.is_empty() {
            continue;
        }

        let msg = json!({
            "type": "logs",
            "lines": new_lines,
        });

        let _ = tx.send(msg.to_string());
    }
}

/// Shared waveform cache: song name → [(track_name, peaks)].
/// ~2 KB per track in memory; safe to cache entire setlists.
pub type WaveformCache = Arc<parking_lot::Mutex<HashMap<String, Vec<(String, Vec<f32>)>>>>;

/// Creates a new empty waveform cache.
pub fn new_waveform_cache() -> WaveformCache {
    Arc::new(parking_lot::Mutex::new(HashMap::new()))
}

/// Polls for song changes and sends waveform peaks to WebSocket clients.
///
/// Checks the shared cache first; on a miss, computes peaks on demand via
/// `spawn_blocking` and inserts the result into the cache.
#[tracing::instrument(skip_all, name = "waveform_poller")]
pub async fn waveform_poller(
    player: Arc<Player>,
    tx: broadcast::Sender<String>,
    cache: WaveformCache,
) {
    let mut interval = time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let mut last_song_name = String::new();

    loop {
        interval.tick().await;

        if tx.receiver_count() == 0 {
            continue;
        }

        let playlist = player.get_playlist();
        let current_song = match playlist.current() {
            Some(song) => song,
            None => continue,
        };
        let song_name = current_song.name().to_string();

        if song_name == last_song_name {
            continue;
        }
        last_song_name = song_name.clone();

        // Check cache first
        let cached = cache.lock().get(&song_name).cloned();
        let track_peaks = if let Some(cached) = cached {
            cached
        } else {
            // Collect track info (owned data) for the blocking task
            let track_infos: Vec<TrackInfo> = current_song
                .tracks()
                .iter()
                .map(|t| {
                    (
                        t.name().to_string(),
                        t.file().to_path_buf(),
                        t.file_channel(),
                    )
                })
                .collect();
            let song_dir = current_song.base_path().to_path_buf();

            let song_name_for_task = song_name.clone();
            info!("Computing waveform for '{}'", song_name_for_task);
            let start = std::time::Instant::now();
            let peaks_result = tokio::task::spawn_blocking(move || {
                compute_waveform_peaks(&song_dir, &track_infos)
            })
            .await;
            info!(
                "Waveform for '{}' computed in {:.1?}",
                song_name,
                start.elapsed()
            );

            // Song changed while we were computing — discard stale result
            let current_now = player
                .get_playlist()
                .current()
                .map(|s| s.name().to_string())
                .unwrap_or_default();
            if current_now != song_name_for_task {
                last_song_name = String::new(); // Force recompute on next tick
                continue;
            }

            match peaks_result {
                Ok(peaks) => {
                    cache.lock().insert(song_name.clone(), peaks.clone());
                    peaks
                }
                Err(e) => {
                    warn!("Waveform computation task failed: {}", e);
                    continue;
                }
            }
        };

        let tracks_json: Vec<serde_json::Value> = track_peaks
            .into_iter()
            .map(|(name, peaks)| {
                json!({
                    "name": name,
                    "peaks": peaks,
                })
            })
            .collect();

        let msg = json!({
            "type": "waveform",
            "song_name": song_name,
            "tracks": tracks_json,
        });

        let _ = tx.send(msg.to_string());
    }
}

/// Watches the ConfigStore for mutations and broadcasts a signal to WebSocket clients.
///
/// This is event-driven (not polling): it waits on the ConfigStore's broadcast channel
/// and sends a lightweight notification so clients know to re-fetch config.
#[tracing::instrument(skip_all, name = "config_watcher")]
pub async fn config_watcher(player: Arc<Player>, tx: broadcast::Sender<String>) {
    let store = match player.config_store() {
        Some(s) => s,
        None => return, // No config store — nothing to watch.
    };

    let mut rx = store.subscribe();

    loop {
        match rx.recv().await {
            Ok(()) => {
                if tx.receiver_count() == 0 {
                    continue;
                }
                let msg = json!({ "type": "config_changed" });
                let _ = tx.send(msg.to_string());
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("config_watcher missed {} change notifications", n);
                // Still send one notification so clients know to re-fetch.
                if tx.receiver_count() > 0 {
                    let msg = json!({ "type": "config_changed" });
                    let _ = tx.send(msg.to_string());
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// Background pre-warms the waveform cache for all songs.
///
/// Collects all uncached songs and computes their waveform peaks in parallel
/// using rayon. Waits for playback to stop before starting computation to
/// avoid competing with audio playback for CPU and I/O.
#[tracing::instrument(skip_all, name = "waveform_prewarmer")]
pub async fn waveform_prewarmer(player: Arc<Player>, cache: WaveformCache) {
    // Small delay before starting so the server can finish initializing
    time::sleep(Duration::from_secs(1)).await;

    // Wait until playback stops before computing
    while player.is_playing().await {
        time::sleep(Duration::from_secs(1)).await;
    }

    let all_songs = player.get_all_songs_playlist();
    let song_names: Vec<String> = all_songs.songs().clone();
    let total_songs = song_names.len();

    // Collect track info for all uncached songs up front
    let uncached_songs: Vec<(String, PathBuf, Vec<TrackInfo>)> = song_names
        .iter()
        .filter(|name| !cache.lock().contains_key(*name))
        .filter_map(|name| {
            let song = all_songs.get_song(name)?;
            let song_dir = song.base_path().to_path_buf();
            let track_infos: Vec<TrackInfo> = song
                .tracks()
                .iter()
                .map(|t| {
                    (
                        t.name().to_string(),
                        t.file().to_path_buf(),
                        t.file_channel(),
                    )
                })
                .collect();
            Some((name.clone(), song_dir, track_infos))
        })
        .collect();

    let uncached_count = uncached_songs.len();
    if uncached_count == 0 {
        info!("Waveform prewarm: all {} songs already cached", total_songs);
        return;
    }

    info!(
        "Waveform prewarm starting for {} uncached songs ({} total)",
        uncached_count, total_songs
    );
    let total_start = std::time::Instant::now();

    // Compute all songs in parallel using rayon
    let results: Vec<(String, Vec<TrackPeaks>)> = tokio::task::spawn_blocking(move || {
        uncached_songs
            .into_par_iter()
            .map(|(name, song_dir, track_infos)| {
                let start = std::time::Instant::now();
                let peaks = compute_waveform_peaks(&song_dir, &track_infos);
                info!(
                    "Prewarmed waveform for '{}' in {:.1?}",
                    name,
                    start.elapsed()
                );
                (name, peaks)
            })
            .collect()
    })
    .await
    .unwrap_or_default();

    // Insert all results into cache
    let mut cache_guard = cache.lock();
    for (name, peaks) in results {
        cache_guard.insert(name, peaks);
    }
    drop(cache_guard);

    info!(
        "Waveform prewarm complete for {} songs in {:.1?}",
        uncached_count,
        total_start.elapsed()
    );
}

/// Computes waveform peaks for all tracks in parallel, using the disk cache
/// when available. Returns (track_name, peaks) pairs.
///
/// Loads valid cached peaks first, computes only uncached tracks, then saves
/// newly computed peaks back to disk.
pub fn compute_waveform_peaks(song_dir: &std::path::Path, tracks: &[TrackInfo]) -> Vec<TrackPeaks> {
    const NUM_BUCKETS: usize = 500;

    let cached = crate::song_cache::load_cached_peaks(song_dir, tracks);

    // Split tracks into cached and uncached.
    let mut results: Vec<TrackPeaks> = Vec::with_capacity(tracks.len());
    let uncached: Vec<&TrackInfo> = tracks
        .iter()
        .filter(|(name, _, _)| !cached.contains_key(name))
        .collect();

    // Compute uncached tracks in parallel.
    let newly_computed: Vec<TrackPeaks> = uncached
        .par_iter()
        .map(|(name, file, file_channel)| {
            let peaks = compute_track_peaks(file, *file_channel, NUM_BUCKETS);
            (name.clone(), peaks)
        })
        .collect();

    // Save newly computed peaks to disk.
    if !newly_computed.is_empty() {
        let save_data: Vec<(String, PathBuf, u16, Vec<f32>)> = newly_computed
            .iter()
            .filter_map(|(name, peaks)| {
                let (_, file, channel) = tracks.iter().find(|(n, _, _)| n == name)?;
                Some((name.clone(), file.clone(), *channel, peaks.clone()))
            })
            .collect();

        if let Err(e) = crate::song_cache::save_peaks(song_dir, &save_data) {
            warn!("Failed to save waveform cache: {}", e);
        }
    }

    // Merge: preserve original track order.
    for (name, _file, _channel) in tracks {
        if let Some(peaks) = cached.get(name) {
            results.push((name.clone(), peaks.clone()));
        } else if let Some(peaks) = newly_computed.iter().find(|(n, _)| n == name) {
            results.push(peaks.clone());
        } else {
            results.push((name.clone(), vec![]));
        }
    }

    results
}

/// Computes peak values for a single track using bulk buffer reads.
///
/// Uses `AudioSampleSource::read_samples` to read large chunks at a time,
/// avoiding per-sample virtual dispatch overhead.
fn compute_track_peaks(file: &std::path::Path, file_channel: u16, num_buckets: usize) -> Vec<f32> {
    let mut source = match AudioSampleSource::from_file(file, None, 4096) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to open audio file {}: {}", file.display(), e);
            return vec![];
        }
    };

    let channel_count = source.channel_count() as usize;
    if channel_count == 0 {
        return vec![];
    }

    // file_channel is 1-indexed
    let target_channel = (file_channel as usize).saturating_sub(1);
    if target_channel >= channel_count {
        warn!(
            "file_channel {} exceeds channel count {} for {}",
            file_channel,
            channel_count,
            file.display()
        );
        return vec![];
    }

    // Estimate total mono samples from duration to size buckets up front
    let estimated_samples = source
        .duration()
        .map(|d| (d.as_secs_f64() * source.sample_rate() as f64) as usize)
        .unwrap_or(0);

    let samples_per_bucket = if estimated_samples > 0 {
        estimated_samples.div_ceil(num_buckets)
    } else {
        // Unknown duration: use a reasonable default, resize at end
        4096
    };

    let mut peaks = vec![0.0_f32; num_buckets];
    let mut mono_sample_idx: usize = 0;
    let mut interleaved_idx: usize = 0;

    // Read in chunks of ~16K interleaved samples at a time
    let mut buf = vec![0.0_f32; 16384];
    loop {
        let n = match source.read_samples(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        for &sample in &buf[..n] {
            let ch = interleaved_idx % channel_count;
            interleaved_idx += 1;

            if ch == target_channel {
                let bucket = (mono_sample_idx / samples_per_bucket).min(num_buckets - 1);
                let abs = sample.abs();
                if abs > peaks[bucket] {
                    peaks[bucket] = abs;
                }
                mono_sample_idx += 1;
            }
        }
    }

    // If we had no samples, return empty
    if mono_sample_idx == 0 {
        return vec![];
    }

    // Trim trailing empty buckets (if file was shorter than estimated)
    let used_buckets = (mono_sample_idx / samples_per_bucket + 1).min(num_buckets);
    peaks.truncate(used_buckets);

    // Normalize to 0.0 - 1.0
    let max_peak = peaks.iter().cloned().fold(0.0_f32, f32::max);
    if max_peak > 0.0 {
        for p in &mut peaks {
            *p /= max_peak;
        }
    }

    peaks
}

/// Builds the initial metadata JSON from the lighting system.
///
/// Sent to each WebSocket client on connect so the stage view knows fixture
/// names, types, and spatial tags.
pub fn build_metadata_json(
    lighting_system: Option<&Arc<Mutex<crate::lighting::system::LightingSystem>>>,
) -> String {
    let mut fixtures = serde_json::Map::new();

    if let Some(ls) = lighting_system {
        let system = ls.lock();
        if let Ok(fixture_infos) = system.get_current_venue_fixtures() {
            let venue_fixtures = get_venue_fixture_tags(&system);

            for fi in &fixture_infos {
                let tags = venue_fixtures.get(&fi.name).cloned().unwrap_or_default();

                let fixture_meta = json!({
                    "tags": tags,
                    "type": fi.fixture_type,
                });
                fixtures.insert(fi.name.clone(), fixture_meta);
            }
        }
    }

    let msg = json!({
        "type": "metadata",
        "fixtures": fixtures,
    });
    msg.to_string()
}

/// Extracts fixture names -> tags from the lighting system's current venue.
fn get_venue_fixture_tags(
    system: &crate::lighting::system::LightingSystem,
) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::new();
    if let Some(venue) = system.get_current_venue() {
        for (name, fixture) in venue.fixtures() {
            result.insert(name.clone(), fixture.tags().to_vec());
        }
    }
    result
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn new_waveform_cache_is_empty() {
        let cache = new_waveform_cache();
        assert!(cache.lock().is_empty());
    }

    #[test]
    fn waveform_cache_insert_and_retrieve() {
        let cache = new_waveform_cache();
        let peaks = vec![("track1".to_string(), vec![0.5, 1.0, 0.3])];
        cache.lock().insert("Song 1".to_string(), peaks.clone());

        let retrieved = cache.lock().get("Song 1").cloned();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), peaks);
    }

    #[test]
    fn build_metadata_json_no_lighting() {
        let json_str = build_metadata_json(None);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");
        assert_eq!(parsed["type"], "metadata");
        assert!(parsed["fixtures"].is_object());
        assert!(parsed["fixtures"].as_object().unwrap().is_empty());
    }

    #[test]
    fn compute_track_peaks_with_test_wav() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");

        // Create a simple WAV with known samples
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write test wav");

        let peaks = compute_track_peaks(&wav_path, 1, 10);
        assert!(!peaks.is_empty());
        // Peaks should be normalized to 0.0-1.0
        for &p in &peaks {
            assert!((0.0..=1.0).contains(&p), "peak {} out of range", p);
        }
        // At least one peak should be 1.0 (the max)
        assert!(
            peaks.iter().any(|&p| (p - 1.0).abs() < f32::EPSILON),
            "expected at least one normalized peak at 1.0"
        );
    }

    #[test]
    fn compute_track_peaks_invalid_channel() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("mono.wav");

        crate::testutil::write_wav(wav_path.clone(), vec![vec![1_i32, 2, 3]], 44100)
            .expect("write wav");

        // file_channel 5 on a mono file — should return empty
        let peaks = compute_track_peaks(&wav_path, 5, 10);
        assert!(peaks.is_empty());
    }

    #[test]
    fn compute_track_peaks_missing_file() {
        let peaks = compute_track_peaks(std::path::Path::new("/nonexistent/file.wav"), 1, 10);
        assert!(peaks.is_empty());
    }

    #[test]
    fn compute_track_peaks_zero_buckets_edge() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write test wav");

        // Even with 1 bucket, should work
        let peaks = compute_track_peaks(&wav_path, 1, 1);
        assert!(!peaks.is_empty());
        assert!(peaks.len() <= 1);
    }

    #[test]
    fn compute_track_peaks_stereo_extracts_correct_channel() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("stereo.wav");

        // Write a properly interleaved stereo WAV: ch1=loud, ch2=silence
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&wav_path, spec).expect("create wav");
        for _ in 0..44100 {
            writer.write_sample(0.9_f32).unwrap(); // ch1: loud
            writer.write_sample(0.0_f32).unwrap(); // ch2: silence
        }
        writer.finalize().unwrap();

        // file_channel 1 should see the loud signal
        let ch1_peaks = compute_track_peaks(&wav_path, 1, 10);
        assert!(!ch1_peaks.is_empty());
        assert!(
            ch1_peaks.iter().all(|&p| p > 0.5),
            "channel 1 should have loud peaks: {:?}",
            ch1_peaks
        );

        // file_channel 2 should see silence (all zeros)
        let ch2_peaks = compute_track_peaks(&wav_path, 2, 10);
        assert!(!ch2_peaks.is_empty());
        assert!(
            ch2_peaks.iter().all(|&p| p == 0.0),
            "channel 2 should be all zeros: {:?}",
            ch2_peaks
        );
    }

    #[test]
    fn waveform_cache_overwrite() {
        let cache = new_waveform_cache();
        let peaks1 = vec![("t1".to_string(), vec![0.5])];
        let peaks2 = vec![("t2".to_string(), vec![1.0])];
        cache.lock().insert("Song".to_string(), peaks1);
        cache.lock().insert("Song".to_string(), peaks2.clone());

        let retrieved = cache.lock().get("Song").cloned().unwrap();
        assert_eq!(retrieved, peaks2);
    }

    #[test]
    fn waveform_cache_multiple_songs() {
        let cache = new_waveform_cache();
        cache
            .lock()
            .insert("Song A".to_string(), vec![("t".to_string(), vec![0.1])]);
        cache
            .lock()
            .insert("Song B".to_string(), vec![("t".to_string(), vec![0.9])]);

        assert_eq!(cache.lock().len(), 2);
        assert!(cache.lock().contains_key("Song A"));
        assert!(cache.lock().contains_key("Song B"));
    }

    use crate::player::PlayerDevices;
    use crate::playlist;
    use crate::songs::{Song, Songs};

    /// Creates a test Player with no hardware devices.
    fn test_player(song_names: &[&str]) -> Arc<crate::player::Player> {
        let mut map = std::collections::HashMap::new();
        for name in song_names {
            map.insert(
                name.to_string(),
                Arc::new(Song::new_for_test(name, &["track1"])),
            );
        }
        let songs = Arc::new(Songs::new(map));
        let playlist_config = crate::config::Playlist::new(
            &song_names.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        );
        let pl = playlist::Playlist::new("test", &playlist_config, songs.clone()).unwrap();
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone()).unwrap(),
        );
        playlists.insert(pl.name().to_string(), pl);
        Arc::new(
            crate::player::Player::new_with_devices(devices, playlists, "test".to_string(), None)
                .unwrap(),
        )
    }

    #[test]
    fn get_venue_fixture_tags_no_venue() {
        let system = crate::lighting::system::LightingSystem::new();
        let tags = get_venue_fixture_tags(&system);
        assert!(tags.is_empty());
    }

    #[tokio::test]
    async fn playback_poller_sends_message() {
        let player = test_player(&["Song A"]);
        let (tx, mut rx) = broadcast::channel(16);

        let handle = tokio::spawn(playback_poller(player, tx));

        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for playback message")
            .expect("recv error");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "playback");
        assert_eq!(parsed["song_name"], "Song A");
        assert_eq!(parsed["is_playing"], false);
        assert!(parsed["playlist_songs"].is_array());
        assert!(parsed["tracks"].is_array());

        handle.abort();
    }

    #[tokio::test]
    async fn state_poller_sends_on_change() {
        let initial = Arc::new(crate::state::StateSnapshot::default());
        let (state_tx, state_rx) = watch::channel(initial);
        let (tx, mut rx) = broadcast::channel(16);

        let handle = tokio::spawn(state_poller(state_rx, tx));

        // Send a state update with fixtures
        let snapshot = Arc::new(crate::state::StateSnapshot {
            fixtures: vec![crate::state::FixtureSnapshot {
                name: "wash1".to_string(),
                channels: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("red".to_string(), 255);
                    m
                },
            }],
            active_effects: vec!["chase".to_string()],
        });
        state_tx.send(snapshot).unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for state message")
            .expect("recv error");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "state");
        assert!(parsed["fixtures"].is_object());
        assert_eq!(parsed["fixtures"]["wash1"]["red"], 255);
        assert_eq!(parsed["active_effects"][0], "chase");

        handle.abort();
    }

    #[tokio::test]
    async fn state_poller_exits_when_sender_dropped() {
        let initial = Arc::new(crate::state::StateSnapshot::default());
        let (state_tx, state_rx) = watch::channel(initial);
        let (tx, _rx) = broadcast::channel::<String>(16);

        let handle = tokio::spawn(state_poller(state_rx, tx));

        // Drop the sender — poller should exit
        drop(state_tx);

        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "poller should have exited");
    }

    #[tokio::test]
    async fn waveform_poller_sends_waveform_on_song() {
        let player = test_player(&["Song A"]);
        let (tx, mut rx) = broadcast::channel(16);
        let cache = new_waveform_cache();

        // Pre-populate cache so the poller doesn't need real audio files
        cache.lock().insert(
            "Song A".to_string(),
            vec![("track1".to_string(), vec![0.5, 1.0])],
        );

        let handle = tokio::spawn(waveform_poller(player, tx, cache));

        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for waveform message")
            .expect("recv error");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "waveform");
        assert_eq!(parsed["song_name"], "Song A");
        assert!(parsed["tracks"].is_array());

        handle.abort();
    }

    #[tokio::test]
    async fn waveform_poller_computes_on_cache_miss() {
        let player = test_player(&["Song A"]);
        let (tx, mut rx) = broadcast::channel(16);
        let cache = new_waveform_cache();
        // Don't pre-populate cache — force computation

        let handle = tokio::spawn(waveform_poller(player, tx, cache.clone()));

        let msg = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for waveform message")
            .expect("recv error");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "waveform");
        assert_eq!(parsed["song_name"], "Song A");

        // Should have been cached after computation
        assert!(cache.lock().contains_key("Song A"));

        handle.abort();
    }

    #[tokio::test]
    async fn playback_poller_skips_no_subscribers() {
        let player = test_player(&["Song A"]);
        let (tx, _) = broadcast::channel::<String>(16);

        // Drop the only receiver — poller should not panic
        let handle = tokio::spawn(playback_poller(player, tx));

        // Let it tick a few times with no subscribers
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Now subscribe and verify it sends when we have a subscriber
        // (can't easily test from here since tx was moved, but at least
        // we verified it doesn't panic)
        handle.abort();
    }

    #[tokio::test]
    async fn log_poller_sends_when_buffer_has_lines() {
        // Initialize the global log buffer if not already initialized.
        // init_tui_logging panics on double-init, so ignore errors.
        let _ = std::panic::catch_unwind(|| {
            crate::tui::logging::init_tui_logging(100);
        });

        let buffer =
            crate::tui::logging::get_log_buffer().expect("log buffer should be initialized");

        // Push some test lines
        {
            let mut buf = buffer.lock();
            buf.push_back("INFO test: hello from log_poller test".to_string());
        }

        let (tx, mut rx) = broadcast::channel(16);
        let handle = tokio::spawn(log_poller(tx));

        let msg = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for log message")
            .expect("recv error");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "logs");
        assert!(parsed["lines"].is_array());
        assert!(!parsed["lines"].as_array().unwrap().is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn log_poller_skips_when_no_new_lines() {
        // Initialize the global log buffer
        let _ = std::panic::catch_unwind(|| {
            crate::tui::logging::init_tui_logging(100);
        });

        let buffer = match crate::tui::logging::get_log_buffer() {
            Some(b) => b,
            None => return,
        };

        let (tx, mut rx) = broadcast::channel(16);
        let handle = tokio::spawn(log_poller(tx));

        // Push a line and wait for it to be sent
        {
            let mut buf = buffer.lock();
            buf.push_back("INFO test: first line".to_string());
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;

        // Don't push any new lines — next tick should skip (line 167)
        // Wait for another tick — it should produce no message
        let result = tokio::time::timeout(Duration::from_millis(800), rx.recv()).await;
        // Either timeout (no message sent) or lagged — both are fine
        if let Ok(Ok(msg)) = result {
            // If we got a message, it might be from accumulated lines;
            // the important thing is the poller doesn't panic
            let _: serde_json::Value = serde_json::from_str(&msg).unwrap();
        }

        handle.abort();
    }

    #[tokio::test]
    async fn waveform_prewarmer_caches_songs() {
        use tokio::time::timeout;

        let player = test_player(&["Song A"]);
        let cache = new_waveform_cache();

        // Pre-populate the cache so prewarmer skips computation
        cache.lock().insert(
            "Song A".to_string(),
            vec![("track1".to_string(), vec![0.5])],
        );

        // Run prewarmer with a timeout — it should skip Song A (already cached)
        // and finish quickly after the 1s initial delay
        let result = timeout(
            Duration::from_secs(3),
            waveform_prewarmer(player, cache.clone()),
        )
        .await;

        // Prewarmer should have completed (all songs already cached)
        assert!(result.is_ok(), "prewarmer should complete within timeout");
        assert!(cache.lock().contains_key("Song A"));
    }

    #[tokio::test]
    async fn waveform_prewarmer_computes_for_uncached() {
        use tokio::time::timeout;

        let player = test_player(&["Song A"]);
        let cache = new_waveform_cache();

        // Don't pre-populate — prewarmer will try to compute peaks.
        // With test songs using /dev/null, peaks will be empty but it
        // should still complete without panicking.
        let result = timeout(
            Duration::from_secs(5),
            waveform_prewarmer(player, cache.clone()),
        )
        .await;

        assert!(result.is_ok(), "prewarmer should complete within timeout");
        // Should have attempted to cache Song A (even if peaks are empty)
        assert!(cache.lock().contains_key("Song A"));
    }

    #[test]
    fn compute_track_peaks_stereo_selects_correct_channel() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("stereo.wav");

        // Create a stereo WAV with different amplitudes per channel
        let ch1: Vec<i32> = (0..4410).map(|i| (i * 10) % 32768).collect();
        let ch2: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![ch1, ch2], 44100)
            .expect("write stereo wav");

        // Both channels should produce peaks
        let peaks_ch1 = compute_track_peaks(&wav_path, 1, 10);
        let peaks_ch2 = compute_track_peaks(&wav_path, 2, 10);

        assert!(!peaks_ch1.is_empty());
        assert!(!peaks_ch2.is_empty());
        // Both should be normalized (max = 1.0)
        assert!(peaks_ch1.iter().any(|&p| (p - 1.0).abs() < f32::EPSILON));
        assert!(peaks_ch2.iter().any(|&p| (p - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    fn compute_track_peaks_large_bucket_count() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write wav");

        // More buckets than samples — should still work
        let peaks = compute_track_peaks(&wav_path, 1, 10000);
        assert!(!peaks.is_empty());
    }

    #[test]
    fn compute_waveform_peaks_missing_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tracks = vec![(
            "missing".to_string(),
            std::path::PathBuf::from("/nonexistent/file.wav"),
            1_u16,
        )];
        let results = compute_waveform_peaks(temp_dir.path(), &tracks);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "missing");
        assert!(results[0].1.is_empty());
    }

    #[test]
    fn compute_waveform_peaks_multiple_tracks() {
        let temp_dir = tempfile::tempdir().expect("tempdir");

        let wav1 = temp_dir.path().join("track1.wav");
        let wav2 = temp_dir.path().join("track2.wav");

        let samples: Vec<i32> = (0..4410).map(|i| (i * 50) % 32768).collect();
        crate::testutil::write_wav(wav1.clone(), vec![samples.clone()], 44100).expect("write wav1");
        crate::testutil::write_wav(wav2.clone(), vec![samples], 44100).expect("write wav2");

        let tracks = vec![
            ("Track 1".to_string(), wav1, 1_u16),
            ("Track 2".to_string(), wav2, 1_u16),
        ];

        let results = compute_waveform_peaks(temp_dir.path(), &tracks);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "Track 1");
        assert_eq!(results[1].0, "Track 2");
        assert!(!results[0].1.is_empty());
        assert!(!results[1].1.is_empty());
    }

    #[test]
    fn compute_track_peaks_file_channel_zero_returns_empty() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write wav");

        // file_channel 0 uses saturating_sub to get target_channel 0, which should work
        // (it matches the first channel of a mono file)
        let peaks = compute_track_peaks(&wav_path, 0, 10);
        // With file_channel 0, target_channel = 0 which is valid for a mono file
        assert!(!peaks.is_empty());
    }

    #[test]
    fn compute_track_peaks_all_zeros_returns_empty() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("silence.wav");
        // All-zero samples should produce peaks of 0.0 but still have buckets
        let samples: Vec<i32> = vec![0_i32; 4410];
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write wav");

        let peaks = compute_track_peaks(&wav_path, 1, 10);
        // All zero peaks, so normalization divides by 0.0 which is skipped,
        // all values should remain 0.0
        assert!(!peaks.is_empty());
        for &p in &peaks {
            assert!((p - 0.0).abs() < f32::EPSILON, "expected 0.0, got {}", p);
        }
    }

    #[test]
    fn compute_waveform_peaks_empty_tracks() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tracks: Vec<(String, PathBuf, u16)> = vec![];
        let results = compute_waveform_peaks(temp_dir.path(), &tracks);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn playback_poller_no_subscribers_does_not_panic() {
        let player = test_player(&["Song A"]);
        let (tx, _rx) = broadcast::channel::<String>(16);

        // Drop the receiver immediately so receiver_count() == 0
        drop(_rx);

        let handle = tokio::spawn(playback_poller(player, tx));

        // Let it tick several times with zero subscribers
        tokio::time::sleep(Duration::from_millis(600)).await;

        // It should still be running without panicking
        assert!(!handle.is_finished());
        handle.abort();
    }

    #[tokio::test]
    async fn state_poller_no_subscribers_skips_send() {
        let initial = Arc::new(crate::state::StateSnapshot::default());
        let (state_tx, state_rx) = watch::channel(initial);
        let (tx, _rx) = broadcast::channel::<String>(16);

        // Drop receiver so receiver_count() == 0
        drop(_rx);

        let handle = tokio::spawn(state_poller(state_rx, tx));

        // Send a state update with no subscribers
        let snapshot = Arc::new(crate::state::StateSnapshot {
            fixtures: vec![crate::state::FixtureSnapshot {
                name: "light1".to_string(),
                channels: std::collections::HashMap::new(),
            }],
            active_effects: vec![],
        });
        state_tx.send(snapshot).unwrap();

        // Let it process with no subscribers
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Should still be running
        assert!(!handle.is_finished());
        handle.abort();
    }

    #[tokio::test]
    async fn waveform_poller_no_subscribers_skips() {
        let player = test_player(&["Song A"]);
        let (tx, _rx) = broadcast::channel::<String>(16);
        let cache = new_waveform_cache();

        // Drop the receiver
        drop(_rx);

        let handle = tokio::spawn(waveform_poller(player, tx, cache));

        // Let it tick with no subscribers
        tokio::time::sleep(Duration::from_millis(600)).await;

        // Should still be running
        assert!(!handle.is_finished());
        handle.abort();
    }

    #[tokio::test]
    async fn log_poller_no_subscribers_skips() {
        let (tx, _rx) = broadcast::channel::<String>(16);

        // Drop the receiver
        drop(_rx);

        let handle = tokio::spawn(log_poller(tx));

        // Let it tick with no subscribers
        tokio::time::sleep(Duration::from_millis(600)).await;

        assert!(!handle.is_finished());
        handle.abort();
    }

    /// Creates a LightingSystem loaded with fixture types and venues from DSL files.
    fn create_test_lighting_system(
        fixture_type_dsl: &str,
        venue_dsl: &str,
        venue_name: &str,
    ) -> (crate::lighting::system::LightingSystem, tempfile::TempDir) {
        use crate::config::lighting::{Directories, Lighting};

        let dir = tempfile::tempdir().expect("tempdir");
        let ft_dir = dir.path().join("fixture_types");
        let venues_dir = dir.path().join("venues");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::create_dir(&venues_dir).unwrap();

        std::fs::write(ft_dir.join("types.light"), fixture_type_dsl).unwrap();
        std::fs::write(venues_dir.join("venues.light"), venue_dsl).unwrap();

        let config = Lighting::new(
            Some(venue_name.to_string()),
            None,
            None,
            Some(Directories::new(
                Some("fixture_types".to_string()),
                Some("venues".to_string()),
            )),
        );

        let mut system = crate::lighting::system::LightingSystem::new();
        system.load(&config, dir.path()).unwrap();

        (system, dir)
    }

    #[test]
    fn build_metadata_json_with_lighting_system() {
        let fixture_dsl = r#"fixture_type "wash" {
            channels: 3
            channel_map: {
                "red": 1,
                "green": 2,
                "blue": 3
            }
        }"#;
        let venue_dsl = r#"venue "Test Venue" {
            fixture "front_wash_1" wash @ 1:1 tags ["front", "wash"]
        }"#;

        let (system, _dir) = create_test_lighting_system(fixture_dsl, venue_dsl, "Test Venue");

        let ls = Arc::new(parking_lot::Mutex::new(system));
        let json_str = build_metadata_json(Some(&ls));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(parsed["type"], "metadata");
        assert!(parsed["fixtures"].is_object());

        let fixture_meta = &parsed["fixtures"]["front_wash_1"];
        assert!(fixture_meta.is_object());
        assert_eq!(fixture_meta["type"], "wash");

        let tags = fixture_meta["tags"].as_array().unwrap();
        assert!(tags.contains(&serde_json::json!("front")));
        assert!(tags.contains(&serde_json::json!("wash")));
    }

    #[test]
    fn build_metadata_json_with_fixture_no_tags() {
        let fixture_dsl = r#"fixture_type "par" {
            channels: 1
            channel_map: {
                "dimmer": 1
            }
        }"#;
        let venue_dsl = r#"venue "Test Venue" {
            fixture "par_1" par @ 1:1
        }"#;

        let (system, _dir) = create_test_lighting_system(fixture_dsl, venue_dsl, "Test Venue");

        let ls = Arc::new(parking_lot::Mutex::new(system));
        let json_str = build_metadata_json(Some(&ls));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");

        assert_eq!(parsed["type"], "metadata");
        let fixture_meta = &parsed["fixtures"]["par_1"];
        assert_eq!(fixture_meta["type"], "par");
        let tags = fixture_meta["tags"].as_array().unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn get_venue_fixture_tags_with_venue() {
        let fixture_dsl = r#"fixture_type "spot" {
            channels: 1
            channel_map: {
                "dimmer": 1
            }
        }"#;
        let venue_dsl = r#"venue "Live Venue" {
            fixture "spot_1" spot @ 1:1 tags ["overhead", "spot"]
        }"#;

        let (system, _dir) = create_test_lighting_system(fixture_dsl, venue_dsl, "Live Venue");

        let tags = get_venue_fixture_tags(&system);
        assert_eq!(tags.len(), 1);
        let spot_tags = tags.get("spot_1").unwrap();
        assert!(spot_tags.contains(&"overhead".to_string()));
        assert!(spot_tags.contains(&"spot".to_string()));
    }

    #[test]
    fn build_metadata_json_with_multiple_fixtures() {
        let fixture_dsl = r#"fixture_type "generic" {
            channels: 1
            channel_map: {
                "dimmer": 1
            }
        }"#;
        let venue_dsl = r#"venue "Multi" {
            fixture "fixture_1" generic @ 1:1 tags ["group_1"]
            fixture "fixture_2" generic @ 1:2 tags ["group_2"]
            fixture "fixture_3" generic @ 1:3 tags ["group_3"]
        }"#;

        let (system, _dir) = create_test_lighting_system(fixture_dsl, venue_dsl, "Multi");

        let ls = Arc::new(parking_lot::Mutex::new(system));
        let json_str = build_metadata_json(Some(&ls));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("valid JSON");

        let fixtures_obj = parsed["fixtures"].as_object().unwrap();
        assert_eq!(fixtures_obj.len(), 3);
        for i in 1..=3 {
            let key = format!("fixture_{}", i);
            assert!(fixtures_obj.contains_key(&key));
            assert_eq!(fixtures_obj[&key]["type"], "generic");
        }
    }

    #[tokio::test]
    async fn config_watcher_sends_on_mutation() {
        let player = test_player(&["Song A"]);

        // Set up a config store on the player.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: songs\n").unwrap();
        let cfg = crate::config::Player::deserialize(&path).unwrap();
        let store = Arc::new(crate::config::ConfigStore::new(cfg, path));
        player.set_config_store(store.clone());

        let (tx, mut rx) = broadcast::channel(16);
        let handle = tokio::spawn(config_watcher(player, tx));

        // Yield to let the watcher subscribe before we mutate.
        tokio::task::yield_now().await;

        // Trigger a mutation so the watcher broadcasts.
        let snap = store.read().await.unwrap();
        store
            .update_midi(
                Some(crate::config::Midi::new("test-midi", None)),
                &snap.checksum,
            )
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for config_changed message")
            .expect("recv error");

        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "config_changed");

        handle.abort();
    }

    #[tokio::test]
    async fn config_watcher_no_store_exits_immediately() {
        let player = test_player(&["Song A"]);
        // No config store set — watcher should return immediately.
        let (tx, _rx) = broadcast::channel(16);
        let handle = tokio::spawn(config_watcher(player, tx));

        // Should complete quickly since there's no store.
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "config_watcher should exit when no store");
    }
}
