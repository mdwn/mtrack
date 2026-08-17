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

// Note: there is deliberately no `dmx-output` area. Verifying DMX on the wire
// needs a reader on the far side of OLA, which does not exist yet; the lighting
// checks record a caveat when olad is absent instead. An area with no checks
// behind it would advertise RUN in the plan and then never appear in the
// results -- exactly the overstated coverage this tool exists to avoid.

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
                } else if !Discovery::get().was_measured() {
                    Some("cabling has not been measured, so this is unknown".to_string())
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
                if !Discovery::get().was_measured() {
                    return Some("cabling has not been measured, so this is unknown".to_string());
                }
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
    /// Runs only when explicitly selected with `--only`.
    ///
    /// For areas whose cost is out of proportion to how often they change —
    /// the trigger reload checks each start several players and sweep timing
    /// delays, adding about a minute to a run that otherwise takes seconds per
    /// check. They still print as not-run rather than vanishing, because a
    /// suite that silently covers less than it appears to is the thing this
    /// harness exists to avoid.
    pub opt_in: bool,
}

/// Every area the suite knows about, in the order they are reported.
pub const AREAS: &[Area] = &[
    Area {
        // Its checks claim an audio device, so a MIDI-only machine cannot run
        // them. Declaring no needs made the plan print RUN and then skip
        // everything -- the overstatement this tool exists to avoid.
        name: "startup",
        description: "config synthesis, device claim, song loading",
        needs: &[Need::AudioOut],
        opt_in: false,
    },
    Area {
        // Each check starts several players and sweeps timing delays, so this
        // costs about a minute against seconds for everything else. It covers
        // config-reload paths that change rarely, so it is opt-in.
        name: "triggers",
        description: "disabling triggers takes effect, and a cancelled reload strands nothing",
        needs: &[Need::AudioOut],
        opt_in: true,
    },
    Area {
        name: "devices",
        description: "device lists agree and are openable",
        needs: &[],
        opt_in: false,
    },
    Area {
        name: "playback",
        description: "transport, clock, playlist navigation",
        needs: &[Need::AudioOut],
        opt_in: false,
    },
    Area {
        name: "audio-routing",
        description: "tracks reach their mapped physical channels",
        needs: &[Need::AudioOut, Need::AudioLoopback],
        opt_in: false,
    },
    Area {
        name: "midi-transmit",
        description: "notes and beat clock on the wire",
        needs: &[Need::MidiLoopback],
        opt_in: false,
    },
    Area {
        name: "midi-config",
        description: "MIDI settings persist correctly",
        needs: &[Need::MidiOut],
        opt_in: false,
    },
    Area {
        name: "subsystems",
        description: "subsystem presence, absence, and misconfiguration",
        needs: &[Need::AudioOut],
        opt_in: false,
    },
    Area {
        name: "persistence",
        description: "config round-trips to disk and back",
        needs: &[],
        opt_in: false,
    },
    Area {
        name: "lighting",
        description: "show creation, validation, cues, live effects",
        needs: &[Need::AudioOut],
        opt_in: false,
    },
];

/// The first unmet need of an area, if any.
pub fn blocked_reason(area: &Area) -> Option<String> {
    // Unmet hardware first: "you have no input device" is more useful than
    // "you did not ask for it" when both are true.
    if let Some(unmet) = area.needs.iter().find_map(|need| need.unmet()) {
        return Some(unmet);
    }
    if area.opt_in && !was_selected(area) {
        return Some(format!(
            "opt-in: slow enough to skew a full run, so it runs only when asked for \
             (--only {})",
            area.name
        ));
    }
    None
}

/// Why the named area cannot run, or `None` if it can.
pub fn blocked_reason_for(name: &str) -> Option<String> {
    AREAS
        .iter()
        .find(|a| a.name == name)
        .and_then(blocked_reason)
}

/// The `--only` filter, so opt-in areas can tell whether they were asked for.
static SELECTION: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Records the run's filter. Called once, before anything runs.
pub fn set_selection(filter: &Option<String>) {
    let _ = SELECTION.set(filter.clone());
}

/// Set when the run is a self-test, which must cover opt-in areas too.
static INCLUDE_OPT_IN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Includes opt-in areas regardless of the filter.
///
/// `--self-test` exists to prove every check can fail. Letting opt-in areas sit
/// it out would put the hole exactly where the slow, rarely-run checks are —
/// the ones least likely to be noticed going vacuous.
pub fn include_opt_in_areas() {
    INCLUDE_OPT_IN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether an opt-in area was named by the filter.
fn was_selected(area: &Area) -> bool {
    if INCLUDE_OPT_IN.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    match SELECTION.get().and_then(|f| f.as_deref()) {
        // A filter naming the area, or one of its checks, counts as asking.
        Some(needle) => {
            area.name.contains(needle)
                || crate::checks::all()
                    .iter()
                    .any(|c| c.area == area.name && c.name.contains(needle))
        }
        None => false,
    }
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
