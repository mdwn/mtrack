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

/// Tempo-aware speed specification that can adapt to tempo changes
#[derive(Debug, Clone, PartialEq)]
pub enum TempoAwareSpeed {
    /// Fixed speed in cycles per second (not tempo-aware)
    Fixed(f64),
    /// Speed specified in measures (tempo-aware)
    Measures(f64),
    /// Speed specified in beats (tempo-aware)
    Beats(f64),
    /// Speed specified in seconds (fixed, not tempo-aware)
    Seconds(f64),
}

impl TempoAwareSpeed {
    /// Get the current speed in cycles per second, using tempo map if available
    pub fn to_cycles_per_second(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        match self {
            TempoAwareSpeed::Fixed(speed) => *speed,
            TempoAwareSpeed::Seconds(duration) => {
                if *duration <= 0.0 {
                    0.0 // Zero/negative duration means stopped
                } else {
                    1.0 / duration
                }
            }
            TempoAwareSpeed::Measures(measures) => {
                if *measures <= 0.0 {
                    return 0.0; // Zero/negative measures means stopped
                }
                if let Some(tm) = tempo_map {
                    let duration = tm.measures_to_duration(*measures, at_time, 0.0);
                    let secs = duration.as_secs_f64();
                    if secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / secs
                    }
                } else {
                    // Fallback: assume 120 BPM, 4/4 time
                    let beats = measures * 4.0;
                    let duration_secs = beats * 60.0 / 120.0;
                    if duration_secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / duration_secs
                    }
                }
            }
            TempoAwareSpeed::Beats(beats) => {
                if *beats <= 0.0 {
                    return 0.0; // Zero/negative beats means stopped
                }
                if let Some(tm) = tempo_map {
                    let duration = tm.beats_to_duration(*beats, at_time, 0.0);
                    let secs = duration.as_secs_f64();
                    if secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / secs
                    }
                } else {
                    // Fallback: assume 120 BPM
                    let duration_secs = beats * 60.0 / 120.0;
                    if duration_secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / duration_secs
                    }
                }
            }
        }
    }
}

/// Tempo-aware frequency specification that can adapt to tempo changes
#[derive(Debug, Clone, PartialEq)]
pub enum TempoAwareFrequency {
    /// Fixed frequency in Hz (not tempo-aware)
    Fixed(f64),
    /// Frequency specified in measures (tempo-aware)
    Measures(f64),
    /// Frequency specified in beats (tempo-aware)
    Beats(f64),
    /// Frequency specified in seconds (fixed, not tempo-aware)
    Seconds(f64),
}

impl TempoAwareFrequency {
    /// Get the current frequency in Hz, using tempo map if available
    pub fn to_hz(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        match self {
            TempoAwareFrequency::Fixed(freq) => *freq,
            TempoAwareFrequency::Seconds(duration) => {
                if *duration <= 0.0 {
                    0.0 // Zero/negative duration means no frequency (stopped)
                } else {
                    1.0 / duration
                }
            }
            TempoAwareFrequency::Measures(measures) => {
                if *measures <= 0.0 {
                    return 0.0; // Zero/negative measures means stopped
                }
                if let Some(tm) = tempo_map {
                    let duration = tm.measures_to_duration(*measures, at_time, 0.0);
                    let secs = duration.as_secs_f64();
                    if secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / secs
                    }
                } else {
                    // Fallback: assume 120 BPM, 4/4 time
                    let beats = measures * 4.0;
                    let duration_secs = beats * 60.0 / 120.0;
                    if duration_secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / duration_secs
                    }
                }
            }
            TempoAwareFrequency::Beats(beats) => {
                if *beats <= 0.0 {
                    return 0.0; // Zero/negative beats means stopped
                }
                if let Some(tm) = tempo_map {
                    let duration = tm.beats_to_duration(*beats, at_time, 0.0);
                    let secs = duration.as_secs_f64();
                    if secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / secs
                    }
                } else {
                    // Fallback: assume 120 BPM
                    let duration_secs = beats * 60.0 / 120.0;
                    if duration_secs <= 0.0 {
                        0.0
                    } else {
                        1.0 / duration_secs
                    }
                }
            }
        }
    }
}
