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
    parser::{Cue, Effect, LightShow},
    EffectInstance,
};
use std::time::Duration;

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
    pub fn update(&mut self, song_time: Duration) -> Vec<EffectInstance> {
        if !self.is_playing {
            return Vec::new();
        }
        self.current_time = song_time;
        let mut triggered_effects = Vec::new();

        // Process all cues that should trigger at or before the current time
        while self.next_cue_index < self.cues.len() {
            let cue = &self.cues[self.next_cue_index];

            if cue.time <= song_time {
                // This cue should trigger
                for effect in &cue.effects {
                    if let Some(effect_instance) = Self::create_effect_instance(effect) {
                        triggered_effects.push(effect_instance);
                    }
                }
                self.next_cue_index += 1;
            } else {
                // No more cues to process at this time
                break;
            }
        }

        triggered_effects
    }

    /// Creates an EffectInstance from a DSL Effect
    pub fn create_effect_instance(effect: &Effect) -> Option<EffectInstance> {
        // Create base effect instance using the DSL EffectType directly with timing
        let mut effect_instance = EffectInstance::new(
            format!(
                "song_effect_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
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
            time: Duration::from_millis(0),
            effects: vec![effect],
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
            time: Duration::from_millis(0),
            effects: vec![effect],
        }];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Test that the first cue triggers at the right time
        let effects = timeline.update(Duration::from_millis(0));
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].target_fixtures, vec!["front_wash"]);
    }

    #[test]
    fn test_timeline_cue_ordering() {
        use crate::lighting::parser::Cue;

        // Create cues in non-chronological order
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());

        let effect = Effect {
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
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
            },
            Cue {
                time: Duration::from_millis(0),
                effects: vec![effect],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Verify cues are processed in chronological order
        let effects = timeline.update(Duration::from_millis(0));
        assert_eq!(effects.len(), 1);

        let effects = timeline.update(Duration::from_millis(5000));
        assert_eq!(effects.len(), 1);

        let effects = timeline.update(Duration::from_millis(10000));
        assert_eq!(effects.len(), 1);
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
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Both cues should trigger at the same time
        let effects = timeline.update(Duration::from_millis(5000));
        assert_eq!(effects.len(), 2);
    }

    #[test]
    fn test_timeline_stop_reset() {
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "red".to_string());

        let effect1 = Effect {
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
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![effect2],
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
        let effects_at_0s_restart = timeline.update(Duration::from_secs(0));
        assert_eq!(effects_at_0s_restart.len(), 1);

        // Should also trigger the second cue again
        let effects_at_2s_restart = timeline.update(Duration::from_secs(2));
        assert_eq!(effects_at_2s_restart.len(), 1);
    }
}
