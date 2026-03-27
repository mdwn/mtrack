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

//! The notification engine coordinates audio playback for notification events.
//!
//! It resolves notification events to cached PCM audio using a three-tier
//! priority: per-song overrides → global overrides → default tones. Audio
//! is played through the mixer via the `mtrack:looping` track mapping.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{debug, info};

use super::audio::{generate_default_tones, load_overrides};
use super::events::NotificationEvent;
use crate::audio;
use crate::audio::mixer::{ActiveSource, AudioMixer};
use crate::audio::sample_source::channel_mapped::ChannelMappedSource;
use crate::audio::sample_source::MemorySampleSource;
use crate::playsync::CancelHandle;
use crate::samples::loader::SampleLoader;

/// Well-known track name for notification audio routing.
/// Users route this in their audio track mappings, e.g.:
/// ```yaml
/// track_mappings:
///   mtrack:looping: [9]
/// ```
pub const NOTIFICATION_TRACK: &str = "mtrack:looping";

/// The notification engine manages audio playback for notification events.
pub struct NotificationEngine {
    /// Pre-generated default tone PCM for each event type.
    default_tones: HashMap<String, Arc<Vec<f32>>>,
    /// Global user-provided audio overrides (loaded at startup).
    global_overrides: HashMap<String, Arc<Vec<f32>>>,
    /// Per-song audio overrides (swapped on song change).
    song_overrides: RwLock<HashMap<String, Arc<Vec<f32>>>>,
    /// Target sample rate (matches audio output).
    sample_rate: u32,
}

impl NotificationEngine {
    /// Creates a new notification engine.
    ///
    /// Generates default tones at the given sample rate and loads any global
    /// override audio files from the provided configuration.
    pub fn new(
        sample_rate: u32,
        global_overrides_config: &HashMap<String, String>,
        global_section_overrides: &HashMap<String, String>,
        base_path: &Path,
    ) -> Self {
        let default_tones = generate_default_tones(sample_rate);

        // Load global overrides.
        let mut loader = SampleLoader::new(sample_rate);
        let mut global_overrides = load_overrides(global_overrides_config, base_path, &mut loader);

        // Load global section name overrides with "section:" prefix.
        for (section_name, path_str) in global_section_overrides {
            let key = format!("section:{}", section_name);
            let path = if Path::new(path_str).is_absolute() {
                path_str.into()
            } else {
                base_path.join(path_str)
            };
            match super::audio::load_audio_file(&mut loader, &path) {
                Ok(samples) => {
                    global_overrides.insert(key, samples);
                }
                Err(e) => {
                    tracing::warn!(
                        section = section_name.as_str(),
                        path = ?path,
                        err = %e,
                        "Failed to load section notification override"
                    );
                }
            }
        }

        info!(
            default_tones = default_tones.len(),
            global_overrides = global_overrides.len(),
            "Notification engine initialized"
        );

        Self {
            default_tones,
            global_overrides,
            song_overrides: RwLock::new(HashMap::new()),
            sample_rate,
        }
    }

    /// Creates a notification engine with only default tones (no overrides).
    pub fn with_defaults(sample_rate: u32) -> Self {
        Self {
            default_tones: generate_default_tones(sample_rate),
            global_overrides: HashMap::new(),
            song_overrides: RwLock::new(HashMap::new()),
            sample_rate,
        }
    }

    /// Sets per-song notification audio overrides.
    ///
    /// Called when a song starts playing. The overrides map contains event keys
    /// (e.g. `"loop_armed"`) and section names mapped to file paths.
    pub fn set_song_overrides(
        &self,
        overrides: &HashMap<String, String>,
        section_overrides: &HashMap<String, String>,
        base_path: &Path,
    ) {
        let mut loader = SampleLoader::new(self.sample_rate);
        let mut loaded = load_overrides(overrides, base_path, &mut loader);

        // Load section name overrides with "section:" prefix.
        for (section_name, path_str) in section_overrides {
            let key = format!("section:{}", section_name);
            let path = if Path::new(path_str).is_absolute() {
                path_str.into()
            } else {
                base_path.join(path_str)
            };
            match super::audio::load_audio_file(&mut loader, &path) {
                Ok(samples) => {
                    loaded.insert(key, samples);
                }
                Err(e) => {
                    tracing::warn!(
                        section = section_name.as_str(),
                        path = ?path,
                        err = %e,
                        "Failed to load song section notification override"
                    );
                }
            }
        }

        *self.song_overrides.write() = loaded;
    }

    /// Clears per-song overrides (called on song stop).
    pub fn clear_song_overrides(&self) {
        self.song_overrides.write().clear();
    }

