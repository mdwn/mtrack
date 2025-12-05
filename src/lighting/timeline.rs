// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
    parser::{Cue, Effect, LayerCommand, LightShow},
    EffectInstance,
};
use std::time::Duration;

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
    }

    /// Starts the timeline at a specific time (for seeking)
    /// This processes all cues before start_time to ensure deterministic state
    pub fn start_at(&mut self, start_time: Duration) -> TimelineUpdate {
        self.is_playing = true;
        self.current_time = start_time;
        self.next_cue_index = self.find_cue_index_at(start_time);

        // Process all cues before start_time to ensure deterministic state
        // This applies layer commands and starts effects that should still be active
        let mut result = TimelineUpdate::default();

        for i in 0..self.next_cue_index {
            let cue = &self.cues[i];

            // Apply all layer commands from historical cues
            result.layer_commands.extend(cue.layer_commands.clone());

            // Process stop sequence commands
            result.stop_sequences.extend(cue.stop_sequences.clone());

            // For effects, only include ones that would still be active at start_time
            for effect in &cue.effects {
                if let Some(effect_instance) = Self::create_effect_instance(effect) {
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
        }

        result
    }

    /// Finds the index of the first cue that should trigger at or after the given time
    fn find_cue_index_at(&self, time: Duration) -> usize {
        // Binary search to find the right cue index
        // We want the first cue that is >= time
        match self.cues.binary_search_by_key(&time, |cue| cue.time) {
            Ok(index) => {
                // Exact match - this cue should trigger
                index
            }
            Err(index) => {
                // No exact match - index is where we would insert
                // This is the first cue >= time
                index
            }
        }
    }

    /// Stops the timeline
    pub fn stop(&mut self) {
        self.is_playing = false;
        self.current_time = Duration::ZERO;
        self.next_cue_index = 0;
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
                // This cue should trigger - process effects
                for effect in &cue.effects {
                    if let Some(effect_instance) = Self::create_effect_instance(effect) {
                        result.effects.push(effect_instance);
                    }
                }
                // Process layer commands
                result.layer_commands.extend(cue.layer_commands.clone());
                // Process stop sequence commands
                result.stop_sequences.extend(cue.stop_sequences.clone());

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

    /// Creates an EffectInstance from a DSL Effect
    pub fn create_effect_instance(effect: &Effect) -> Option<EffectInstance> {
        // Create base effect instance using the DSL EffectType directly with timing
        // Include sequence name in ID if this effect came from a sequence
        let effect_id = if let Some(ref seq_name) = effect.sequence_name {
            format!(
                "seq_{}_effect_{}",
                seq_name,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        } else {
            format!(
                "song_effect_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            )
        };

        let mut effect_instance = EffectInstance::new(
            effect_id,
            effect.effect_type.clone(),
            effect.groups.clone(),
            effect.up_time,
            effect.hold_time,
            effect.down_time,
        );

        // Apply layering information if specified in DSL
        if let Some(layer) = effect.layer {
            effect_instance.layer = layer;
        }
        if let Some(blend_mode) = effect.blend_mode {
            effect_instance.blend_mode = blend_mode;
        }

        Some(effect_instance)
    }
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
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(0),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
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
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
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
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![effect2],
                layer_commands: vec![],
                stop_sequences: vec![],
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
                    layer: EffectLayer::Foreground,
                    fade_time: None,
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(1),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Release,
                    layer: EffectLayer::Background,
                    fade_time: Some(Duration::from_secs(2)),
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Master,
                    layer: EffectLayer::Midground,
                    fade_time: None,
                    intensity: Some(0.5),
                    speed: Some(2.0),
                }],
                stop_sequences: vec![],
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
        assert_eq!(result0.layer_commands[0].layer, EffectLayer::Foreground);

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
            time: Duration::from_secs(0),
            effects: vec![effect],
            layer_commands: vec![LayerCommand {
                command_type: LayerCommandType::Master,
                layer: EffectLayer::Background,
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
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
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
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
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
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
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
}
