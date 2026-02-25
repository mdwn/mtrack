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

use crate::lighting::{
    parser::{Cue, Effect, LayerCommand, LayerCommandType, LightShow},
    EffectInstance,
};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Global atomic counter for generating unique, deterministic effect IDs.
static EFFECT_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Result of processing timeline cues
#[derive(Debug, Clone, Default)]
pub struct TimelineUpdate {
    /// Effects to be triggered
    pub effects: Vec<EffectInstance>,
    /// Effects to be triggered with pre-calculated elapsed time (for seeking)
    /// Maps effect ID to (effect, elapsed_time)
    pub effects_with_elapsed: std::collections::HashMap<String, (EffectInstance, Duration)>,
    /// Layer commands to be executed
    pub layer_commands: Vec<LayerCommand>,
    /// Names of sequences to stop
    pub stop_sequences: Vec<String>,
}

/// Timeline processor for lighting cues during song playback
pub struct LightingTimeline {
    /// Cues sorted by time (now using DSL Cue directly)
    cues: Vec<Cue>,
    /// Current position in the timeline
    current_time: Duration,
    /// Index of the next cue to process
    next_cue_index: usize,
    /// Whether the timeline is currently playing
    is_playing: bool,
    /// Tempo map for tempo-aware effects (from the first show that has one)
    tempo_map: Option<crate::lighting::tempo::TempoMap>,
    /// Sequences that have been stopped — future cues from these sequences are suppressed
    stopped_sequences: HashSet<String>,
}

impl LightingTimeline {
    /// Creates a new lighting timeline from DSL light shows
    pub fn new(shows: Vec<LightShow>) -> Self {
        let mut all_cues = Vec::new();
        let mut tempo_map: Option<crate::lighting::tempo::TempoMap> = None;

        // Extract cues and tempo map from shows
        // Use the first show's tempo map if available
        for show in shows {
            all_cues.extend(show.cues);
            // Use the first show's tempo map
            if tempo_map.is_none() {
                tempo_map = show.tempo_map;
            }
        }

        let mut timeline = Self::new_with_cues(all_cues);
        timeline.tempo_map = tempo_map;
        timeline
    }

    /// Creates a new lighting timeline from DSL cues (for testing)
    pub(crate) fn new_with_cues(cues: Vec<Cue>) -> Self {
        let mut timeline = Self {
            cues,
            current_time: Duration::ZERO,
            next_cue_index: 0,
            is_playing: false,
            tempo_map: None,
            stopped_sequences: HashSet::new(),
        };
        timeline.sort_cues();
        timeline
    }

    /// Get the tempo map for this timeline
    pub fn tempo_map(&self) -> Option<&crate::lighting::tempo::TempoMap> {
        self.tempo_map.as_ref()
    }

    /// Sorts cues by time
    fn sort_cues(&mut self) {
        self.cues.sort_by(|a, b| a.time.cmp(&b.time));
    }

    /// Starts the timeline
    pub fn start(&mut self) {
        self.is_playing = true;
        self.current_time = Duration::ZERO;
        self.next_cue_index = 0;
        self.stopped_sequences.clear();
    }

