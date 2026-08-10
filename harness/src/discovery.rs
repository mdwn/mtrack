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
//! Finds out how this machine is actually cabled.
//!
//! The suite is meant to run against an unknown rig, so rather than being told
//! which outputs feed which inputs, it measures. Audio loopback is found by
//! playing a distinct tone per output channel and seeing where each one
//! arrives; MIDI loopback is found by sending an identifying SysEx on each
//! output port and seeing which inputs receive it.
//!
//! Discovery needs exclusive use of the devices, so it runs once before any
//! server starts and its result is cached for the rest of the process (and, on
//! disk, for subsequent runs).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crate::capabilities::{AudioInput, AudioOutput, Capabilities};
use crate::songs::TRACK_TONES;

/// One audio output channel arriving on one input channel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioLoopback {
    pub out_device: String,
    pub out_channel: u16,
    pub in_device: String,
    pub in_channel: u16,
}

/// One MIDI output port whose traffic arrives on an input port.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MidiLoopback {
    pub out_port: String,
    pub in_port: String,
}

/// Everything discovery learned about the cabling.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Discovery {
    pub audio: Vec<AudioLoopback>,
    pub midi: Vec<MidiLoopback>,
    /// The device inventory this result was measured against. If the hardware
    /// changes, a cached result is stale and is re-measured.
    fingerprint: String,
    /// Unix seconds at which this was measured.
    #[serde(default)]
    measured_at: u64,
    /// How this map was obtained, for the report. Not persisted.
    #[serde(skip)]
    source: String,
}

/// How long a cached cabling map stays trusted.
///
/// A stale map does not fail loudly -- it silently changes which areas report
/// as unverifiable, which is the quietest possible way for a rig change to
/// corrupt a run's conclusions.
const CACHE_MAX_AGE_SECS: u64 = 24 * 60 * 60;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Renders a duration in seconds as a short human-readable age.
fn describe_age(secs: u64) -> String {
    match secs {
        s if s < 90 => format!("{s}s ago"),
        s if s < 5400 => format!("{}m ago", s / 60),
        s => format!("{}h ago", s / 3600),
    }
}

/// Set when the caller must not make noise -- `--list` says "show me the plan",
/// not "play tones out of every output and send SysEx to every MIDI port",
/// which on a live rig is a genuinely unwelcome surprise.
static PROBING_FORBIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Forbids measuring. A cached map is still used if one is valid.
pub fn forbid_probing() {
    PROBING_FORBIDDEN.store(true, std::sync::atomic::Ordering::SeqCst);
}

