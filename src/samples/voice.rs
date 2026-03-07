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
//! Handles voice allocation, stealing, and release group behavior.
//! Voice management is source-agnostic — it works with any trigger source
//! (MIDI, audio triggers, etc.) via generic release groups.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, warn};

use crate::config::samples::{ReleaseBehavior, RetriggerBehavior};
use crate::playsync::CancelHandle;

/// Global voice ID counter.
static NEXT_VOICE_ID: AtomicU64 = AtomicU64::new(1);

/// Represents an active voice playing a sample.
pub(super) struct Voice {
    /// Unique ID for this voice.
    id: u64,
    /// The sample name being played.
    sample_name: String,
    /// Optional release group for voice management (e.g. "midi:10:36", "kick").
    /// Voices in the same release group can be released together.
    release_group: Option<String>,
    /// What to do when this voice's release group is released.
    release_behavior: ReleaseBehavior,
    /// When this voice started playing.
    start_time: Instant,
    /// The audio source ID in the mixer (used for testing/debugging).
    #[allow(dead_code)]
    mixer_source_id: u64,
    /// Cancel handle for stopping this voice without lock contention.
    cancel_handle: CancelHandle,
    /// Scheduled sample at which this voice should stop (for sample-accurate cuts).
    cancel_at_sample: Arc<AtomicU64>,
    /// Shared flag set by the mixer when the audio source finishes playing.
    is_finished: Arc<AtomicBool>,
}

impl Voice {
    /// Creates a new voice.
    pub(super) fn new(
        sample_name: String,
        release_group: Option<String>,
        release_behavior: ReleaseBehavior,
        mixer_source_id: u64,
        cancel_handle: CancelHandle,
        cancel_at_sample: Arc<AtomicU64>,
        is_finished: Arc<AtomicBool>,
    ) -> Self {
        Self {
            id: NEXT_VOICE_ID.fetch_add(1, Ordering::Relaxed),
            sample_name,
            release_group,
            release_behavior,
            start_time: Instant::now(),
            mixer_source_id,
            cancel_handle,
            cancel_at_sample,
            is_finished,
        }
    }

    /// Checks if this voice belongs to the given release group.
    fn matches_release(&self, group: &str) -> bool {
        self.release_group.as_deref() == Some(group)
    }

    /// Returns a clone of this voice's cancel handle.
    fn cancel_handle(&self) -> CancelHandle {
        self.cancel_handle.clone()
    }

    /// Returns a clone of this voice's cancel_at_sample.
    fn cancel_at_sample(&self) -> Arc<AtomicU64> {
        self.cancel_at_sample.clone()
    }
}

/// Manages active voices for sample playback.
pub(super) struct VoiceManager {
    /// Active voices.
    voices: Vec<Voice>,
    /// Global maximum voices limit.
    max_voices: u32,
    /// Per-sample voice limits (sample_name -> max_voices).
    sample_limits: HashMap<String, u32>,
}

impl VoiceManager {
    /// Creates a new voice manager.
    pub(super) fn new(max_voices: u32) -> Self {
        Self {
            voices: Vec::new(),
            max_voices,
            sample_limits: HashMap::new(),
        }
    }

    /// Sets the per-sample voice limit.
    pub(super) fn set_sample_limit(&mut self, sample_name: &str, limit: u32) {
        self.sample_limits.insert(sample_name.to_string(), limit);
    }

    /// Removes voices whose audio source has finished playing in the mixer.
    /// This prevents PlayToCompletion voices from accumulating indefinitely.
    fn sweep_finished(&mut self) {
        self.voices
            .retain(|v| !v.is_finished.load(Ordering::Relaxed));
    }