    /// Starts the timeline at a specific time (for seeking)
    /// This processes all cues before start_time to ensure deterministic state
    pub fn start_at(&mut self, start_time: Duration) -> TimelineUpdate {
        self.is_playing = true;
        self.current_time = start_time;
        self.stopped_sequences.clear();
        self.next_cue_index = self.find_cue_index_at(start_time);

        // Process all cues before start_time to ensure deterministic state
        // This applies layer commands and starts effects that should still be active
        let mut result = TimelineUpdate::default();

        for i in 0..self.next_cue_index {
            let cue = &self.cues[i];

            // Process start_sequences first: clear stopped status for new invocations
            for seq_name in &cue.start_sequences {
                self.stopped_sequences.remove(seq_name);
            }

            // Apply all layer commands from historical cues
            result.layer_commands.extend(cue.layer_commands.clone());

            // Simulate clear commands: purge effects that would have been stopped
            // so we only start effects that are actually active at the seek point
            for cmd in &cue.layer_commands {
                if cmd.command_type == LayerCommandType::Clear {
                    if let Some(layer) = cmd.layer {
                        result
                            .effects_with_elapsed
                            .retain(|_, (effect, _)| effect.layer != layer);
                    } else {
                        result.effects_with_elapsed.clear();
                    }
                }
            }

            process_stop_sequences(&mut self.stopped_sequences, cue, &mut result);

            // Purge already-accumulated effects from stopped sequences,
            // mirroring what EffectEngine::stop_sequence() does at runtime.
            for seq_name in &cue.stop_sequences {
                let prefix = format!("seq_{}_", seq_name);
                result
                    .effects_with_elapsed
                    .retain(|id, _| !id.starts_with(&prefix));
            }

            // For effects, only include ones that would still be active at start_time
            for effect in &cue.effects {
                // Skip effects from stopped sequences
                if let Some(ref seq_name) = effect.sequence_name {
                    if self.stopped_sequences.contains(seq_name) {
                        continue;
                    }
                }

                let effect_instance = Self::create_effect_instance(effect, cue.time);
                // Check if this effect would still be active at start_time
                let effect_start_time = cue.time;
                let elapsed_at_start = start_time.saturating_sub(effect_start_time);

                // Get the effect's total duration
                let should_include = if let Some(duration) = effect_instance.total_duration() {
                    // Timed effect - only include if it would still be running
                    elapsed_at_start < duration
                } else {
                    // Perpetual effect - always include if it was triggered before start_time
                    true
                };

                if should_include {
                    // Store the elapsed time in a map so we can start the effect at the correct point
                    result.effects_with_elapsed.insert(
                        effect_instance.id.clone(),
                        (effect_instance, elapsed_at_start),
                    );
                }
            }
        }

        result
    }

    /// Finds the index of the first cue that should trigger at or after the given time.
    /// Uses `partition_point` to handle duplicate cue times correctly —
    /// `binary_search` may land on any duplicate, but `partition_point`
    /// always returns the first cue with `time >= target`.
    fn find_cue_index_at(&self, time: Duration) -> usize {
        self.cues.partition_point(|cue| cue.time < time)
    }

    /// Stops the timeline
    pub fn stop(&mut self) {
        self.is_playing = false;
        self.current_time = Duration::ZERO;
        self.next_cue_index = 0;
        self.stopped_sequences.clear();
    }

    /// Returns true if all cues have been processed (including empty timelines)
    pub fn is_finished(&self) -> bool {
        self.next_cue_index >= self.cues.len()
    }

    /// Updates the timeline with the current song time
    /// Returns both effects and layer commands to be processed
    pub fn update(&mut self, song_time: Duration) -> TimelineUpdate {
        if !self.is_playing {
            return TimelineUpdate::default();
        }
        self.current_time = song_time;
        let mut result = TimelineUpdate::default();

        // Process all cues that should trigger at or before the current time
        while self.next_cue_index < self.cues.len() {
            let cue = &self.cues[self.next_cue_index];

            if cue.time <= song_time {
                // Process start_sequences first: clear stopped status for new invocations
                for seq_name in &cue.start_sequences {
                    self.stopped_sequences.remove(seq_name);
                }

                // Process effects, skipping those from stopped sequences
                for effect in &cue.effects {
                    if let Some(ref seq_name) = effect.sequence_name {
                        if self.stopped_sequences.contains(seq_name) {
                            continue;
                        }
                    }
                    let effect_instance = Self::create_effect_instance(effect, cue.time);
                    result.effects.push(effect_instance);
                }
                // Process layer commands
                result.layer_commands.extend(cue.layer_commands.clone());

                process_stop_sequences(&mut self.stopped_sequences, cue, &mut result);

                self.next_cue_index += 1;
            } else {
                // No more cues to process at this time
                break;
            }
        }

        result
    }