    /// Plays a notification for the given event through the mixer.
    ///
    /// Resolution order: song override → global override → default tone.
    /// For `SectionEntering` events, also tries the generic `"section_entering"`
    /// key if no per-section override exists.
    ///
    /// Does nothing if the `mtrack:looping` track mapping is not configured.
    pub fn play(
        &self,
        event: NotificationEvent,
        mixer: &AudioMixer,
        mappings: &HashMap<String, Vec<u16>>,
    ) {
        // Only play if the user has configured the track mapping.
        if !mappings.contains_key(NOTIFICATION_TRACK) {
            return;
        }

        let samples = match self.resolve_audio(&event) {
            Some(s) => s,
            None => {
                debug!(event = ?event, "No audio found for notification event");
                return;
            }
        };

        let source = MemorySampleSource::from_shared(
            samples,
            1, // mono
            self.sample_rate,
            1.0,
        );

        // Map the mono source channel to the notification track.
        let mapped = ChannelMappedSource::new(
            Box::new(source),
            vec![vec![NOTIFICATION_TRACK.to_string()]],
            1,
        );

        let is_finished = Arc::new(AtomicBool::new(false));
        let active_source = ActiveSource {
            id: audio::next_source_id(),
            source: Box::new(mapped),
            track_mappings: mappings.clone(),
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            cancel_handle: CancelHandle::new(),
            is_finished,
            start_at_sample: None,
            cancel_at_sample: None,
            gain_envelope: None,
        };

        mixer.add_source(active_source);
    }

