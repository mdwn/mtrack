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

//! Voice management for polyphonic sample playback.
//!
//! Handles voice allocation, stealing, and note-off behavior.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tracing::{debug, warn};

use crate::config::samples::{NoteOffBehavior, RetriggerBehavior};
use crate::playsync::CancelHandle;

/// Global voice ID counter.
static NEXT_VOICE_ID: AtomicU64 = AtomicU64::new(1);

/// Represents an active voice playing a sample.
pub struct Voice {
    /// Unique ID for this voice.
    id: u64,
    /// The sample name being played.
    sample_name: String,
    /// The MIDI note that triggered this voice (for Note Off matching).
    trigger_note: Option<u8>,
    /// The MIDI channel that triggered this voice (for Note Off matching).
    trigger_channel: Option<u8>,
    /// When this voice started playing.
    start_time: Instant,
    /// The audio source ID in the mixer (used for testing/debugging).
    #[allow(dead_code)]
    mixer_source_id: u64,
    /// Cancel handle for stopping this voice without lock contention.
    cancel_handle: CancelHandle,
    /// Scheduled sample at which this voice should stop (for sample-accurate cuts).
    cancel_at_sample: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Voice {
    /// Creates a new voice.
    pub fn new(
        sample_name: String,
        trigger_note: Option<u8>,
        trigger_channel: Option<u8>,
        mixer_source_id: u64,
        cancel_handle: CancelHandle,
        cancel_at_sample: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
            id: NEXT_VOICE_ID.fetch_add(1, Ordering::SeqCst),
            sample_name,
            trigger_note,
            trigger_channel,
            start_time: Instant::now(),
            mixer_source_id,
            cancel_handle,
            cancel_at_sample,
        }
    }

    /// Checks if this voice matches a Note Off event.
    pub fn matches_note_off(&self, note: u8, channel: u8) -> bool {
        match (self.trigger_note, self.trigger_channel) {
            (Some(n), Some(c)) => n == note && c == channel,
            (Some(n), None) => n == note,
            _ => false,
        }
    }

    /// Returns a clone of this voice's cancel handle.
    pub fn cancel_handle(&self) -> CancelHandle {
        self.cancel_handle.clone()
    }

    /// Returns a clone of this voice's cancel_at_sample.
    pub fn cancel_at_sample(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        self.cancel_at_sample.clone()
    }
}

/// Manages active voices for sample playback.
pub struct VoiceManager {
    /// Active voices.
    voices: Vec<Voice>,
    /// Global maximum voices limit.
    max_voices: u32,
    /// Per-sample voice limits (sample_name -> max_voices).
    sample_limits: HashMap<String, u32>,
}

impl VoiceManager {
    /// Creates a new voice manager.
    pub fn new(max_voices: u32) -> Self {
        Self {
            voices: Vec::new(),
            max_voices,
            sample_limits: HashMap::new(),
        }
    }

    /// Sets the per-sample voice limit.
    pub fn set_sample_limit(&mut self, sample_name: &str, limit: u32) {
        self.sample_limits.insert(sample_name.to_string(), limit);
    }

    /// Adds a new voice, potentially stealing old voices if limits are exceeded.
    /// Returns the cancel_at_sample Arcs for any voices that should be stopped.
    /// The caller can set these to schedule the stop at a specific sample time.
    pub fn add_voice(
        &mut self,
        voice: Voice,
        retrigger: RetriggerBehavior,
    ) -> Vec<std::sync::Arc<std::sync::atomic::AtomicU64>> {
        let mut voices_to_stop = Vec::new();

        // Handle retrigger behavior
        match retrigger {
            RetriggerBehavior::Cut => {
                // Stop all existing voices for this sample
                for v in self.voices.iter() {
                    if v.sample_name == voice.sample_name {
                        voices_to_stop.push(v.cancel_at_sample());
                    }
                }
                self.voices.retain(|v| v.sample_name != voice.sample_name);
            }
            RetriggerBehavior::Polyphonic => {
                // Check per-sample limit
                if let Some(&limit) = self.sample_limits.get(&voice.sample_name) {
                    let count = self
                        .voices
                        .iter()
                        .filter(|v| v.sample_name == voice.sample_name)
                        .count();
                    if count >= limit as usize {
                        // Steal oldest voice for this sample
                        if let Some(oldest) = self
                            .voices
                            .iter()
                            .filter(|v| v.sample_name == voice.sample_name)
                            .min_by_key(|v| v.start_time)
                        {
                            voices_to_stop.push(oldest.cancel_at_sample());
                            let oldest_id = oldest.id;
                            self.voices.retain(|v| v.id != oldest_id);
                            debug!(
                                sample = voice.sample_name,
                                limit, "Per-sample voice limit reached, stealing oldest"
                            );
                        }
                    }
                }
            }
        }

        // Check global limit
        if self.voices.len() >= self.max_voices as usize {
            // Steal oldest voice globally
            if let Some(oldest) = self.voices.iter().min_by_key(|v| v.start_time) {
                voices_to_stop.push(oldest.cancel_at_sample());
                let oldest_id = oldest.id;
                self.voices.retain(|v| v.id != oldest_id);
                warn!(
                    max_voices = self.max_voices,
                    "Global voice limit reached, stealing oldest"
                );
            }
        }

        self.voices.push(voice);
        voices_to_stop
    }