    /// Get all cues with their times and indices
    pub fn cues(&self) -> Vec<(Duration, usize)> {
        self.cues
            .iter()
            .enumerate()
            .map(|(index, cue)| (cue.time, index))
            .collect()
    }

    /// Creates an EffectInstance from a DSL Effect.
    /// cue_time is the song time when this effect was supposed to start (for deterministic randomness).
    pub fn create_effect_instance(effect: &Effect, cue_time: Duration) -> EffectInstance {
        let id = EFFECT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let effect_id = if let Some(ref seq_name) = effect.sequence_name {
            format!("seq_{}_effect_{}", seq_name, id)
        } else {
            format!("song_effect_{}", id)
        };

        let mut effect_instance = EffectInstance::new(
            effect_id,
            effect.effect_type.clone(),
            effect.groups.clone(),
            effect.up_time,
            effect.hold_time,
            effect.down_time,
        );

        // Store the cue time for deterministic randomness
        effect_instance.cue_time = Some(cue_time);

        // Apply layering information if specified in DSL
        if let Some(layer) = effect.layer {
            effect_instance.layer = layer;
        }
        if let Some(blend_mode) = effect.blend_mode {
            effect_instance.blend_mode = blend_mode;
        }

        effect_instance
    }
}

/// Process stop_sequences for a cue: iteration-boundary stops (where the cue
/// also starts effects from the same sequence) only go to the engine, while
/// explicit stops also suppress future cues from that sequence.
fn process_stop_sequences(
    stopped_sequences: &mut HashSet<String>,
    cue: &crate::lighting::parser::Cue,
    result: &mut TimelineUpdate,
) {
    let cue_sequence_names: HashSet<&String> = cue
        .effects
        .iter()
        .filter_map(|e| e.sequence_name.as_ref())
        .collect();
    for seq_name in &cue.stop_sequences {
        if !cue_sequence_names.contains(seq_name) {
            stopped_sequences.insert(seq_name.clone());
        }
    }
    result.stop_sequences.extend(cue.stop_sequences.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::EffectType;
    use crate::lighting::parser::Effect;
    use std::collections::HashMap;

    #[test]
    fn test_timeline_creation() {
        let cues = vec![];
        let timeline = LightingTimeline::new_with_cues(cues);
        assert_eq!(timeline.cues.len(), 0);
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_timeline_start_stop() {
        let cues = vec![];
        let mut timeline = LightingTimeline::new_with_cues(cues);

        assert!(!timeline.is_playing);
        timeline.start();
        assert!(timeline.is_playing);
        timeline.stop();
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_timeline_is_finished() {
        use crate::lighting::parser::Cue;

        // Empty timeline should be considered finished (prevents blocking forever)
        let empty_timeline = LightingTimeline::new_with_cues(vec![]);
        assert!(
            empty_timeline.is_finished(),
            "Empty timeline should be finished"
        );

        // Timeline with cues should not be finished initially
        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };
        let cues = vec![Cue {
            stop_sequences: vec![],
            start_sequences: vec![],
            time: Duration::from_millis(0),
            effects: vec![effect],
            layer_commands: vec![],
        }];
        let mut timeline = LightingTimeline::new_with_cues(cues);
        assert!(
            !timeline.is_finished(),
            "Timeline with unprocessed cues should not be finished"
        );

        // After processing all cues, should be finished
        timeline.start();
        let _ = timeline.update(Duration::from_secs(1));
        assert!(
            timeline.is_finished(),
            "Timeline should be finished after all cues processed"
        );
    }

    #[test]
    fn test_timeline_with_dsl_cues() {
        use crate::lighting::parser::Cue;

        // Create DSL cues directly
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());
        parameters.insert("dimmer".to_string(), "60%".to_string());

        let effect = Effect {
            sequence_name: None,
            groups: vec!["front_wash".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![Cue {
            stop_sequences: vec![],
            start_sequences: vec![],
            time: Duration::from_millis(0),
            effects: vec![effect],
            layer_commands: vec![],
        }];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Test that the first cue triggers at the right time
        let result = timeline.update(Duration::from_millis(0));
        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.effects[0].target_fixtures, vec!["front_wash"]);
    }

    #[test]
    fn test_timeline_cue_ordering() {
        use crate::lighting::parser::Cue;

        // Create cues in non-chronological order
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());

        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_millis(10000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(0),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Verify cues are processed in chronological order
        let result = timeline.update(Duration::from_millis(0));
        assert_eq!(result.effects.len(), 1);

        let result = timeline.update(Duration::from_millis(5000));
        assert_eq!(result.effects.len(), 1);

        let result = timeline.update(Duration::from_millis(10000));
        assert_eq!(result.effects.len(), 1);
    }

    #[test]
    fn test_timeline_edge_cases() {
        // Test with empty timeline
        let timeline = LightingTimeline::new_with_cues(vec![]);
        assert_eq!(timeline.cues.len(), 0);

        // Test with cues at the same time
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());

        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Both cues should trigger at the same time
        let result = timeline.update(Duration::from_millis(5000));
        assert_eq!(result.effects.len(), 2);
    }

    #[test]
    fn test_timeline_stop_reset() {
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "red".to_string());

        let effect1 = Effect {
            sequence_name: None,
            groups: vec!["fixture1".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let effect2 = Effect {
            sequence_name: None,
            groups: vec!["fixture2".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect1],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![effect2],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Start the timeline and process some cues
        timeline.start();
        let _effects_at_0s = timeline.update(Duration::from_secs(0));
        let _effects_at_2s = timeline.update(Duration::from_secs(2));

        // Stop the timeline - this should reset the cue index
        timeline.stop();

        // Start again - should trigger the first cue again
        timeline.start();
        let result_at_0s_restart = timeline.update(Duration::from_secs(0));
        assert_eq!(result_at_0s_restart.effects.len(), 1);

        // Should also trigger the second cue again
        let result_at_2s_restart = timeline.update(Duration::from_secs(2));
        assert_eq!(result_at_2s_restart.effects.len(), 1);
    }

    #[test]
    fn test_timeline_layer_commands() {
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        // Create cue with a layer command
        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: Some(EffectLayer::Foreground),
                    fade_time: None,
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(1),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Release,
                    layer: Some(EffectLayer::Background),
                    fade_time: Some(Duration::from_secs(2)),
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Master,
                    layer: Some(EffectLayer::Midground),
                    fade_time: None,
                    intensity: Some(0.5),
                    speed: Some(2.0),
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // First cue: clear command
        let result0 = timeline.update(Duration::from_secs(0));
        assert_eq!(result0.effects.len(), 0);
        assert_eq!(result0.layer_commands.len(), 1);
        assert_eq!(
            result0.layer_commands[0].command_type,
            LayerCommandType::Clear
        );
        assert_eq!(
            result0.layer_commands[0].layer,
            Some(EffectLayer::Foreground)
        );

        // Second cue: release command with fade time
        let result1 = timeline.update(Duration::from_secs(1));
        assert_eq!(result1.layer_commands.len(), 1);
        assert_eq!(
            result1.layer_commands[0].command_type,
            LayerCommandType::Release
        );
        assert_eq!(
            result1.layer_commands[0].fade_time,
            Some(Duration::from_secs(2))
        );

        // Third cue: master command with intensity and speed
        let result2 = timeline.update(Duration::from_secs(2));
        assert_eq!(result2.layer_commands.len(), 1);
        assert_eq!(
            result2.layer_commands[0].command_type,
            LayerCommandType::Master
        );
        assert!((result2.layer_commands[0].intensity.unwrap() - 0.5).abs() < 0.01);
        assert!((result2.layer_commands[0].speed.unwrap() - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_timeline_mixed_effects_and_layer_commands() {
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        // Create cue with both an effect and a layer command
        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![Cue {
            stop_sequences: vec![],
            start_sequences: vec![],
            time: Duration::from_secs(0),
            effects: vec![effect],
            layer_commands: vec![LayerCommand {
                command_type: LayerCommandType::Master,
                layer: Some(EffectLayer::Background),
                fade_time: None,
                intensity: Some(0.75),
                speed: None,
            }],
        }];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        let result = timeline.update(Duration::from_secs(0));

        // Should have both an effect and a layer command
        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.layer_commands.len(), 1);
        assert_eq!(
            result.layer_commands[0].command_type,
            LayerCommandType::Master
        );
    }

    #[test]
    fn test_timeline_start_at() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Start at 5 seconds - should skip the first cue
        let _historical_update = timeline.start_at(Duration::from_secs(5));
        assert!(timeline.is_playing);
        assert_eq!(timeline.current_time, Duration::from_secs(5));

        // Update at 5 seconds - should trigger the cue at 5s
        let result = timeline.update(Duration::from_secs(5));
        assert_eq!(result.effects.len(), 1);

        // Update at 10 seconds - should trigger the cue at 10s
        let result = timeline.update(Duration::from_secs(10));
        assert_eq!(result.effects.len(), 1);
    }

    #[test]
    fn test_timeline_find_cue_index_at() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Test finding cue index at different times
        timeline.start_at(Duration::from_secs(0));
        assert_eq!(timeline.next_cue_index, 0);

        timeline.start_at(Duration::from_secs(3));
        // Should point to cue at 5s (first cue >= 3s)
        assert_eq!(timeline.next_cue_index, 1);

        timeline.start_at(Duration::from_secs(5));
        assert_eq!(timeline.next_cue_index, 1);

        timeline.start_at(Duration::from_secs(7));
        // Should point to cue at 10s (first cue >= 7s)
        assert_eq!(timeline.next_cue_index, 2);

        timeline.start_at(Duration::from_secs(15));
        // Should point past the end
        assert_eq!(timeline.next_cue_index, 3);
    }

    #[test]
    fn test_timeline_cue_listing() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let timeline = LightingTimeline::new_with_cues(cues);

        // Test cue listing
        let cue_list = timeline.cues();
        assert_eq!(cue_list.len(), 3);
        assert_eq!(cue_list[0], (Duration::from_secs(0), 0));
        assert_eq!(cue_list[1], (Duration::from_secs(5), 1));
        assert_eq!(cue_list[2], (Duration::from_secs(10), 2));
    }

    #[test]
    fn test_start_at_clears_purge_old_effects() {
        // Verify that seeking past a clear() command does NOT include
        // effects from before the clear in the historical update.
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        let bg_effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: Some(EffectLayer::Background),
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let fg_effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: Some(EffectLayer::Foreground),
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start perpetual effects on bg and fg
            Cue {
                time: Duration::from_secs(0),
                effects: vec![bg_effect.clone(), fg_effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @5s: clear foreground only
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: Some(EffectLayer::Foreground),
                    fade_time: None,
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @10s: start a new fg effect
            Cue {
                time: Duration::from_secs(10),
                effects: vec![fg_effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 12s — past the clear at 5s and the new effect at 10s
        let update = timeline.start_at(Duration::from_secs(12));

        // Should have 2 effects: the bg from @0s and the new fg from @10s
        // The old fg from @0s should have been purged by the clear at @5s
        assert_eq!(
            update.effects_with_elapsed.len(),
            2,
            "Should have 2 effects (bg from @0s + new fg from @10s), got {}: {:?}",
            update.effects_with_elapsed.len(),
            update
                .effects_with_elapsed
                .values()
                .map(|(e, _)| &e.id)
                .collect::<Vec<_>>()
        );

        // Verify the bg effect is present with 12s elapsed
        let bg = update
            .effects_with_elapsed
            .values()
            .find(|(e, _)| e.layer == EffectLayer::Background);
        assert!(bg.is_some(), "Background effect should be present");
        assert_eq!(bg.unwrap().1, Duration::from_secs(12));

        // Verify the new fg effect is present with 2s elapsed (started at @10s)
        let fg = update
            .effects_with_elapsed
            .values()
            .find(|(e, _)| e.layer == EffectLayer::Foreground);
        assert!(fg.is_some(), "Foreground effect should be present");
        assert_eq!(fg.unwrap().1, Duration::from_secs(2));
    }

    #[test]
    fn test_start_at_clear_all_purges_all_effects() {
        // Verify that seeking past a clear() (all layers) purges everything
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        let effect = Effect {
            sequence_name: None,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start effects
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone(), effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @5s: clear all layers
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: None,
                    fade_time: None,
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @10s: start one new effect
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        let update = timeline.start_at(Duration::from_secs(12));

        // Only the effect from @10s should survive (the 2 from @0s were cleared at @5s)
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "Should have 1 effect after clear-all, got {}",
            update.effects_with_elapsed.len()
        );
    }

    #[test]
    fn test_start_at_stopped_sequences_suppresses_future_effects() {
        // Verify that seeking past a stop_sequence command suppresses
        // effects from that sequence at and after the stop point.
        use crate::lighting::parser::Cue;

        let seq_effect = |seq: &str| Effect {
            sequence_name: Some(seq.to_string()),
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start effects from sequence A and B
            Cue {
                time: Duration::from_secs(0),
                effects: vec![seq_effect("seqA"), seq_effect("seqB")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @5s: explicitly stop sequence A (no new effects from A in this cue)
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![],
                stop_sequences: vec!["seqA".to_string()],
                start_sequences: vec![],
            },
            // @8s: new effect from sequence A — should be suppressed
            Cue {
                time: Duration::from_secs(8),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 10s — past the stop at 5s and the suppressed effect at 8s
        let update = timeline.start_at(Duration::from_secs(10));

        // Should have 1 effect: seqB from @0s (seqA was stopped at @5s, so
        // both the @0s seqA effect and the @8s seqA effect are excluded)
        let seq_names: Vec<_> = update
            .effects_with_elapsed
            .values()
            .filter_map(|(e, _)| e.id.split("_effect_").next())
            .collect();
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "Should have 1 effect (seqB only), got {}: {:?}",
            update.effects_with_elapsed.len(),
            seq_names
        );
        // The surviving effect should be from seqB
        let surviving = update.effects_with_elapsed.values().next().unwrap();
        assert!(
            surviving.0.id.starts_with("seq_seqB_"),
            "Surviving effect should be from seqB, got: {}",
            surviving.0.id
        );
    }

    #[test]
    fn test_start_at_iteration_boundary_does_not_suppress() {
        // Verify that iteration-boundary stops (stop + start in same cue)
        // do NOT suppress the sequence — only explicit stops do.
        use crate::lighting::parser::Cue;

        let seq_effect = |seq: &str| Effect {
            sequence_name: Some(seq.to_string()),
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start effects from seqA (iteration 1)
            Cue {
                time: Duration::from_secs(0),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec!["seqA".to_string()],
            },
            // @5s: iteration boundary — stop old seqA, start new seqA effects
            Cue {
                time: Duration::from_secs(5),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec!["seqA".to_string()],
                start_sequences: vec!["seqA".to_string()],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 7s — past the iteration boundary at 5s
        let update = timeline.start_at(Duration::from_secs(7));

        // Should have 1 effect: the seqA from @5s (iteration boundary didn't suppress)
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "Should have 1 effect (seqA iteration 2), got {}",
            update.effects_with_elapsed.len()
        );
        let surviving = update.effects_with_elapsed.values().next().unwrap();
        assert!(surviving.0.id.starts_with("seq_seqA_"));
        assert_eq!(surviving.1, Duration::from_secs(2)); // 7s - 5s = 2s elapsed
    }
}