impl Discovery {
    /// Discovers once per process, reusing a cached result when the hardware
    /// inventory is unchanged.
    pub fn get() -> &'static Discovery {
        static DISCOVERY: OnceLock<Discovery> = OnceLock::new();
        DISCOVERY.get_or_init(|| {
            let caps = Capabilities::get();
            let fingerprint = fingerprint(caps);

            if !force_rediscover() {
                if let Some(mut cached) = load_cache() {
                    let age = now_secs().saturating_sub(cached.measured_at);
                    if cached.fingerprint == fingerprint && age <= CACHE_MAX_AGE_SECS {
                        cached.source = format!("cached, measured {}", describe_age(age));
                        println!(
                            "  (using cabling measured {}; --rediscover to re-measure)",
                            describe_age(age)
                        );
                        return cached;
                    }
                    if age > CACHE_MAX_AGE_SECS
                        && !PROBING_FORBIDDEN.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        println!(
                            "  (cached cabling is {} old, re-measuring)",
                            describe_age(age)
                        );
                    }
                }
            }

            if PROBING_FORBIDDEN.load(std::sync::atomic::Ordering::SeqCst) {
                return Discovery {
                    audio: Vec::new(),
                    midi: Vec::new(),
                    fingerprint,
                    measured_at: 0,
                    source: "not measured -- probing suppressed for --list".to_string(),
                };
            }

            let mut discovered = measure(caps, fingerprint);
            discovered.measured_at = now_secs();
            save_cache(&discovered);
            discovered.source = "freshly measured".to_string();
            discovered
        })
    }

    /// Audio loopbacks for a given output device.
    pub fn audio_for_device(&self, out_device: &str) -> Vec<&AudioLoopback> {
        self.audio
            .iter()
            .filter(|l| l.out_device == out_device)
            .collect()
    }

    /// The input channel an output channel is patched to, on one device pair.
    pub fn input_channel_for(&self, out_device: &str, out_channel: u16) -> Option<(&str, u16)> {
        self.audio
            .iter()
            .find(|l| l.out_device == out_device && l.out_channel == out_channel)
            .map(|l| (l.in_device.as_str(), l.in_channel))
    }

    /// A MIDI port pair usable as a loopback, preferring one where both
    /// directions are the same port, and hardware over the OS through-port.
    ///
    /// Hardware first because the through-port loops back with no cable
    /// attached: on a rig that has both, it is the one port whose success says
    /// nothing about the interface the operator actually uses. Preferred
    /// against rather than excluded -- a machine with only the through-port
    /// still verifies mtrack's MIDI generation, and every result says which
    /// path it ran on.
    pub fn midi_pair(&self) -> Option<&MidiLoopback> {
        self.midi
            .iter()
            .find(|l| l.out_port == l.in_port && !is_through_port(&l.out_port))
            .or_else(|| self.midi.iter().find(|l| l.out_port == l.in_port))
            .or_else(|| self.midi.first())
    }

    /// Whether this map reflects an actual measurement. A suppressed probe
    /// yields an empty map, which must not be reported as "nothing is patched".
    pub fn was_measured(&self) -> bool {
        self.measured_at != 0
    }

    /// How this cabling map was obtained.
    pub fn source(&self) -> &str {
        if self.source.is_empty() {
            "unknown"
        } else {
            &self.source
        }
    }

    pub fn has_audio_loopback(&self) -> bool {
        !self.audio.is_empty()
    }

    pub fn has_midi_loopback(&self) -> bool {
        !self.midi.is_empty()
    }

    /// A human-readable summary of the cabling.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for l in &self.audio {
            lines.push(format!(
                "audio: {} ch{} -> {} ch{}",
                short(&l.out_device),
                l.out_channel,
                short(&l.in_device),
                l.in_channel
            ));
        }
        for l in &self.midi {
            lines.push(if l.out_port == l.in_port {
                format!("midi:  {} (loops to itself)", short(&l.out_port))
            } else {
                format!("midi:  {} -> {}", short(&l.out_port), short(&l.in_port))
            });
        }
        lines
    }
}

/// Trims a device name to something readable in a summary line.
fn short(name: &str) -> &str {
    name.split(" (").next().unwrap_or(name)
}

/// Whether the cache should be ignored.
fn force_rediscover() -> bool {
    std::env::var("MTRACK_E2E_REDISCOVER")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// Identifies the hardware inventory, so a cached result can be invalidated
/// when a device is added, removed, or renamed.
fn fingerprint(caps: &Capabilities) -> String {
    let mut parts: BTreeSet<String> = BTreeSet::new();
    for d in &caps.all_audio_out {
        parts.insert(format!("ao:{}:{}", d.name, d.max_channels));
    }
    for d in &caps.all_audio_in {
        parts.insert(format!("ai:{}:{}", d.name, d.max_channels));
    }
    for d in &caps.all_midi {
        parts.insert(format!("m:{}:{}{}", d.name, d.has_input, d.has_output));
    }
    // Selection matters as much as inventory: a run with a subsystem disabled
    // must not reuse cabling measured while it was enabled.
    // Probe breadth changes what the map can contain, so a --probe-all result
    // and a single-pair result are not interchangeable.
    parts.insert(format!("probe_all:{}", probe_all_devices()));
    // Amplitude decides whether tones clear the noise floor. One run at 0.02
    // discovers nothing and caches that emptiness for every later run.
    parts.insert(format!("amplitude:{}", crate::songs::amplitude()));
    parts.insert(format!(
        "sel:{}:{}:{}",
        caps.audio_out
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("-"),
        caps.audio_in
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("-"),
        caps.midi_out
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("-"),
    ));
    parts.into_iter().collect::<Vec<_>>().join("|")
}

fn cache_path() -> PathBuf {
    std::env::var("MTRACK_E2E_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // The workspace target dir, matching where the mtrack binary is
            // looked up. Under harness/target it would survive a workspace
            // `cargo clean`, which is precisely when a stale cabling map is
            // least expected.
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("target")
                .join("hardware-e2e-discovery.json")
        })
}