    /// Handles a Note Off event for the specified note and channel.
    /// Returns the cancel handles for voices that should be stopped or faded.
    pub fn handle_note_off(
        &mut self,
        note: u8,
        channel: u8,
        behavior: NoteOffBehavior,
    ) -> Vec<CancelHandle> {
        let mut to_stop = Vec::new();

        match behavior {
            NoteOffBehavior::PlayToCompletion => {
                // Do nothing - let the sample play to completion
            }
            NoteOffBehavior::Stop | NoteOffBehavior::Fade => {
                // Find and remove matching voices
                // Note: Fade currently behaves like Stop (immediate stop, no fade-out)
                for v in self.voices.iter() {
                    if v.matches_note_off(note, channel) {
                        to_stop.push(v.cancel_handle());
                    }
                }
                self.voices.retain(|v| !v.matches_note_off(note, channel));
            }
        }

        to_stop
    }

    /// Returns the current number of active voices.
    pub fn active_count(&self) -> usize {
        self.voices.len()
    }

    /// Clears all voices.
    /// Returns the cancel handles for all voices that should be stopped.
    pub fn clear(&mut self) -> Vec<CancelHandle> {
        let handles: Vec<CancelHandle> = self.voices.iter().map(|v| v.cancel_handle()).collect();
        self.voices.clear();
        handles
    }
}

impl std::fmt::Debug for VoiceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceManager")
            .field("active_voices", &self.voices.len())
            .field("max_voices", &self.max_voices)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_voice(sample: &str, note: Option<u8>, channel: Option<u8>, id: u64) -> Voice {
        Voice::new(
            sample.to_string(),
            note,
            channel,
            id,
            CancelHandle::new(),
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        )
    }

    #[test]
    fn test_voice_note_off_matching() {
        let voice = make_voice("test", Some(60), Some(10), 1);

        assert!(voice.matches_note_off(60, 10));
        assert!(!voice.matches_note_off(61, 10));
        assert!(!voice.matches_note_off(60, 11));

        // Voice without channel should match any channel
        let voice2 = make_voice("test", Some(60), None, 2);
        assert!(voice2.matches_note_off(60, 10));
        assert!(voice2.matches_note_off(60, 5));
        assert!(!voice2.matches_note_off(61, 10));
    }

    #[test]
    fn test_voice_manager_cut_retrigger() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("kick", Some(36), Some(10), 1);
        let stopped = manager.add_voice(voice1, RetriggerBehavior::Cut);
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 1);

        // Add another voice for the same sample - should cut the previous
        let voice2 = make_voice("kick", Some(36), Some(10), 2);
        let stopped = manager.add_voice(voice2, RetriggerBehavior::Cut);
        assert_eq!(stopped.len(), 1); // One voice should be stopped
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_voice_manager_polyphonic() {
        let mut manager = VoiceManager::new(32);
        manager.set_sample_limit("snare", 4);

        // Add 4 voices - should all be allowed
        for i in 1..=4 {
            let voice = make_voice("snare", Some(38), Some(10), i);
            let stopped = manager.add_voice(voice, RetriggerBehavior::Polyphonic);
            assert!(stopped.is_empty());
        }
        assert_eq!(manager.active_count(), 4);

        // Add 5th voice - should steal oldest
        let voice5 = make_voice("snare", Some(38), Some(10), 5);
        let stopped = manager.add_voice(voice5, RetriggerBehavior::Polyphonic);
        assert_eq!(stopped.len(), 1); // Voice 1 was oldest
        assert_eq!(manager.active_count(), 4);
    }

    #[test]
    fn test_voice_manager_global_limit() {
        let mut manager = VoiceManager::new(3);

        for i in 1..=3 {
            let voice = make_voice(&format!("sample{}", i), Some(36), Some(10), i);
            let stopped = manager.add_voice(voice, RetriggerBehavior::Polyphonic);
            assert!(stopped.is_empty());
        }

        // Add 4th voice - should steal oldest globally
        let voice4 = make_voice("sample4", Some(36), Some(10), 4);
        let stopped = manager.add_voice(voice4, RetriggerBehavior::Polyphonic);
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 3);
    }

    #[test]
    fn test_note_off_stop() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("kick", Some(36), Some(10), 1);
        let voice2 = make_voice("snare", Some(38), Some(10), 2);
        manager.add_voice(voice1, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);

        // Note Off for kick should stop only the kick
        let stopped = manager.handle_note_off(36, 10, NoteOffBehavior::Stop);
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_note_off_play_to_completion() {
        let mut manager = VoiceManager::new(32);

        let voice = make_voice("kick", Some(36), Some(10), 1);
        manager.add_voice(voice, RetriggerBehavior::Polyphonic);

        // Note Off with PlayToCompletion should not stop anything
        let stopped = manager.handle_note_off(36, 10, NoteOffBehavior::PlayToCompletion);
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 1);
    }
}
