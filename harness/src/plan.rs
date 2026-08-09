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
//! What this run will and will not exercise, decided before anything runs.
//!
//! The suite is expected to run against whatever hardware happens to be
//! present: audio only, MIDI only, or some combination. Every area declares
//! what it needs, so a machine missing a subsystem runs the applicable subset
//! and reports the rest as skipped rather than failing or, worse, passing
//! quietly.

use crate::capabilities::Capabilities;
use crate::discovery::Discovery;

/// A capability an area depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// An audio output device that can be opened.
    AudioOut,
    /// An output channel patched back to an input, so what was played can be
    /// heard back.
    AudioLoopback,
    /// A MIDI output port.
    MidiOut,
    /// A MIDI port pair, so what was transmitted can be read back.
    MidiLoopback,
    /// An OLA daemon, so DMX frames reach the wire.
    OlaDaemon,
}

impl Need {
    /// Why this need is unmet, or `None` if it is satisfied.
    pub fn unmet(&self) -> Option<String> {
        let caps = Capabilities::get();
        match self {
            Need::AudioOut => caps
                .audio_out
                .is_none()
                .then(|| "no audio output device".to_string()),
            Need::AudioLoopback => {
                if caps.audio_out.is_none() {
                    Some("no audio output device to play through".to_string())
                } else if caps.audio_in.is_none() {
                    Some("no audio input device to capture with".to_string())
                } else if !Discovery::get().has_audio_loopback() {
                    Some("no output channel is patched back to an input".to_string())
                } else {
                    None
                }
            }
            Need::MidiOut => caps
                .midi_out
                .is_none()
                .then(|| "no MIDI output port".to_string()),
            // A loopback presupposes a port to transmit on, so the simpler
            // failure is reported first rather than as "nothing loops back".
            Need::MidiLoopback => Need::MidiOut.unmet().or_else(|| {
                (!Discovery::get().has_midi_loopback()).then(|| {
                    "no MIDI output port loops back to an input (try `sudo modprobe snd-virmidi`, \
                     or ALSA's Midi Through)"
                        .to_string()
                })
            }),
            Need::OlaDaemon => caps
                .ola_port
                .is_none()
                .then(|| "no OLA daemon is listening".to_string()),
        }
    }
}

/// A group of cases sharing the same requirements.
pub struct Area {
    pub name: &'static str,
    pub description: &'static str,
    pub needs: &'static [Need],
}

/// Every area the suite knows about, in the order they are reported.
pub const AREAS: &[Area] = &[
    Area {
        name: "startup",
        description: "config synthesis, device claim, song loading",
        needs: &[],
    },
    Area {
        name: "devices",
        description: "device lists agree and are openable",
        needs: &[],
    },
    Area {
        name: "playback",
        description: "transport, clock, playlist navigation",
        needs: &[Need::AudioOut],
    },
    Area {
        name: "audio-routing",
        description: "tracks reach their mapped physical channels",
        needs: &[Need::AudioOut, Need::AudioLoopback],
    },
    Area {
        name: "midi-transmit",
        description: "notes and beat clock on the wire",
        needs: &[Need::MidiLoopback],
    },
    Area {
        name: "midi-config",
        description: "MIDI settings persist correctly",
        needs: &[Need::MidiOut],
    },
    Area {
        name: "subsystems",
        description: "subsystem presence, absence, and misconfiguration",
        needs: &[Need::AudioOut],
    },
    Area {
        name: "persistence",
        description: "config round-trips to disk and back",
        needs: &[],
    },
    Area {
        name: "lighting",
        description: "show creation, validation, cues, live effects",
        needs: &[Need::AudioOut],
    },
    Area {
        name: "dmx-output",
        description: "DMX frames on the wire",
        needs: &[Need::OlaDaemon],
    },
];

/// The first unmet need of an area, if any.
pub fn blocked_reason(area: &Area) -> Option<String> {
    area.needs.iter().find_map(|need| need.unmet())
}

/// Why the named area cannot run, or `None` if it can.
pub fn blocked_reason_for(name: &str) -> Option<String> {
    AREAS
        .iter()
        .find(|a| a.name == name)
        .and_then(blocked_reason)
}

/// Prints the plan before anything runs.
///
/// This is the answer to "what is this run actually going to verify", and it
/// is printed up front so an unattended or interrupted run still says what it
/// intended to cover.
pub fn print_plan() {
    let discovery = Discovery::get();
    let cabling = discovery.describe();
    println!("=== mtrack hardware e2e: discovered cabling ===");
    if cabling.is_empty() {
        println!("  none -- nothing is patched back, so nothing can be verified by capture");
    } else {
        for line in cabling {
            println!("  {line}");
        }
    }
    println!();

    println!("=== mtrack hardware e2e: run plan ===");
    let mut running = 0;
    let mut skipped = 0;
    for area in AREAS {
        match blocked_reason(area) {
            None => {
                running += 1;
                println!("  RUN   {:<14} {}", area.name, area.description);
            }
            Some(reason) => {
                skipped += 1;
                println!("  SKIP  {:<14} {}", area.name, reason);
            }
        }
    }
    println!("\n  {running} area(s) will run, {skipped} will not.");
    println!("=====================================\n");
}

/// Whether anything at all can be exercised.
///
/// Used to distinguish "this machine has no hardware worth testing" from "the
/// suite passed", which would otherwise look identical.
pub fn anything_runs() -> bool {
    let caps = Capabilities::get();
    caps.audio_out.is_some() || caps.midi_out.is_some()
}
