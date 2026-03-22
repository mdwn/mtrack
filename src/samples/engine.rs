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

//! Main sample engine that coordinates trigger matching, sample loading, and playback.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use midly::live::LiveEvent;
use midly::MidiMessage;
use parking_lot::RwLock;
use tracing::{debug, error, info, warn};

use super::loader::{LoadedSample, SampleLoader};
use super::trigger::TriggerEvent;
use super::voice::{Voice, VoiceManager};
use crate::audio;
use crate::audio::sample_source::ChannelMappedSource;
use crate::config::samples::{ReleaseBehavior, SampleDefinition, SampleTrigger, SamplesConfig};
use crate::config::ToMidiEvent;
use crate::playsync::CancelHandle;

/// Precomputed data for a loaded sample file, avoiding allocations during trigger.
struct PrecomputedSampleData {
    /// The loaded sample audio data.
    loaded: LoadedSample,
    /// Precomputed channel labels for ChannelMappedSource.
    channel_labels: Vec<Vec<String>>,
    /// Precomputed track mappings for the mixer.
    track_mappings: HashMap<String, Vec<u16>>,
}

/// Active sample definition with preloaded audio data.
struct ActiveSample {
    /// The sample definition from config.
    definition: SampleDefinition,
    /// Loaded audio data by file path, with precomputed mappings.
    loaded_files: HashMap<PathBuf, PrecomputedSampleData>,
    /// Base path for resolving relative file paths.
    base_path: PathBuf,
}

/// Data prepared for sample playback, produced by `prepare_sample`.
struct PreparedSample {
    source_id: u64,
    channel_mapped: Box<ChannelMappedSource>,
    track_mappings: HashMap<String, Vec<u16>>,
    channel_count: u16,
    retrigger: crate::config::samples::RetriggerBehavior,
    release_behavior: ReleaseBehavior,
}

/// A trigger definition with pre-converted MIDI event for matching.
struct ActiveTrigger {
    /// The MIDI event to match (as LiveEvent for efficient comparison).
    midi_event: LiveEvent<'static>,
    /// The sample name to trigger.
    sample_name: String,
}

/// The sample engine manages MIDI-triggered sample playback.
pub struct SampleEngine {
    /// Sample loader for loading audio files.
    loader: SampleLoader,
    /// Active sample definitions by name.
    samples: HashMap<String, ActiveSample>,
    /// Active triggers.
    triggers: Vec<ActiveTrigger>,
    /// Voice manager for polyphony.
    voice_manager: RwLock<VoiceManager>,
    /// Channel for adding sources without lock contention.
    source_tx: crate::audio::SourceSender,
    /// Reference to mixer for sample scheduling.
    mixer: Arc<crate::audio::mixer::AudioMixer>,
    /// Fixed delay in samples for consistent trigger latency.
    fixed_delay_samples: u64,
    /// Track mappings from the active profile, for resolving output_track names.
    profile_track_mappings: HashMap<String, Vec<u16>>,
}

impl SampleEngine {
    /// Creates a new sample engine.
    ///
    /// The `buffer_size` is used to calculate the minimum fixed delay for consistent
    /// trigger latency. The delay is set to 1x the buffer size to ensure samples are
    /// always scheduled ahead of the current mixing position.
    pub fn new(
        mixer: Arc<crate::audio::mixer::AudioMixer>,
        source_tx: crate::audio::SourceSender,
        max_voices: u32,
        buffer_size: usize,
        profile_track_mappings: HashMap<String, Vec<u16>>,
    ) -> Self {
        let sample_rate = mixer.sample_rate();
        // Fixed delay of 1x buffer size ensures the sample is scheduled ahead of current mixing
        // This accounts for async channel delivery between trigger and audio callback
        // At 256 samples @ 44.1kHz: ~5.8ms latency
        let fixed_delay_samples = buffer_size as u64;
        Self {
            loader: SampleLoader::new(sample_rate),
            samples: HashMap::new(),
            triggers: Vec::new(),
            voice_manager: RwLock::new(VoiceManager::new(max_voices)),
            source_tx,
            mixer,
            fixed_delay_samples,
            profile_track_mappings,
        }
    }