fn load_cache() -> Option<Discovery> {
    let body = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&body).ok()
}

fn save_cache(discovery: &Discovery) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string_pretty(discovery) {
        let _ = std::fs::write(path, body);
    }
}

/// Runs the actual measurements.
fn measure(caps: &Capabilities, fingerprint: String) -> Discovery {
    println!("\n  Probing for loopback cabling (this plays brief tones and sends MIDI)...");

    let audio = measure_audio(caps);
    let midi = measure_midi(caps);

    let discovery = Discovery {
        audio,
        midi,
        fingerprint,
        measured_at: now_secs(),
        source: "freshly measured".to_string(),
    };

    if discovery.audio.is_empty() && discovery.midi.is_empty() {
        println!("  No loopback found on any device pair.");
    } else {
        for line in discovery.describe() {
            println!("    {line}");
        }
    }
    println!();

    discovery
}

/// Probes every plausible output-device/input-device pair for audio loopback.
///
/// Each pair costs one pass, but a pass covers all of that device's channels at
/// once because every output carries a different tone.
fn measure_audio(caps: &Capabilities) -> Vec<AudioLoopback> {
    if caps.audio_out.is_none() || caps.audio_in.is_none() {
        return Vec::new();
    }
    let outputs = probe_candidates_out(caps);
    let inputs = probe_candidates_in(caps);
    if outputs.is_empty() || inputs.is_empty() {
        return Vec::new();
    }

    let mut found = Vec::new();
    for out in &outputs {
        for input in &inputs {
            match crate::capture::probe_pair(out, input, &TRACK_TONES) {
                Ok(pairs) => {
                    for (out_channel, in_channel) in pairs {
                        found.push(AudioLoopback {
                            out_device: out.name.clone(),
                            out_channel,
                            in_device: input.name.clone(),
                            in_channel,
                        });
                    }
                }
                Err(e) => {
                    println!(
                        "    (could not probe {} -> {}: {e})",
                        short(&out.name),
                        short(&input.name)
                    );
                }
            }
        }
    }
    found
}

/// Output devices worth probing: real hardware only, since ALSA plug nodes are
/// aliases of devices already in the list and would multiply the pass count
/// while measuring the same cable.
fn probe_candidates_out(caps: &Capabilities) -> Vec<AudioOutput> {
    // The selected device is always a candidate. Filtering the wider set to raw
    // hardware alone meant that under --probe-all a rig whose only openable
    // output is a plug device (a 24-bit console, which the harness explicitly
    // accepts as legitimate) produced no candidates at all -- so --probe-all
    // lost every loopback on exactly the rig it is most useful for.
    let mut candidates: Vec<AudioOutput> = caps.audio_out.iter().cloned().collect();
    if !probe_all_devices() {
        return candidates;
    }
    for extra in caps
        .all_audio_out
        .iter()
        .filter(|d| d.name.contains("hw:CARD=") && !d.name.contains("plughw:"))
    {
        if !candidates.iter().any(|c| c.name == extra.name) {
            candidates.push(extra.clone());
        }
    }
    candidates
}

/// Input devices worth probing, on the same reasoning.
fn probe_candidates_in(caps: &Capabilities) -> Vec<AudioInput> {
    // The selected device is always a candidate. Filtering the wider set to raw
    // hardware alone meant that under --probe-all a rig whose only openable
    // output is a plug device (a 24-bit console, which the harness explicitly
    // accepts as legitimate) produced no candidates at all -- so --probe-all
    // lost every loopback on exactly the rig it is most useful for.
    let mut candidates: Vec<AudioInput> = caps.audio_in.iter().cloned().collect();
    if !probe_all_devices() {
        return candidates;
    }
    for extra in caps
        .all_audio_in
        .iter()
        .filter(|d| d.name.contains("hw:CARD=") && !d.name.contains("plughw:"))
    {
        if !candidates.iter().any(|c| c.name == extra.name) {
            candidates.push(extra.clone());
        }
    }
    candidates
}

