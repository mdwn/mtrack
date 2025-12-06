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

use std::time::Duration;

use super::super::effects::{BlendMode, EffectLayer, EffectType};

// Light show DSL data structures
#[derive(Debug, Clone)]
pub struct LightShow {
    pub name: String,
    pub cues: Vec<Cue>,
    pub tempo_map: Option<crate::lighting::tempo::TempoMap>,
}

/// A reusable sequence of cues that can be referenced in shows
#[derive(Debug, Clone)]
pub struct Sequence {
    pub cues: Vec<Cue>,
}

impl Sequence {
    /// Calculate the duration of this sequence based on when all effects complete
    /// Returns None if the sequence contains perpetual effects (never finishes)
    /// Otherwise returns the time from sequence start (0) to when the last effect completes
    pub fn duration(&self) -> Option<Duration> {
        if self.cues.is_empty() {
            return Some(Duration::ZERO);
        }

        let mut max_completion_time = Duration::ZERO;
        let mut has_any_duration = false;

        for cue in &self.cues {
            for effect in &cue.effects {
                if let Some(effect_duration) = effect.total_duration() {
                    // Effect completes at: cue_time + effect_duration
                    let completion_time = cue.time + effect_duration;
                    if completion_time > max_completion_time {
                        max_completion_time = completion_time;
                    }
                    has_any_duration = true;
                }
                // Perpetual effects are ignored for duration calculation
            }
        }

        if has_any_duration {
            Some(max_completion_time)
        } else {
            // All effects are perpetual - sequence never finishes
            None
        }
    }
}

/// Loop mode for sequence references
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SequenceLoop {
    Once,
    Loop, // Infinite loop
    PingPong,
    Random,
    Count(usize), // Loop N times
}

/// Unexpanded sequence cue information for two-pass parsing
#[derive(Debug, Clone)]
pub(crate) struct UnexpandedSequenceCue {
    pub time: Duration,
    pub effects: Vec<Effect>,
    pub layer_commands: Vec<LayerCommand>,
    pub stop_sequences: Vec<String>,
    pub sequence_references: Vec<(String, Option<SequenceLoop>)>, // (sequence_name, loop_param)
}

#[derive(Debug, Clone)]
pub struct Cue {
    pub time: Duration,
    pub effects: Vec<Effect>,
    pub layer_commands: Vec<LayerCommand>,
    pub stop_sequences: Vec<String>, // Names of sequences to stop at this cue time
}

#[derive(Debug, Clone)]
pub struct Effect {
    pub groups: Vec<String>,
    pub effect_type: EffectType,
    pub layer: Option<EffectLayer>,
    pub blend_mode: Option<BlendMode>,
    pub up_time: Option<Duration>,
    pub hold_time: Option<Duration>,
    pub down_time: Option<Duration>,
    pub sequence_name: Option<String>, // Track which sequence this effect came from (for stopping)
}

impl Effect {
    /// Calculate the total duration of this effect
    /// Returns None for perpetual/indefinite effects
    pub fn total_duration(&self) -> Option<Duration> {
        // Check if this is an indefinite effect (no hold_time and no down_time)
        let hold = self.hold_time.unwrap_or(Duration::from_secs(0));
        let down = self.down_time.unwrap_or(Duration::from_secs(0));
        let is_indefinite = hold.is_zero() && down.is_zero();

        // Effects are perpetual if they are indefinite AND have no explicit duration
        if is_indefinite {
            match &self.effect_type {
                // Static effects with no duration are perpetual
                EffectType::Static { duration: None, .. } => return None,
                // ColorCycle, Chase, Rainbow have no duration field - perpetual by design
                EffectType::ColorCycle { .. } => return None,
                EffectType::Chase { .. } => return None,
                EffectType::Rainbow { .. } => return None,
                // Strobe and Pulse with no duration are perpetual
                EffectType::Strobe { duration: None, .. } => return None,
                EffectType::Pulse { duration: None, .. } => return None,
                _ => {} // Fall through to calculate duration
            }
        }

        // For dimmers, use duration field (timing params not used)
        if let EffectType::Dimmer { duration, .. } = &self.effect_type {
            return Some(*duration);
        }

        let duration = self.up_time.unwrap_or(Duration::from_secs(0))
            + self.hold_time.unwrap_or(Duration::from_secs(0))
            + self.down_time.unwrap_or(Duration::from_secs(0));

        Some(duration)
    }
}

/// Layer control command types (grandMA-inspired)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerCommandType {
    /// Clear - immediately stop all effects on the layer
    Clear,
    /// Release - gracefully fade out all effects on the layer
    Release,
    /// Freeze - pause all effects on the layer at their current state
    Freeze,
    /// Unfreeze - resume paused effects on the layer
    Unfreeze,
    /// Master - set layer intensity and/or speed master
    Master,
}

/// A layer control command
#[derive(Debug, Clone)]
pub struct LayerCommand {
    pub command_type: LayerCommandType,
    pub layer: Option<EffectLayer>, // None means all layers (only valid for clear)
    pub fade_time: Option<Duration>,
    pub intensity: Option<f64>,
    pub speed: Option<f64>,
}