    /// Loads global sample configuration.
    /// This should be called at startup with the global config.
    pub fn load_global_config(
        &mut self,
        config: &SamplesConfig,
        base_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        info!(
            samples = config.samples().len(),
            triggers = config.sample_triggers().len(),
            "Loading global samples configuration"
        );

        // Load sample definitions
        for (name, definition) in config.samples() {
            self.load_sample(name, definition, base_path)?;
        }

        // Load triggers
        for trigger in config.sample_triggers() {
            self.add_trigger(trigger)?;
        }

        info!(
            loaded_samples = self.samples.len(),
            loaded_triggers = self.triggers.len(),
            memory_kb = self.loader.total_memory_usage() / 1024,
            "Global samples loaded"
        );

        Ok(())
    }

    /// Loads per-song sample configuration.
    /// This merges with global config (song config overrides global).
    pub fn load_song_config(
        &mut self,
        config: &SamplesConfig,
        base_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        if config.samples().is_empty() && config.sample_triggers().is_empty() {
            return Ok(());
        }

        info!(
            samples = config.samples().len(),
            triggers = config.sample_triggers().len(),
            "Loading song samples configuration"
        );

        // Load/override sample definitions
        for (name, definition) in config.samples() {
            self.load_sample(name, definition, base_path)?;
        }

        // Add/override triggers
        for trigger in config.sample_triggers() {
            self.add_trigger(trigger)?;
        }

        Ok(())
    }

    /// Loads a sample definition and preloads its audio data.
    fn load_sample(
        &mut self,
        name: &str,
        definition: &SampleDefinition,
        base_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        // Load all files referenced by this definition
        let raw_loaded_files = self.loader.load_definition(definition, base_path)?;

        // Precompute channel labels and track mappings for each loaded file
        // This avoids string formatting and HashMap allocation on every trigger
        let loaded_files: HashMap<PathBuf, PrecomputedSampleData> = raw_loaded_files
            .into_iter()
            .map(|(path, loaded)| {
                let (channel_labels, track_mappings) =
                    if let Some(track_name) = definition.output_track() {
                        // Resolve output_track through the profile's track_mappings
                        match self.profile_track_mappings.get(track_name) {
                            Some(channels) => {
                                let labels = (0..loaded.channel_count())
                                    .map(|_| vec![track_name.to_string()])
                                    .collect();
                                let mut map = HashMap::new();
                                map.insert(track_name.to_string(), channels.clone());
                                (labels, map)
                            }
                            None => {
                                warn!(
                                sample = name,
                                track = track_name,
                                "output_track not found in track_mappings, sample will be silent"
                            );
                                (
                                    (0..loaded.channel_count()).map(|_| Vec::new()).collect(),
                                    HashMap::new(),
                                )
                            }
                        }
                    } else if !definition.output_channels().is_empty() {
                        // Existing synthetic __sample_out_{ch} approach
                        let output_channels = definition.output_channels();
                        let labels = (0..loaded.channel_count())
                            .map(|_| {
                                output_channels
                                    .iter()
                                    .map(|ch| format!("__sample_out_{}", ch))
                                    .collect()
                            })
                            .collect();
                        let map = output_channels
                            .iter()
                            .map(|ch| (format!("__sample_out_{}", ch), vec![*ch]))
                            .collect();
                        (labels, map)
                    } else {
                        warn!(sample = name, "No output routing configured for sample");
                        (
                            (0..loaded.channel_count()).map(|_| Vec::new()).collect(),
                            HashMap::new(),
                        )
                    };

                (
                    path,
                    PrecomputedSampleData {
                        loaded,
                        channel_labels,
                        track_mappings,
                    },
                )
            })
            .collect();

        // Set up per-sample voice limit if configured
        if let Some(max_voices) = definition.max_voices() {
            let mut vm = self.voice_manager.write();
            vm.set_sample_limit(name, max_voices);
        }

        self.samples.insert(
            name.to_string(),
            ActiveSample {
                definition: definition.clone(),
                loaded_files,
                base_path: base_path.to_path_buf(),
            },
        );

        debug!(name, "Sample loaded");
        Ok(())
    }