    /// Adds a new voice, potentially stealing old voices if limits are exceeded.
    /// Returns the cancel_at_sample Arcs for any voices that should be stopped.
    /// The caller can set these to schedule the stop at a specific sample time.
    pub(super) fn add_voice(
        &mut self,
        voice: Voice,
        retrigger: RetriggerBehavior,
    ) -> Vec<Arc<AtomicU64>> {
        // Sweep finished voices before checking limits.
        self.sweep_finished();

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

    /// Releases voices matching the given release group.
    /// Each voice's own ReleaseBehavior determines whether it is stopped.
    /// Returns the cancel handles for voices that should be stopped or faded.
    pub(super) fn release(&mut self, group: &str) -> Vec<CancelHandle> {
        let mut to_stop = Vec::new();

        for v in self.voices.iter() {
            if v.matches_release(group) {
                match v.release_behavior {
                    ReleaseBehavior::PlayToCompletion => {
                        // Let this voice play to completion
                    }
                    ReleaseBehavior::Stop | ReleaseBehavior::Fade => {
                        // Note: Fade currently behaves like Stop (immediate stop, no fade-out)
                        to_stop.push(v.cancel_handle());
                    }
                }
            }
        }

        // Remove only the voices that were stopped (not PlayToCompletion ones)
        self.voices.retain(|v| {
            !v.matches_release(group) || v.release_behavior == ReleaseBehavior::PlayToCompletion
        });

        to_stop
    }

    /// Returns the current number of active voices.
    pub(super) fn active_count(&self) -> usize {
        self.voices.len()
    }

    /// Clears all voices.
    /// Returns the cancel handles for all voices that should be stopped.
    pub(super) fn clear(&mut self) -> Vec<CancelHandle> {
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

    fn make_voice(sample: &str, release_group: Option<&str>, id: u64) -> Voice {
        make_voice_with_behavior(sample, release_group, id, ReleaseBehavior::Stop)
    }

    fn make_voice_with_behavior(
        sample: &str,
        release_group: Option<&str>,
        id: u64,
        release_behavior: ReleaseBehavior,
    ) -> Voice {
        Voice::new(
            sample.to_string(),
            release_group.map(|s| s.to_string()),
            release_behavior,
            id,
            CancelHandle::new(),
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn test_voice_release_matching() {
        let voice = make_voice("test", Some("midi:10:60"), 1);

        assert!(voice.matches_release("midi:10:60"));
        assert!(!voice.matches_release("midi:10:61"));
        assert!(!voice.matches_release("midi:11:60"));

        // Voice without release group matches nothing
        let voice2 = make_voice("test", None, 2);
        assert!(!voice2.matches_release("midi:10:60"));
        assert!(!voice2.matches_release("anything"));
    }

    #[test]
    fn test_voice_manager_cut_retrigger() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("kick", Some("midi:10:36"), 1);
        let stopped = manager.add_voice(voice1, RetriggerBehavior::Cut);
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 1);

        // Add another voice for the same sample - should cut the previous
        let voice2 = make_voice("kick", Some("midi:10:36"), 2);
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
            let voice = make_voice("snare", Some("midi:10:38"), i);
            let stopped = manager.add_voice(voice, RetriggerBehavior::Polyphonic);
            assert!(stopped.is_empty());
        }
        assert_eq!(manager.active_count(), 4);

        // Add 5th voice - should steal oldest
        let voice5 = make_voice("snare", Some("midi:10:38"), 5);
        let stopped = manager.add_voice(voice5, RetriggerBehavior::Polyphonic);
        assert_eq!(stopped.len(), 1); // Voice 1 was oldest
        assert_eq!(manager.active_count(), 4);
    }

    #[test]
    fn test_voice_manager_global_limit() {
        let mut manager = VoiceManager::new(3);

        for i in 1..=3 {
            let voice = make_voice(&format!("sample{}", i), Some("midi:10:36"), i);
            let stopped = manager.add_voice(voice, RetriggerBehavior::Polyphonic);
            assert!(stopped.is_empty());
        }

        // Add 4th voice - should steal oldest globally
        let voice4 = make_voice("sample4", Some("midi:10:36"), 4);
        let stopped = manager.add_voice(voice4, RetriggerBehavior::Polyphonic);
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 3);
    }