    /// Resolves a notification event to cached PCM audio.
    ///
    /// Checks song overrides first, then global overrides, then default tones.
    /// For `SectionEntering`, tries the specific section key first, then falls
    /// back to the generic `"section_entering"` key.
    fn resolve_audio(&self, event: &NotificationEvent) -> Option<Arc<Vec<f32>>> {
        let override_key = event.override_key();
        let fallback_key = event.fallback_key();

        // Try song overrides.
        {
            let song = self.song_overrides.read();
            if let Some(samples) = song.get(&override_key) {
                return Some(samples.clone());
            }
            // For section events, try generic fallback in song overrides too.
            if override_key != fallback_key {
                if let Some(samples) = song.get(fallback_key) {
                    return Some(samples.clone());
                }
            }
        }

        // Try global overrides.
        if let Some(samples) = self.global_overrides.get(&override_key) {
            return Some(samples.clone());
        }
        if override_key != fallback_key {
            if let Some(samples) = self.global_overrides.get(fallback_key) {
                return Some(samples.clone());
            }
        }

        // Fall back to default tones.
        if let Some(samples) = self.default_tones.get(fallback_key) {
            return Some(samples.clone());
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_defaults_has_all_tones() {
        let engine = NotificationEngine::with_defaults(44100);
        assert!(engine.default_tones.contains_key("section_entering"));
        assert!(engine.default_tones.contains_key("loop_armed"));
        assert!(engine.default_tones.contains_key("break_requested"));
        assert!(engine.default_tones.contains_key("loop_exited"));
    }

    #[test]
    fn resolve_audio_defaults() {
        let engine = NotificationEngine::with_defaults(44100);

        assert!(engine
            .resolve_audio(&NotificationEvent::LoopArmed)
            .is_some());
        assert!(engine
            .resolve_audio(&NotificationEvent::BreakRequested)
            .is_some());
        assert!(engine
            .resolve_audio(&NotificationEvent::LoopExited)
            .is_some());
        assert!(engine
            .resolve_audio(&NotificationEvent::SectionEntering {
                section_name: "verse".to_string(),
            })
            .is_some());
    }

    #[test]
    fn resolve_audio_section_entering_uses_generic_fallback() {
        let engine = NotificationEngine::with_defaults(44100);

        // No per-section override, should fall back to "section_entering" tone.
        let audio = engine
            .resolve_audio(&NotificationEvent::SectionEntering {
                section_name: "nonexistent".to_string(),
            })
            .unwrap();

        let default = engine.default_tones.get("section_entering").unwrap();
        assert!(Arc::ptr_eq(&audio, default));
    }

    #[test]
    fn song_overrides_take_precedence() {
        let engine = NotificationEngine::with_defaults(44100);

        let custom_audio = Arc::new(vec![0.5f32; 100]);
        {
            let mut overrides = engine.song_overrides.write();
            overrides.insert("loop_armed".to_string(), custom_audio.clone());
        }

        let resolved = engine.resolve_audio(&NotificationEvent::LoopArmed).unwrap();
        assert!(Arc::ptr_eq(&resolved, &custom_audio));
    }

    #[test]
    fn global_overrides_take_precedence_over_defaults() {
        let custom_audio = Arc::new(vec![0.3f32; 200]);
        let engine = NotificationEngine {
            default_tones: generate_default_tones(44100),
            global_overrides: HashMap::from([("loop_armed".to_string(), custom_audio.clone())]),
            song_overrides: RwLock::new(HashMap::new()),
            sample_rate: 44100,
        };

        let resolved = engine.resolve_audio(&NotificationEvent::LoopArmed).unwrap();
        assert!(Arc::ptr_eq(&resolved, &custom_audio));
    }

    #[test]
    fn song_overrides_take_precedence_over_global() {
        let global_audio = Arc::new(vec![0.3f32; 200]);
        let song_audio = Arc::new(vec![0.7f32; 150]);

        let engine = NotificationEngine {
            default_tones: generate_default_tones(44100),
            global_overrides: HashMap::from([("loop_armed".to_string(), global_audio)]),
            song_overrides: RwLock::new(HashMap::from([(
                "loop_armed".to_string(),
                song_audio.clone(),
            )])),
            sample_rate: 44100,
        };

        let resolved = engine.resolve_audio(&NotificationEvent::LoopArmed).unwrap();
        assert!(Arc::ptr_eq(&resolved, &song_audio));
    }

    #[test]
    fn section_specific_override() {
        let verse_audio = Arc::new(vec![0.9f32; 50]);
        let engine = NotificationEngine {
            default_tones: generate_default_tones(44100),
            global_overrides: HashMap::from([("section:verse".to_string(), verse_audio.clone())]),
            song_overrides: RwLock::new(HashMap::new()),
            sample_rate: 44100,
        };

        let resolved = engine
            .resolve_audio(&NotificationEvent::SectionEntering {
                section_name: "verse".to_string(),
            })
            .unwrap();
        assert!(Arc::ptr_eq(&resolved, &verse_audio));

        // Other sections still fall back to generic.
        let chorus = engine
            .resolve_audio(&NotificationEvent::SectionEntering {
                section_name: "chorus".to_string(),
            })
            .unwrap();
        let default = engine.default_tones.get("section_entering").unwrap();
        assert!(Arc::ptr_eq(&chorus, default));
    }

    #[test]
    fn clear_song_overrides() {
        let engine = NotificationEngine::with_defaults(44100);

        {
            let mut overrides = engine.song_overrides.write();
            overrides.insert("loop_armed".to_string(), Arc::new(vec![0.5f32; 100]));
        }

        engine.clear_song_overrides();
        assert!(engine.song_overrides.read().is_empty());
    }

    #[test]
    fn play_adds_source_to_mixer() {
        let engine = NotificationEngine::with_defaults(48000);
        let mixer = AudioMixer::new(2, 48000);
        let mappings = HashMap::from([(NOTIFICATION_TRACK.to_string(), vec![1u16, 2])]);

        engine.play(NotificationEvent::LoopArmed, &mixer, &mappings);

        let sources = mixer.get_active_sources();
        assert_eq!(sources.read().len(), 1);
    }

    #[test]
    fn play_skips_when_no_track_mapping() {
        let engine = NotificationEngine::with_defaults(48000);
        let mixer = AudioMixer::new(2, 48000);
        let mappings = HashMap::new(); // no mtrack:looping mapping

        engine.play(NotificationEvent::LoopArmed, &mixer, &mappings);

        let sources = mixer.get_active_sources();
        assert_eq!(sources.read().len(), 0);
    }

    #[test]
    fn play_produces_nonzero_audio() {
        let engine = NotificationEngine::with_defaults(48000);
        let mixer = AudioMixer::new(2, 48000);
        let mappings = HashMap::from([(NOTIFICATION_TRACK.to_string(), vec![1u16, 2])]);

        engine.play(NotificationEvent::LoopArmed, &mixer, &mappings);

        // Process enough frames to cover the tone (50ms at 48kHz = 2400 frames).
        let frames = mixer.process_frames(2400);
        let has_nonzero = frames.iter().any(|&s| s != 0.0);
        assert!(has_nonzero, "Notification should produce audible samples");
    }

    #[test]
    fn play_all_event_types() {
        let engine = NotificationEngine::with_defaults(48000);
        let mappings = HashMap::from([(NOTIFICATION_TRACK.to_string(), vec![1u16])]);

        let events = [
            NotificationEvent::SectionEntering {
                section_name: "verse".to_string(),
            },
            NotificationEvent::LoopArmed,
            NotificationEvent::BreakRequested,
            NotificationEvent::LoopExited,
        ];

        for event in events {
            let mixer = AudioMixer::new(1, 48000);
            engine.play(event.clone(), &mixer, &mappings);
            assert_eq!(
                mixer.get_active_sources().read().len(),
                1,
                "Event {:?} should add a source",
                event,
            );
        }
    }
}