    /// Adds a trigger mapping.
    fn add_trigger(&mut self, trigger: &SampleTrigger) -> Result<(), Box<dyn Error>> {
        let midi_event = trigger.trigger().to_midi_event()?;

        // Remove any existing trigger with the same MIDI event
        self.triggers.retain(|t| t.midi_event != midi_event);

        self.triggers.push(ActiveTrigger {
            midi_event,
            sample_name: trigger.sample().to_string(),
        });

        debug!(sample = trigger.sample(), "Trigger added");
        Ok(())
    }

    /// Triggers a sample by name using a source-agnostic TriggerEvent.
    /// This is the common entry point for all trigger sources (MIDI, audio triggers, etc.).
    pub fn trigger(&self, event: &TriggerEvent) {
        let prepared = match self.prepare_sample(&event.sample_name, event.velocity) {
            Some(p) => p,
            None => return,
        };

        let source_cancel_handle = CancelHandle::new();
        let source_cancel_at_sample = Arc::new(AtomicU64::new(0));
        let is_finished = Arc::new(AtomicBool::new(false));

        let voice = Voice::new(
            event.sample_name.clone(),
            event.release_group.clone(),
            prepared.release_behavior,
            prepared.source_id,
            source_cancel_handle.clone(),
            source_cancel_at_sample.clone(),
            is_finished.clone(),
        );

        let start_at_sample = self.mixer.current_sample() + self.fixed_delay_samples;

        let to_stop = {
            let mut vm = self.voice_manager.write();
            vm.add_voice(voice, prepared.retrigger)
        };

        for cancel_at in to_stop {
            cancel_at.store(start_at_sample, Ordering::Relaxed);
        }

        let active_source = crate::audio::mixer::ActiveSource {
            id: prepared.source_id,
            source: prepared.channel_mapped,
            track_mappings: prepared.track_mappings,
            channel_mappings: Vec::new(),
            cached_source_channel_count: prepared.channel_count,
            is_finished,
            cancel_handle: source_cancel_handle,
            start_at_sample: Some(start_at_sample),
            cancel_at_sample: Some(source_cancel_at_sample),
            gain_envelope: None,
        };

        if let Err(e) = self.source_tx.send(active_source) {
            error!(error = %e, "Failed to send sample to mixer");
        }

        debug!(
            sample = event.sample_name.as_str(),
            velocity = event.velocity,
            source_id = prepared.source_id,
            "Sample triggered"
        );
    }

    /// Releases all voices in the named release group.
    /// Each voice's own ReleaseBehavior (stored at trigger time) determines
    /// whether it is stopped or allowed to play to completion.
    pub fn release(&self, group: &str) {
        let to_stop = {
            let mut vm = self.voice_manager.write();
            vm.release(group)
        };

        let stopped_count = to_stop.len();
        if !to_stop.is_empty() {
            for handle in to_stop {
                handle.cancel();
            }
            debug!(group, stopped = stopped_count, "Release handled");
        }
    }

    /// Prepares a sample for playback: looks up the definition, resolves the file
    /// for the given velocity, and creates the audio source. Returns None if the
    /// sample cannot be prepared (not found, no file for velocity, etc.).
    fn prepare_sample(&self, sample_name: &str, velocity: u8) -> Option<PreparedSample> {
        let sample = match self.samples.get(sample_name) {
            Some(s) => s,
            None => {
                warn!(sample = sample_name, "Sample not found");
                return None;
            }
        };

        let (file_path, volume) = match sample.definition.file_for_velocity(velocity) {
            Some((file, vol)) => {
                let path = if Path::new(file).is_absolute() {
                    PathBuf::from(file)
                } else {
                    sample.base_path.join(file)
                };
                (path, vol)
            }
            None => {
                warn!(
                    sample = sample_name,
                    velocity, "No sample file for velocity"
                );
                return None;
            }
        };

        let precomputed = match sample.loaded_files.get(&file_path) {
            Some(p) => p,
            None => {
                error!(
                    sample = sample_name,
                    path = ?file_path,
                    "Sample file not loaded"
                );
                return None;
            }
        };

        let source = precomputed.loaded.create_source(volume);
        let source_id = audio::next_source_id();

        let channel_mapped = ChannelMappedSource::new(
            Box::new(source),
            precomputed.channel_labels.clone(),
            precomputed.loaded.channel_count(),
        );
        let track_mappings = precomputed.track_mappings.clone();

        Some(PreparedSample {
            source_id,
            channel_mapped: Box::new(channel_mapped),
            track_mappings,
            channel_count: precomputed.loaded.channel_count(),
            retrigger: sample.definition.retrigger(),
            release_behavior: sample.definition.release_behavior(),
        })
    }