    #[test]
    fn test_release_stop() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("kick", Some("midi:10:36"), 1);
        let voice2 = make_voice("snare", Some("midi:10:38"), 2);
        manager.add_voice(voice1, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);

        // Release for kick group should stop only the kick
        let stopped = manager.release("midi:10:36");
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_release_play_to_completion() {
        let mut manager = VoiceManager::new(32);

        let voice = make_voice_with_behavior(
            "kick",
            Some("midi:10:36"),
            1,
            ReleaseBehavior::PlayToCompletion,
        );
        manager.add_voice(voice, RetriggerBehavior::Polyphonic);

        // Voice with PlayToCompletion should not be stopped on release
        let stopped = manager.release("midi:10:36");
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_release_custom_group() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("cymbal", Some("cymbal"), 1);
        let voice2 = make_voice("cymbal", Some("cymbal"), 2);
        let voice3 = make_voice("kick", Some("kick"), 3);
        manager.add_voice(voice1, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice3, RetriggerBehavior::Polyphonic);

        // Release cymbal group should stop both cymbal voices
        let stopped = manager.release("cymbal");
        assert_eq!(stopped.len(), 2);
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_voice_manager_clear() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("kick", Some("midi:10:36"), 1);
        let voice2 = make_voice("snare", Some("midi:10:38"), 2);
        manager.add_voice(voice1, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);
        assert_eq!(manager.active_count(), 2);

        let handles = manager.clear();
        assert_eq!(handles.len(), 2);
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn test_voice_manager_debug_format() {
        let manager = VoiceManager::new(16);
        let debug_str = format!("{:?}", manager);
        assert!(debug_str.contains("VoiceManager"));
        assert!(debug_str.contains("active_voices"));
        assert!(debug_str.contains("max_voices"));
        assert!(debug_str.contains("16"));
    }

    #[test]
    fn test_voice_manager_sweep_finished() {
        let mut manager = VoiceManager::new(32);

        let is_finished = Arc::new(AtomicBool::new(false));
        let voice = Voice::new(
            "test".to_string(),
            None,
            ReleaseBehavior::Stop,
            1,
            CancelHandle::new(),
            Arc::new(AtomicU64::new(0)),
            is_finished.clone(),
        );
        manager.add_voice(voice, RetriggerBehavior::Polyphonic);
        assert_eq!(manager.active_count(), 1);

        // Mark as finished and add another voice (triggers sweep)
        is_finished.store(true, Ordering::Relaxed);
        let voice2 = make_voice("other", None, 2);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);
        // First voice should have been swept, only second remains
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_release_nonexistent_group() {
        let mut manager = VoiceManager::new(32);
        let voice = make_voice("kick", Some("midi:10:36"), 1);
        manager.add_voice(voice, RetriggerBehavior::Polyphonic);

        // Release a group that no voice belongs to
        let stopped = manager.release("nonexistent");
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_voice_without_release_group() {
        let mut manager = VoiceManager::new(32);
        let voice = make_voice("ambient", None, 1);
        manager.add_voice(voice, RetriggerBehavior::Polyphonic);

        // Releasing any group should not affect a voice with no release group
        let stopped = manager.release("midi:10:36");
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 1);
    }

    #[test]
    fn test_release_fade_behavior_stops_voice() {
        let mut manager = VoiceManager::new(32);

        let voice = make_voice_with_behavior("pad", Some("pad"), 1, ReleaseBehavior::Fade);
        manager.add_voice(voice, RetriggerBehavior::Polyphonic);

        // Fade currently behaves like Stop
        let stopped = manager.release("pad");
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn test_set_sample_limit_directly() {
        let mut manager = VoiceManager::new(32);
        manager.set_sample_limit("snare", 2);

        // Add 2 voices — both allowed
        let voice1 = make_voice("snare", None, 1);
        let voice2 = make_voice("snare", None, 2);
        assert!(manager
            .add_voice(voice1, RetriggerBehavior::Polyphonic)
            .is_empty());
        assert!(manager
            .add_voice(voice2, RetriggerBehavior::Polyphonic)
            .is_empty());

        // 3rd voice should steal oldest
        let voice3 = make_voice("snare", None, 3);
        let stopped = manager.add_voice(voice3, RetriggerBehavior::Polyphonic);
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 2);
    }

    #[test]
    fn test_polyphonic_no_sample_limit_respects_global() {
        let mut manager = VoiceManager::new(2);
        // No per-sample limit set

        let voice1 = make_voice("a", None, 1);
        let voice2 = make_voice("b", None, 2);
        assert!(manager
            .add_voice(voice1, RetriggerBehavior::Polyphonic)
            .is_empty());
        assert!(manager
            .add_voice(voice2, RetriggerBehavior::Polyphonic)
            .is_empty());

        // 3rd voice hits global limit
        let voice3 = make_voice("c", None, 3);
        let stopped = manager.add_voice(voice3, RetriggerBehavior::Polyphonic);
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 2);
    }

