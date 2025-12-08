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

use std::time::{Duration, Instant};

use super::types::{BlendMode, EffectLayer, EffectType};

/// An instance of an effect with timing and targeting information
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub id: String,
    pub effect_type: EffectType,
    pub target_fixtures: Vec<String>, // Fixture names or group names
    pub priority: u8,                 // Higher priority overrides lower
    pub layer: EffectLayer,           // Layer for layering system
    pub blend_mode: BlendMode,        // How to blend with other effects
    pub start_time: Option<Instant>,  // Real-time instant when effect started
    pub cue_time: Option<Duration>, // Song time when effect was supposed to start (for deterministic randomness)
    pub up_time: Option<Duration>,  // Fade in duration (0% to 100%)
    pub hold_time: Option<Duration>, // Time at full intensity (100%)
    pub down_time: Option<Duration>, // Fade out duration (100% to 0%)
    pub enabled: bool,
}

impl EffectInstance {
    /// Determine if this effect is permanent (state-changing) or temporary (show effect)
    pub fn is_permanent(&self) -> bool {
        match &self.effect_type {
            EffectType::Static { duration, .. } => {
                // Static effects are permanent only if they have no duration AND no timing parameters
                duration.is_none()
                    && self.up_time.is_none()
                    && self.hold_time.is_none()
                    && self.down_time.is_none()
            }
            // Dimmer effects are always permanent so their resulting brightness persists
            EffectType::Dimmer { .. } => true,
            EffectType::ColorCycle { .. } => false, // Cycles complete and end
            EffectType::Strobe { .. } => false,     // Strobe completes and end
            EffectType::Chase { .. } => false,      // Chases complete and end
            EffectType::Rainbow { .. } => false,    // Rainbow cycles complete and end
            EffectType::Pulse { .. } => false,      // Pulse cycles complete and end
        }
    }