    /// Processes an incoming MIDI event.
    /// This is the main entry point called when MIDI data is received.
    pub fn process_midi_event(&self, raw_event: &[u8]) {
        let event = match LiveEvent::parse(raw_event) {
            Ok(e) => e,
            Err(e) => {
                debug!(error = ?e, "Failed to parse MIDI event");
                return;
            }
        };

        // Check for Note Off events first (for voice management)
        if let LiveEvent::Midi { channel, message } = &event {
            match message {
                MidiMessage::NoteOff { key, .. } => {
                    let group = Self::midi_release_group(u8::from(*channel) + 1, u8::from(*key));
                    self.release(&group);
                }
                MidiMessage::NoteOn { key, vel } if u8::from(*vel) == 0 => {
                    // Note On with velocity 0 is equivalent to Note Off
                    let group = Self::midi_release_group(u8::from(*channel) + 1, u8::from(*key));
                    self.release(&group);
                }
                _ => {}
            }
        }

        // NoteOn with velocity 0 was already handled as a release above — skip trigger matching
        let is_note_off_as_note_on = matches!(
            &event,
            LiveEvent::Midi { message: MidiMessage::NoteOn { vel, .. }, .. }
            if u8::from(*vel) == 0
        );

        // Check against triggers (hot path - minimal overhead)
        for trigger in &self.triggers {
            if !is_note_off_as_note_on && self.matches_trigger(&event, &trigger.midi_event) {
                let velocity = self.extract_velocity(&event);
                let release_group = self
                    .extract_note_channel(&event)
                    .map(|(note, channel)| Self::midi_release_group(channel, note));
                let trigger_event = TriggerEvent {
                    sample_name: trigger.sample_name.clone(),
                    velocity,
                    release_group,
                };
                self.trigger(&trigger_event);
            }
        }
    }