    #[test]
    fn test_cut_retrigger_different_sample_no_cut() {
        let mut manager = VoiceManager::new(32);

        let voice1 = make_voice("kick", None, 1);
        manager.add_voice(voice1, RetriggerBehavior::Cut);

        // Different sample name — should not cut the kick
        let voice2 = make_voice("snare", None, 2);
        let stopped = manager.add_voice(voice2, RetriggerBehavior::Cut);
        assert!(stopped.is_empty());
        assert_eq!(manager.active_count(), 2);
    }

    #[test]
    fn test_sweep_finished_multiple() {
        let mut manager = VoiceManager::new(32);

        let finished1 = Arc::new(AtomicBool::new(false));
        let finished2 = Arc::new(AtomicBool::new(false));
        let voice1 = Voice::new(
            "a".to_string(),
            None,
            ReleaseBehavior::Stop,
            1,
            CancelHandle::new(),
            Arc::new(AtomicU64::new(0)),
            finished1.clone(),
        );
        let voice2 = Voice::new(
            "b".to_string(),
            None,
            ReleaseBehavior::Stop,
            2,
            CancelHandle::new(),
            Arc::new(AtomicU64::new(0)),
            finished2.clone(),
        );
        let voice3 = make_voice("c", None, 3);

        manager.add_voice(voice1, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice3, RetriggerBehavior::Polyphonic);
        assert_eq!(manager.active_count(), 3);

        // Mark first two as finished
        finished1.store(true, Ordering::Relaxed);
        finished2.store(true, Ordering::Relaxed);

        // Adding a new voice triggers sweep
        let voice4 = make_voice("d", None, 4);
        manager.add_voice(voice4, RetriggerBehavior::Polyphonic);
        // Only voice3 and voice4 should remain
        assert_eq!(manager.active_count(), 2);
    }

    #[test]
    fn test_release_mixed_behaviors_in_same_group() {
        let mut manager = VoiceManager::new(32);

        // Two voices in the same group with different ReleaseBehaviors
        let voice1 =
            make_voice_with_behavior("ride", Some("cymbal"), 1, ReleaseBehavior::PlayToCompletion);
        let voice2 = make_voice_with_behavior("hi-hat", Some("cymbal"), 2, ReleaseBehavior::Stop);
        manager.add_voice(voice1, RetriggerBehavior::Polyphonic);
        manager.add_voice(voice2, RetriggerBehavior::Polyphonic);

        // Release should only stop the hi-hat (Stop), not the ride (PlayToCompletion)
        let stopped = manager.release("cymbal");
        assert_eq!(stopped.len(), 1);
        assert_eq!(manager.active_count(), 1); // ride still playing
    }
}