    pub fn new(
        id: String,
        effect_type: EffectType,
        target_fixtures: Vec<String>,
        up_time: Option<Duration>,
        hold_time: Option<Duration>,
        down_time: Option<Duration>,
    ) -> Self {
        // Extract duration from effect_type if available
        // Effects without an explicit duration are perpetual until replaced
        let duration = match &effect_type {
            EffectType::Static { duration, .. } => *duration,
            EffectType::Dimmer { duration, .. } => Some(*duration), // Dimmer duration becomes up_time
            EffectType::ColorCycle { .. } => None,                  // Perpetual until replaced
            EffectType::Strobe { duration, .. } => *duration,
            EffectType::Chase { .. } => None, // Perpetual until replaced
            EffectType::Rainbow { .. } => None, // Perpetual until replaced
            EffectType::Pulse { duration, .. } => *duration,
        };

        // Determine timing based on effect type, but allow override from parameters
        let (default_up_time, default_hold_time, default_down_time) = match &effect_type {
            EffectType::Dimmer { .. } => (None, None, None), // Dimmer uses its duration field
            EffectType::Static { duration: None, .. } => {
                // If timing parameters are provided, treat as timed effect
                if up_time.is_some() || hold_time.is_some() || down_time.is_some() {
                    // Use provided timing parameters for timed static effect
                    (up_time, hold_time, down_time)
                } else {
                    (None, None, None) // Truly indefinite static effect
                }
            }
            EffectType::Static {
                duration: Some(_), ..
            } => (None, duration, None), // Static effects with duration just hold for that duration
            _ => (None, duration, None), // Effects without duration are perpetual until replaced
        };

        // Use provided timing or fall back to defaults
        let final_up_time = up_time.or(default_up_time);
        let final_hold_time = hold_time.or(default_hold_time);
        let final_down_time = down_time.or(default_down_time);

        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            layer: EffectLayer::Background,
            blend_mode: BlendMode::Replace,
            start_time: None,
            cue_time: None,
            up_time: final_up_time,
            hold_time: final_hold_time,
            down_time: final_down_time,
            enabled: true,
        }
    }

    #[cfg(test)]
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    #[cfg(test)]
    pub fn with_timing(mut self, start_time: Option<Instant>, hold_time: Option<Duration>) -> Self {
        self.start_time = start_time;
        self.hold_time = hold_time;
        self
    }

    /// Calculate the crossfade multiplier for this effect at the given elapsed time
    pub fn calculate_crossfade_multiplier(&self, elapsed: Duration) -> f64 {
        // elapsed is the time since the effect started

        let up_time = self.up_time.unwrap_or(Duration::from_secs(0));
        let hold_time = self.hold_time.unwrap_or(Duration::from_secs(0));
        let down_time = self.down_time.unwrap_or(Duration::from_secs(0));

        let up_end = up_time;
        let hold_end = up_time + hold_time;
        let total_end = up_time + hold_time + down_time;

        // Small epsilon to make boundary checks inclusive and avoid flapping
        let eps = Duration::from_micros(1);

        // Check if this is an indefinite effect (no hold_time and no down_time)
        let is_indefinite = hold_time.is_zero() && down_time.is_zero();

        if up_time.is_zero() {
            // No fade in phase - go directly to hold or fade out
            if is_indefinite {
                // Indefinite effect (like static effects) - always at full intensity
                1.0
            } else if elapsed <= hold_end + eps {
                // Hold phase (100%)
                1.0
            } else if elapsed < total_end + eps {
                // Fade out phase (100% to 0%)
                if down_time.is_zero() {
                    0.0
                } else {
                    let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                    let t = if down_time.is_zero() {
                        1.0
                    } else {
                        (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0)
                    };
                    1.0 - t
                }
            } else {
                // Effect has ended
                0.0
            }
        } else if elapsed < up_end + eps {
            // Fade in phase (0% to 100%)
            (elapsed.as_secs_f64() / up_time.as_secs_f64()).clamp(0.0, 1.0)
        } else if is_indefinite {
            // Indefinite effect after fade-in - always at full intensity
            1.0
        } else if elapsed <= hold_end + eps {
            // Hold phase (100%)
            1.0
        } else if elapsed < total_end + eps {
            // Fade out phase (100% to 0%)
            if down_time.is_zero() {
                0.0
            } else {
                let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                let t = (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0);
                1.0 - t
            }
        } else {
            // Effect has ended
            0.0
        }
    }

    /// Get the total duration of this effect (up_time + hold_time + down_time)
    /// Returns None for indefinite/perpetual effects (effects without explicit duration or timing)
    pub fn total_duration(&self) -> Option<Duration> {
        // Check if this is an indefinite effect (no hold_time and no down_time)
        // This matches the semantics in calculate_crossfade_multiplier()
        // An effect with only up_time (fade-in) but no hold/down time runs indefinitely
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

    /// Determine if the effect has reached its intended terminal state for the given elapsed time
    /// This prefers value-based completion when applicable (e.g., dimmer hitting end level).
    pub fn has_reached_terminal_state(&self, elapsed: Duration) -> bool {
        let eps = Duration::from_micros(1);
        let value_eps = 0.0; // require exact target value for termination
        match &self.effect_type {
            EffectType::Dimmer {
                duration,
                start_level,
                end_level,
                ..
            } => {
                // Dimmer effect completes when end_level is reached
                if duration.is_zero() {
                    return true; // Instant transition
                }

                // Terminal when we've reached end_level
                let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
                let value = start_level + (end_level - start_level) * progress;
                (value - *end_level).abs() <= value_eps
            }
            EffectType::Static { .. } => {
                // Use total_duration() to include hold_time, up_time, and down_time
                // This ensures static effects with hold_time expire correctly
                self.total_duration()
                    .map(|d| elapsed + eps >= d)
                    .unwrap_or(false)
            }
            EffectType::Strobe { duration, .. } => {
                duration.map(|d| elapsed + eps >= d).unwrap_or(false)
            }
            EffectType::Pulse { duration, .. } => {
                duration.map(|d| elapsed + eps >= d).unwrap_or(false)
            }
            // Cycle-like effects terminate at configured duration
            EffectType::ColorCycle { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
            EffectType::Chase { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
            EffectType::Rainbow { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
        }
    }
}