    /// Checks if a MIDI event matches a trigger.
    fn matches_trigger(&self, event: &LiveEvent, trigger_event: &LiveEvent) -> bool {
        // For now, we do exact matching on the event type and channel/key
        // but ignore velocity in the match (velocity is used for sample selection)
        match (event, trigger_event) {
            (
                LiveEvent::Midi {
                    channel: c1,
                    message: m1,
                },
                LiveEvent::Midi {
                    channel: c2,
                    message: m2,
                },
            ) => {
                if c1 != c2 {
                    return false;
                }
                match (m1, m2) {
                    (MidiMessage::NoteOn { key: k1, .. }, MidiMessage::NoteOn { key: k2, .. }) => {
                        k1 == k2
                    }
                    (
                        MidiMessage::NoteOff { key: k1, .. },
                        MidiMessage::NoteOff { key: k2, .. },
                    ) => k1 == k2,
                    (
                        MidiMessage::Controller {
                            controller: c1,
                            value: v1,
                        },
                        MidiMessage::Controller {
                            controller: c2,
                            value: v2,
                        },
                    ) => c1 == c2 && v1 == v2,
                    (
                        MidiMessage::ProgramChange { program: p1 },
                        MidiMessage::ProgramChange { program: p2 },
                    ) => p1 == p2,
                    (
                        MidiMessage::ChannelAftertouch { vel: v1 },
                        MidiMessage::ChannelAftertouch { vel: v2 },
                    ) => v1 == v2,
                    (
                        MidiMessage::Aftertouch { key: k1, vel: v1 },
                        MidiMessage::Aftertouch { key: k2, vel: v2 },
                    ) => k1 == k2 && v1 == v2,
                    (MidiMessage::PitchBend { bend: b1 }, MidiMessage::PitchBend { bend: b2 }) => {
                        b1 == b2
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Extracts velocity from a MIDI event.
    fn extract_velocity(&self, event: &LiveEvent) -> u8 {
        match event {
            LiveEvent::Midi { message, .. } => match message {
                MidiMessage::NoteOn { vel, .. } => u8::from(*vel),
                MidiMessage::NoteOff { vel, .. } => u8::from(*vel),
                MidiMessage::Aftertouch { vel, .. } => u8::from(*vel),
                MidiMessage::ChannelAftertouch { vel } => u8::from(*vel),
                MidiMessage::Controller { value, .. } => u8::from(*value),
                _ => 127, // Default to max for events without velocity
            },
            _ => 127,
        }
    }

    /// Builds a release group string for a MIDI note event.
    fn midi_release_group(channel_1indexed: u8, note: u8) -> String {
        format!("midi:{}:{}", channel_1indexed, note)
    }

    /// Extracts note and channel from a MIDI event for Note Off matching.
    fn extract_note_channel(&self, event: &LiveEvent) -> Option<(u8, u8)> {
        match event {
            LiveEvent::Midi {
                channel,
                message: MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. },
            } => Some((u8::from(*key), u8::from(*channel) + 1)),
            _ => None,
        }
    }

    /// Stops all sample playback.
    pub fn stop_all(&self) {
        let to_stop = {
            let mut vm = self.voice_manager.write();
            vm.clear()
        };

        // Cancel all voices via their handles (lock-free)
        let stopped_count = to_stop.len();
        for handle in to_stop {
            handle.cancel();
        }

        if stopped_count > 0 {
            info!(stopped = stopped_count, "All samples stopped");
        }
    }

    /// Returns the number of active voices.
    pub fn active_voice_count(&self) -> usize {
        self.voice_manager.read().active_count()
    }

    /// Returns the total memory used by loaded samples.
    pub fn memory_usage(&self) -> usize {
        self.loader.total_memory_usage()
    }
}

impl std::fmt::Debug for SampleEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleEngine")
            .field("samples", &self.samples.len())
            .field("triggers", &self.triggers.len())
            .field("active_voices", &self.active_voice_count())
            .field("memory_kb", &(self.memory_usage() / 1024))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::mixer::AudioMixer;

    fn create_test_mixer_and_sender() -> (Arc<AudioMixer>, crate::audio::SourceSender) {
        let mixer = Arc::new(AudioMixer::new(2, 44100));
        let (tx, _rx) = crossbeam_channel::unbounded();
        (mixer, tx)
    }

    fn make_samples_config(
        sample_name: &str,
        file: &str,
        trigger_channel: u8,
        trigger_note: u8,
    ) -> SamplesConfig {
        let mut samples = HashMap::new();
        samples.insert(
            sample_name.to_string(),
            SampleDefinition::new(
                Some(file.to_string()),
                vec![1, 2],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::PlayToCompletion,
                crate::config::samples::RetriggerBehavior::Cut,
                None,
                50,
            ),
        );
        let triggers = vec![SampleTrigger::new(
            crate::config::midi::note_on(trigger_channel, trigger_note, 127),
            sample_name.to_string(),
        )];
        SamplesConfig::new(samples, triggers, 32)
    }

    fn create_loaded_engine() -> SampleEngine {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
        let config = make_samples_config("kick", "1Channel44.1k.wav", 10, 36);
        engine.load_global_config(&config, &base_path).unwrap();
        engine
    }

    #[test]
    fn test_trigger_fires_sample() {
        let engine = create_loaded_engine();

        let event = TriggerEvent {
            sample_name: "kick".to_string(),
            velocity: 100,
            release_group: Some("test:group".to_string()),
        };

        engine.trigger(&event);

        assert_eq!(engine.active_voice_count(), 1);
    }

    #[test]
    fn test_trigger_unknown_sample_is_noop() {
        let engine = create_loaded_engine();

        let event = TriggerEvent {
            sample_name: "nonexistent".to_string(),
            velocity: 100,
            release_group: None,
        };

        engine.trigger(&event);

        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_trigger_cut_retrigger() {
        let engine = create_loaded_engine();

        let event = TriggerEvent {
            sample_name: "kick".to_string(),
            velocity: 100,
            release_group: Some("midi:10:36".to_string()),
        };

        engine.trigger(&event);
        engine.trigger(&event);

        // Cut retrigger: second trigger should replace the first
        assert_eq!(engine.active_voice_count(), 1);
    }

    #[test]
    fn test_release_stops_voices() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        // Use Stop release behavior so release actually stops the voice
        let mut samples = HashMap::new();
        samples.insert(
            "pad".to_string(),
            SampleDefinition::new(
                Some("1Channel44.1k.wav".to_string()),
                vec![1, 2],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::Stop,
                crate::config::samples::RetriggerBehavior::Polyphonic,
                None,
                50,
            ),
        );
        let config = SamplesConfig::new(samples, vec![], 32);
        engine.load_global_config(&config, &base_path).unwrap();

        let event = TriggerEvent {
            sample_name: "pad".to_string(),
            velocity: 100,
            release_group: Some("group:pad".to_string()),
        };

        engine.trigger(&event);
        assert_eq!(engine.active_voice_count(), 1);

        engine.release("group:pad");
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_release_nonexistent_group_is_noop() {
        let engine = create_loaded_engine();

        let event = TriggerEvent {
            sample_name: "kick".to_string(),
            velocity: 100,
            release_group: Some("midi:10:36".to_string()),
        };
        engine.trigger(&event);

        engine.release("nonexistent");
        assert_eq!(engine.active_voice_count(), 1);
    }

    #[test]
    fn test_stop_all() {
        let engine = create_loaded_engine();

        let event = TriggerEvent {
            sample_name: "kick".to_string(),
            velocity: 100,
            release_group: None,
        };

        engine.trigger(&event);
        engine.trigger(&event);
        // With Cut retrigger, only 1 voice remains
        assert_eq!(engine.active_voice_count(), 1);

        engine.stop_all();
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_stop_all_empty_is_noop() {
        let engine = create_loaded_engine();
        engine.stop_all();
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_process_midi_event_triggers_sample() {
        let engine = create_loaded_engine();

        // NoteOn on channel 10 (0-indexed: 9), note 36
        let raw = [0x99, 36, 100]; // 0x90 | 9 = 0x99
        engine.process_midi_event(&raw);

        assert_eq!(engine.active_voice_count(), 1);
    }

    #[test]
    fn test_process_midi_event_note_off_releases() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        let mut samples = HashMap::new();
        samples.insert(
            "pad".to_string(),
            SampleDefinition::new(
                Some("1Channel44.1k.wav".to_string()),
                vec![1, 2],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::Stop,
                crate::config::samples::RetriggerBehavior::Polyphonic,
                None,
                50,
            ),
        );
        let triggers = vec![SampleTrigger::new(
            crate::config::midi::note_on(10, 36, 127),
            "pad".to_string(),
        )];
        let config = SamplesConfig::new(samples, triggers, 32);
        engine.load_global_config(&config, &base_path).unwrap();

        // Trigger with NoteOn
        let note_on = [0x99, 36, 100]; // channel 10 (0-indexed 9), note 36
        engine.process_midi_event(&note_on);
        assert_eq!(engine.active_voice_count(), 1);

        // Release with NoteOff
        let note_off = [0x89, 36, 0]; // NoteOff channel 10 (0-indexed 9), note 36
        engine.process_midi_event(&note_off);
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_process_midi_event_vel0_as_note_off() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        let mut samples = HashMap::new();
        samples.insert(
            "pad".to_string(),
            SampleDefinition::new(
                Some("1Channel44.1k.wav".to_string()),
                vec![1, 2],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::Stop,
                crate::config::samples::RetriggerBehavior::Polyphonic,
                None,
                50,
            ),
        );
        let triggers = vec![SampleTrigger::new(
            crate::config::midi::note_on(10, 36, 127),
            "pad".to_string(),
        )];
        let config = SamplesConfig::new(samples, triggers, 32);
        engine.load_global_config(&config, &base_path).unwrap();

        // Trigger with NoteOn
        let note_on = [0x99, 36, 100];
        engine.process_midi_event(&note_on);
        assert_eq!(engine.active_voice_count(), 1);

        // Release with NoteOn velocity 0 (treated as NoteOff)
        let note_on_vel0 = [0x99, 36, 0];
        engine.process_midi_event(&note_on_vel0);
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_process_midi_event_invalid_data() {
        let engine = create_loaded_engine();

        // Invalid MIDI data
        let invalid = [0xFF, 0xFF];
        engine.process_midi_event(&invalid);

        // Should not crash, no voices created
        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_process_midi_event_no_matching_trigger() {
        let engine = create_loaded_engine();

        // NoteOn on a different note than the trigger
        let raw = [0x99, 60, 100]; // channel 10, note 60 (trigger is note 36)
        engine.process_midi_event(&raw);

        assert_eq!(engine.active_voice_count(), 0);
    }

    #[test]
    fn test_load_sample_with_max_voices() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        let mut samples = HashMap::new();
        samples.insert(
            "limited".to_string(),
            SampleDefinition::new(
                Some("1Channel44.1k.wav".to_string()),
                vec![1],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::PlayToCompletion,
                crate::config::samples::RetriggerBehavior::Polyphonic,
                Some(2), // Max 2 voices
                50,
            ),
        );
        let config = SamplesConfig::new(samples, vec![], 32);
        engine.load_global_config(&config, &base_path).unwrap();

        // Trigger 3 times — only 2 should remain active (oldest stolen)
        for _ in 0..3 {
            let event = TriggerEvent {
                sample_name: "limited".to_string(),
                velocity: 100,
                release_group: None,
            };
            engine.trigger(&event);
        }

        assert_eq!(engine.active_voice_count(), 2);
    }

    #[test]
    fn test_prepare_sample_absolute_path() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());

        // Use an absolute path to the asset file
        let abs_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("1Channel44.1k.wav");
        let abs_path_str = abs_path.to_string_lossy().to_string();

        let definition = SampleDefinition::new(
            Some(abs_path_str.clone()),
            vec![1, 2],
            crate::config::samples::VelocityConfig::ignore(None),
            ReleaseBehavior::PlayToCompletion,
            crate::config::samples::RetriggerBehavior::Cut,
            None,
            50,
        );

        // Use a different base_path than where the file is
        let base_path = PathBuf::from("/tmp");
        engine
            .load_sample("abs_sample", &definition, &base_path)
            .unwrap();

        // The file should have been loaded using the absolute path, not base_path.join
        let prepared = engine.prepare_sample("abs_sample", 100);
        assert!(prepared.is_some());
    }

    #[test]
    fn test_prepare_sample_not_found() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());

        // Try to prepare a sample that hasn't been loaded
        let prepared = engine.prepare_sample("nonexistent", 100);
        assert!(prepared.is_none());
    }

    #[test]
    fn test_prepare_sample_no_file_for_velocity() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        // Create a sample with velocity layers that don't cover all velocities
        let layers = vec![crate::config::samples::VelocityLayer::new(
            [10, 50],
            "1Channel44.1k.wav".to_string(),
        )];
        let definition = SampleDefinition::new(
            None,
            vec![1],
            crate::config::samples::VelocityConfig::with_layers(layers, false),
            ReleaseBehavior::PlayToCompletion,
            crate::config::samples::RetriggerBehavior::Cut,
            None,
            50,
        );

        engine
            .load_sample("layered", &definition, &base_path)
            .unwrap();

        // Velocity 5 is below the layer range [10, 50] — no file found
        let prepared = engine.prepare_sample("layered", 5);
        assert!(prepared.is_none());

        // Velocity 100 is above the layer range — no file found
        let prepared = engine.prepare_sample("layered", 100);
        assert!(prepared.is_none());

        // Velocity 30 is within the range — should find a file
        let prepared = engine.prepare_sample("layered", 30);
        assert!(prepared.is_some());
    }

    #[test]
    fn test_trigger_matching_non_midi_event_returns_false() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());

