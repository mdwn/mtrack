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

use parking_lot::RwLock;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use midly::live::LiveEvent;
use midly::MidiMessage;
use tracing::{debug, error, info, warn};

use super::loader::{LoadedSample, SampleLoader};
use super::voice::{Voice, VoiceManager};
use crate::audio;
use crate::audio::sample_source::ChannelMappedSource;
use crate::config::samples::{NoteOffBehavior, SampleDefinition, SampleTrigger, SamplesConfig};
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
    mixer: std::sync::Arc<crate::audio::mixer::AudioMixer>,
    /// Fixed delay in samples for consistent trigger latency.
    fixed_delay_samples: u64,
}

impl SampleEngine {
    /// Creates a new sample engine.
    ///
    /// The `buffer_size` is used to calculate the minimum fixed delay for consistent
    /// trigger latency. The delay is set to 2x the buffer size to ensure samples are
    /// always scheduled ahead of the current mixing position.
    pub fn new(
        mixer: std::sync::Arc<crate::audio::mixer::AudioMixer>,
        source_tx: crate::audio::SourceSender,
        max_voices: u32,
        buffer_size: usize,
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
        let output_channels = definition.output_channels();
        let loaded_files: HashMap<PathBuf, PrecomputedSampleData> = raw_loaded_files
            .into_iter()
            .map(|(path, loaded)| {
                let channel_labels: Vec<Vec<String>> = (0..loaded.channel_count())
                    .map(|_| {
                        output_channels
                            .iter()
                            .map(|ch| format!("__sample_out_{}", ch))
                            .collect()
                    })
                    .collect();

                let track_mappings: HashMap<String, Vec<u16>> = output_channels
                    .iter()
                    .map(|ch| (format!("__sample_out_{}", ch), vec![*ch]))
                    .collect();

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
                    self.handle_note_off(u8::from(*key), u8::from(*channel) + 1);
                }
                MidiMessage::NoteOn { key, vel } if u8::from(*vel) == 0 => {
                    // Note On with velocity 0 is equivalent to Note Off
                    self.handle_note_off(u8::from(*key), u8::from(*channel) + 1);
                }
                _ => {}
            }
        }