/// Whether to probe every hardware device pair rather than just the selected
/// one. Off by default: each extra pair adds several seconds of tones.
fn probe_all_devices() -> bool {
    std::env::var("MTRACK_E2E_PROBE_ALL")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// Manufacturer ID reserved for non-commercial use, so the probe's SysEx
/// cannot be mistaken for a real device's message.
const PROBE_SYSEX_ID: u8 = 0x7D;

/// Whether a MIDI port is the OS's software through-port rather than an
/// interface.
///
/// ALSA's is `Midi Through Port-0`; it loops back with no cable present. Matched
/// on the full `midi through` rather than bare `through`, which would also claim
/// any interface with the word in its name -- a "Passthrough 4x4" is real
/// hardware and must not be demoted to a software port.
pub fn is_through_port(name: &str) -> bool {
    name.to_ascii_lowercase().contains("midi through")
}

/// Probes every MIDI output port to see which input ports receive its traffic.
///
/// SysEx rather than a note: it carries an identifying payload, and no
/// synthesiser will make a sound in response, which matters when the port
/// under test is connected to real gear.
fn measure_midi(caps: &Capabilities) -> Vec<MidiLoopback> {
    use midir::{Ignore, MidiInput, MidiOutput};

    if caps.all_midi.is_empty() || caps.midi_out.is_none() {
        return Vec::new();
    }

    let Ok(output_lister) = MidiOutput::new("mtrack-e2e-probe-out") else {
        return Vec::new();
    };
    let out_ports = output_lister.ports();
    if out_ports.is_empty() {
        return Vec::new();
    }

    let mut found = Vec::new();
    for (index, out_port) in out_ports.iter().enumerate() {
        let Ok(sender) = MidiOutput::new("mtrack-e2e-probe-out") else {
            continue;
        };
        let Ok(out_name) = sender.port_name(out_port) else {
            continue;
        };

        // Every input is opened before the message is sent, so one pass per
        // output port covers all inputs.
        let Ok(mut input_lister) = MidiInput::new("mtrack-e2e-probe-in") else {
            continue;
        };
        input_lister.ignore(Ignore::None);
        let in_ports = input_lister.ports();

        let mut listeners = Vec::new();
        for in_port in &in_ports {
            let Ok(mut input) = MidiInput::new("mtrack-e2e-probe-in") else {
                continue;
            };
            input.ignore(Ignore::None);
            let Ok(in_name) = input.port_name(in_port) else {
                continue;
            };
            let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = seen.clone();
            let expected = index as u8 & 0x7F;
            if let Ok(conn) = input.connect(
                in_port,
                "mtrack-e2e-probe",
                move |_, bytes, _| {
                    if bytes.first() == Some(&0xF0)
                        && bytes.get(1) == Some(&PROBE_SYSEX_ID)
                        && bytes.get(2) == Some(&expected)
                    {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                },
                (),
            ) {
                listeners.push((in_name, seen, conn));
            }
        }

        if listeners.is_empty() {
            continue;
        }

        let Ok(mut connection) = sender.connect(out_port, "mtrack-e2e-probe") else {
            continue;
        };
        let message = [0xF0, PROBE_SYSEX_ID, index as u8 & 0x7F, 0xF7];
        let _ = connection.send(&message);
        // Give the message time to traverse the port before tearing down.
        std::thread::sleep(Duration::from_millis(250));
        connection.close();

        for (in_name, seen, conn) in listeners {
            if seen.load(std::sync::atomic::Ordering::SeqCst) {
                found.push(MidiLoopback {
                    out_port: out_name.clone(),
                    in_port: in_name,
                });
            }
            conn.close();
        }
    }

    found
}