        let trigger = LiveEvent::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn {
                key: 60.into(),
                vel: 127.into(),
            },
        };

        // SysEx event should not match any MIDI trigger
        let sysex_event = LiveEvent::Common(midly::live::SystemCommon::Undefined(0xF4, &[]));
        assert!(!engine.matches_trigger(&sysex_event, &trigger));
    }

    #[test]
    fn test_extract_note_channel_non_note_returns_none() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());

        // PitchBend is not a note event
        let event = LiveEvent::Midi {
            channel: 0.into(),
            message: MidiMessage::PitchBend {
                bend: midly::PitchBend(midly::num::u14::from(8192)),
            },
        };
        assert_eq!(engine.extract_note_channel(&event), None);

        // ProgramChange is not a note event
        let event2 = LiveEvent::Midi {
            channel: 0.into(),
            message: MidiMessage::ProgramChange { program: 5.into() },
        };
        assert_eq!(engine.extract_note_channel(&event2), None);
    }

    #[test]
    fn test_extract_velocity_non_midi_defaults_to_max() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());

        let event = LiveEvent::Common(midly::live::SystemCommon::Undefined(0xF4, &[]));
        assert_eq!(engine.extract_velocity(&event), 127);
    }

    #[test]
    fn test_process_midi_event_channel_aftertouch_trigger() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        let mut samples = HashMap::new();
        samples.insert(
            "pad".to_string(),
            SampleDefinition::new(
                Some("1Channel44.1k.wav".to_string()),
                vec![1, 2],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::PlayToCompletion,
                crate::config::samples::RetriggerBehavior::Cut,
                None,
                50,
            ),
        );

        // Create a ChannelAftertouch trigger via deserialization (fields are private)
        let trigger_event: crate::config::midi::Event = serde_json::from_value(serde_json::json!({
            "type": "channel_aftertouch",
            "channel": 1,
            "velocity": 100
        }))
        .unwrap();
        let triggers = vec![SampleTrigger::new(trigger_event, "pad".to_string())];
        let config = SamplesConfig::new(samples, triggers, 32);
        engine.load_global_config(&config, &base_path).unwrap();

        // Send a ChannelAftertouch MIDI event: status byte 0xD0 | channel 0 = 0xD0, value 100
        let raw = [0xD0, 100];
        engine.process_midi_event(&raw);

        assert_eq!(engine.active_voice_count(), 1);
    }

    #[test]
    fn test_process_midi_event_aftertouch_trigger() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let mut engine = SampleEngine::new(mixer, source_tx, 32, 256, HashMap::new());
        let base_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");

        let mut samples = HashMap::new();
        samples.insert(
            "fx".to_string(),
            SampleDefinition::new(
                Some("1Channel44.1k.wav".to_string()),
                vec![1, 2],
                crate::config::samples::VelocityConfig::ignore(None),
                ReleaseBehavior::PlayToCompletion,
                crate::config::samples::RetriggerBehavior::Cut,
                None,
                50,
            ),
        );

        // Create an Aftertouch trigger (polyphonic key pressure) via deserialization
        let trigger_event: crate::config::midi::Event = serde_json::from_value(serde_json::json!({
            "type": "aftertouch",
            "channel": 1,
            "key": 60,
            "velocity": 80
        }))
        .unwrap();
        let triggers = vec![SampleTrigger::new(trigger_event, "fx".to_string())];
        let config = SamplesConfig::new(samples, triggers, 32);
        engine.load_global_config(&config, &base_path).unwrap();

        // Send an Aftertouch MIDI event: status byte 0xA0 | channel 0 = 0xA0, key 60, vel 80
        let raw = [0xA0, 60, 80];
        engine.process_midi_event(&raw);

        assert_eq!(engine.active_voice_count(), 1);
    }
}
