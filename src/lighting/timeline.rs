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
}

impl LightingTimeline {
    /// Creates a new lighting timeline from DSL cues
    pub fn new(cues: Vec<Cue>) -> Self {
        let mut timeline = Self {
            cues,
            current_time: Duration::ZERO,
            next_cue_index: 0,
            is_playing: false,
        };
        timeline.sort_cues();
        timeline
    }

    /// Creates a new lighting timeline from DSL light shows
    pub fn new_from_shows(shows: Vec<LightShow>) -> Self {
        let mut all_cues = Vec::new();
        for show in shows {
            all_cues.extend(show.cues);
        }
        Self::new(all_cues)
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
    fn create_effect_instance(effect: &Effect) -> Option<EffectInstance> {
        // Create base effect instance using the DSL EffectType directly
        let effect_instance = EffectInstance::new(
            format!(
                "song_effect_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            effect.effect_type.clone(),
            effect.groups.clone(),
        );

        // Apply builder methods based on parameters if needed
        // (DSL effects already have their parameters applied to the EffectType)
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
        let timeline = LightingTimeline::new(cues);
        assert_eq!(timeline.cues.len(), 0);
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_timeline_start_stop() {
        let cues = vec![];
        let mut timeline = LightingTimeline::new(cues);

        assert!(!timeline.is_playing);
        timeline.start();
        assert!(timeline.is_playing);
        timeline.stop();
        assert!(!timeline.is_playing);
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
        };

        let cues = vec![Cue {
            time: Duration::from_millis(0),
            effects: vec![effect],
        }];

        let mut timeline = LightingTimeline::new(cues);
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

        let mut timeline = LightingTimeline::new(cues);
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
        let timeline = LightingTimeline::new(vec![]);
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

        let mut timeline = LightingTimeline::new(cues);
        timeline.start();

        // Both cues should trigger at the same time
        let effects = timeline.update(Duration::from_millis(5000));
        assert_eq!(effects.len(), 2);
    }
}