        // Check against triggers (hot path - minimal overhead)
        for trigger in &self.triggers {
            if self.matches_trigger(&event, &trigger.midi_event) {
                let velocity = self.extract_velocity(&event);
                self.trigger_sample(&trigger.sample_name, velocity, &event);
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

    /// Triggers a sample by name with the given velocity.
    fn trigger_sample(&self, sample_name: &str, velocity: u8, event: &LiveEvent) {
        let sample = match self.samples.get(sample_name) {
            Some(s) => s,
            None => {
                warn!(sample = sample_name, "Sample not found");
                return;
            }
        };

        // Get the file to play based on velocity
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
                return;
            }
        };

        // Get the precomputed sample data
        let precomputed = match sample.loaded_files.get(&file_path) {
            Some(p) => p,
            None => {
                error!(
                    sample = sample_name,
                    path = ?file_path,
                    "Sample file not loaded"
                );
                return;
            }
        };

        // Create a new source for playback
        let source = precomputed.loaded.create_source(volume);
        let source_id = audio::next_source_id();

        // Use precomputed channel labels and track mappings (no allocations!)
        let channel_mapped = ChannelMappedSource::new(
            Box::new(source),
            precomputed.channel_labels.clone(),
            precomputed.loaded.channel_count(),
        );
        let track_mappings = precomputed.track_mappings.clone();

        // Extract note/channel for Note Off tracking
        let (trigger_note, trigger_channel) = self
            .extract_note_channel(event)
            .map(|(n, c)| (Some(n), Some(c)))
            .unwrap_or((None, None));

        // Create a per-source cancel handle and scheduled cancel time for lock-free voice stopping
        let source_cancel_handle = CancelHandle::new();
        let source_cancel_at_sample = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)); // 0 = no scheduled cancel

        // Create voice entry with its own cancel handle and scheduled cancel time
        let voice = Voice::new(
            sample_name.to_string(),
            trigger_note,
            trigger_channel,
            source_id,
            source_cancel_handle.clone(),
            source_cancel_at_sample.clone(),
        );

        // Schedule the source to start at a fixed delay from now for consistent latency
        let start_at_sample = self.mixer.current_sample() + self.fixed_delay_samples;

        // Acquire voice manager lock BEFORE adding to mixer to prevent race conditions
        // with concurrent triggers for the same sample (important for cut/monophonic mode)
        let mut vm = self.voice_manager.write();
        let to_stop = vm.add_voice(voice, sample.definition.retrigger());

        // Schedule old voices to stop at the same time the new one starts (sample-accurate cut)
        for cancel_at in to_stop {
            cancel_at.store(start_at_sample, std::sync::atomic::Ordering::Relaxed);
        }

        // Add the new source via channel (audio callback receives and adds it)
        let active_source = crate::audio::mixer::ActiveSource {
            id: source_id,
            source: Box::new(channel_mapped),
            track_mappings,
            channel_mappings: Vec::new(), // Will be computed by mixer
            cached_source_channel_count: precomputed.loaded.channel_count(),
            is_finished: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancel_handle: source_cancel_handle.clone(),
            start_at_sample: Some(start_at_sample),
            cancel_at_sample: Some(source_cancel_at_sample.clone()),
        };

        // Send via channel - audio callback handles addition to mixer
        if let Err(e) = self.source_tx.send(active_source) {
            error!(error = %e, "Failed to send sample to mixer");
        }
        drop(vm);

        debug!(
            sample = sample_name,
            velocity, volume, source_id, "Sample triggered"
        );
    }

    /// Handles a Note Off event.
    fn handle_note_off(&self, note: u8, channel: u8) {
        // Find all samples that might have Note Off behavior
        // and check their voices
        let sample_behaviors: Vec<_> = self
            .samples
            .iter()
            .map(|(name, s)| (name.clone(), s.definition.note_off()))
            .collect();

        for (name, behavior) in sample_behaviors {
            if behavior == NoteOffBehavior::PlayToCompletion {
                continue;
            }

            let mut vm = self.voice_manager.write();
            let to_stop = vm.handle_note_off(note, channel, behavior);
            drop(vm);

            let stopped_count = to_stop.len();
            if !to_stop.is_empty() {
                // Cancel voices via their handles (lock-free, no mixer write lock needed)
                for handle in to_stop {
                    handle.cancel();
                }
                debug!(
                    sample = name,
                    note,
                    channel,
                    stopped = stopped_count,
                    "Note Off handled"
                );
            }
        }
    }

    /// Stops all sample playback.
    pub fn stop_all(&self) {
        let mut vm = self.voice_manager.write();
        let to_stop = vm.clear();
        drop(vm);

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

    fn create_test_mixer_and_sender() -> (std::sync::Arc<AudioMixer>, crate::audio::SourceSender) {
        let mixer = std::sync::Arc::new(AudioMixer::new(2, 44100));
        let (tx, _rx) = crossbeam_channel::unbounded();
        (mixer, tx)
    }

    #[test]
    fn test_engine_creation() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256);

        assert_eq!(engine.active_voice_count(), 0);
        assert_eq!(engine.samples.len(), 0);
        assert_eq!(engine.triggers.len(), 0);
    }

    #[test]
    fn test_velocity_extraction() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256);

        // Note On
        let note_on = LiveEvent::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn {
                key: 60.into(),
                vel: 100.into(),
            },
        };
        assert_eq!(engine.extract_velocity(&note_on), 100);

        // CC
        let cc = LiveEvent::Midi {
            channel: 0.into(),
            message: MidiMessage::Controller {
                controller: 1.into(),
                value: 64.into(),
            },
        };
        assert_eq!(engine.extract_velocity(&cc), 64);
    }

    #[test]
    fn test_trigger_matching() {
        let (mixer, source_tx) = create_test_mixer_and_sender();
        let engine = SampleEngine::new(mixer, source_tx, 32, 256);

        let trigger = LiveEvent::Midi {
            channel: 9.into(), // Channel 10 (0-indexed)
            message: MidiMessage::NoteOn {
                key: 36.into(),
                vel: 127.into(),
            },
        };

        // Same note, different velocity - should match
        let event1 = LiveEvent::Midi {
            channel: 9.into(),
            message: MidiMessage::NoteOn {
                key: 36.into(),
                vel: 80.into(),
            },
        };

        // Different note - should not match
        let event2 = LiveEvent::Midi {
            channel: 9.into(),
            message: MidiMessage::NoteOn {
                key: 37.into(),
                vel: 127.into(),
            },
        };

        // Different channel - should not match
        let event3 = LiveEvent::Midi {
            channel: 0.into(),
            message: MidiMessage::NoteOn {
                key: 36.into(),
                vel: 127.into(),
            },
        };

        assert!(engine.matches_trigger(&event1, &trigger));
        assert!(!engine.matches_trigger(&event2, &trigger));
        assert!(!engine.matches_trigger(&event3, &trigger));
    }
}
