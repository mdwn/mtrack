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
//! Probes the machine for real hardware and decides what the suite can
//! exercise.
//!
//! The suite assumes everything is present and disables what is not, so this
//! module is the single place that answers "can we?". Anything that comes back
//! unavailable is recorded as a [`Skip`] and printed in the end-of-run report
//! rather than silently passing.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};

/// An audio device with output channels, as mtrack itself sees it.
#[derive(Debug, Clone)]
pub struct AudioOutput {
    pub name: String,
    pub max_channels: u16,
    pub host: String,
    pub sample_rates: Vec<u32>,
}

/// An audio device with input channels. mtrack's own device listing only
/// enumerates outputs, so these are gathered from cpal directly.
#[derive(Debug, Clone)]
pub struct AudioInput {
    pub name: String,
    pub max_channels: u16,
    pub host: String,
}

/// A MIDI device and the directions it supports.
#[derive(Debug, Clone)]
pub struct MidiPort {
    pub name: String,
    pub has_input: bool,
    pub has_output: bool,
}

/// Something the suite wanted to exercise but could not.
#[derive(Debug, Clone)]
pub struct Skip {
    pub area: &'static str,
    pub reason: String,
}

/// Everything the suite learned about this machine.
#[derive(Debug, Clone)]
pub struct Capabilities {
    /// The output device the suite will drive.
    pub audio_out: Option<AudioOutput>,
    /// The input device the suite will capture from, if any.
    pub audio_in: Option<AudioInput>,
    /// The MIDI device the suite will send on.
    pub midi_out: Option<MidiPort>,
    /// The MIDI device the suite will listen on. May be the same physical
    /// device as `midi_out` when it is bidirectional.
    pub midi_in: Option<MidiPort>,
    /// Whether an OLA daemon answered on the DMX port.
    pub ola_port: Option<u16>,

    /// Every output device found, for diagnostics.
    pub all_audio_out: Vec<AudioOutput>,
    /// Every input device found, for diagnostics.
    pub all_audio_in: Vec<AudioInput>,
    /// Every MIDI device found, for diagnostics.
    pub all_midi: Vec<MidiPort>,

    skips: Vec<Skip>,
}

/// Devices that are not real hardware and must never be selected.
fn is_synthetic(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains(":null") || lower.starts_with("mock") || lower.contains("dummy")
}

/// Whether a name refers to a real ALSA hardware device rather than a plug
/// layer in front of one.
///
/// This matters more than it looks. ALSA's `plughw`, `sysdefault`, `default`
/// and `pulse` nodes advertise 32 channels regardless of the hardware behind
/// them, because the plug layer will convert anything. Ranking by channel
/// count alone therefore picks a fictional 32-channel device over the real
/// 8-channel interface, and the resulting track mappings address channels that
/// do not physically exist.
fn is_real_hardware(name: &str) -> bool {
    name.contains("hw:CARD=") && !name.contains("plughw:")
}

/// Ranks a device for selection: most specific hardware first, then channels.
///
/// Naming a card matters more than channel count. A box with a real interface
/// also exposes generic `default` and `pulse` nodes that report the same
/// (fictional) 32 channels, so ranking on channels alone ties and picks
/// whichever came last -- which on a production rig meant testing through
/// PulseAudio instead of the console sitting right there.
///
/// Tiers, highest first:
///   2. a raw hardware device for a named card
///   1. any device for a named card (plug layer in front of real hardware)
///   0. generic routes such as `default` or `pulse`
fn device_rank(name: &str, channels: u16) -> (u8, u16) {
    let tier = if is_real_hardware(name) {
        2
    } else if name.contains("CARD=") {
        1
    } else {
        0
    };
    (tier, channels)
}

/// Reads a device-name override from the environment.
fn env_override(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.trim().is_empty())
}

/// Subsystems the operator asked to be treated as absent.
fn disabled_subsystems() -> Vec<String> {
    env_override("MTRACK_E2E_DISABLE")
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

impl Capabilities {
    /// Probes the machine once per process.
    pub fn get() -> &'static Capabilities {
        static CAPS: OnceLock<Capabilities> = OnceLock::new();
        CAPS.get_or_init(Capabilities::probe)
    }

    fn probe() -> Capabilities {
        let mut skips = Vec::new();

        let all_audio_out = probe_audio_outputs(&mut skips);
        let all_audio_in = probe_audio_inputs();
        let all_midi = probe_midi(&mut skips);

        let mut audio_out = select_audio_output(&all_audio_out, &mut skips);
        let mut audio_in = select_audio_input(&all_audio_in, &mut skips);
        let (mut midi_out, mut midi_in) = select_midi(&all_midi, &mut skips);
        let mut ola_port = probe_ola(&mut skips);

        // `MTRACK_E2E_DISABLE=midi,audio,dmx` pretends a subsystem is absent.
        // A machine that has everything cannot otherwise exercise the paths a
        // machine that is missing something will take, and those paths are the
        // ones most likely to rot unnoticed.
        for subsystem in disabled_subsystems() {
            match subsystem.as_str() {
                "audio" => {
                    audio_out = None;
                    audio_in = None;
                    skips.push(Skip {
                        area: "audio",
                        reason: "disabled via MTRACK_E2E_DISABLE".to_string(),
                    });
                }
                "midi" => {
                    midi_out = None;
                    midi_in = None;
                    skips.push(Skip {
                        area: "midi",
                        reason: "disabled via MTRACK_E2E_DISABLE".to_string(),
                    });
                }
                "dmx" => {
                    ola_port = None;
                    skips.push(Skip {
                        area: "dmx",
                        reason: "disabled via MTRACK_E2E_DISABLE".to_string(),
                    });
                }
                other => println!("  (ignoring unknown MTRACK_E2E_DISABLE entry '{other}')"),
            }
        }

        Capabilities {
            audio_out,
            audio_in,
            midi_out,
            midi_in,
            ola_port,
            all_audio_out,
            all_audio_in,
            all_midi,
            skips,
        }
    }

    /// The hardware summary shown at the top of the report.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(match &self.audio_out {
            Some(d) => format!("audio out: {} ({} ch, {})", d.name, d.max_channels, d.host),
            None => "audio out: none".to_string(),
        });
        lines.push(match &self.audio_in {
            Some(d) => format!("audio in:  {} ({} ch, {})", d.name, d.max_channels, d.host),
            None => "audio in:  none".to_string(),
        });
        lines.push(match &self.midi_out {
            Some(d) => format!("midi out:  {}", d.name),
            None => "midi out:  none".to_string(),
        });
        lines.push(match &self.midi_in {
            Some(d) => format!("midi in:   {}", d.name),
            None => "midi in:   none".to_string(),
        });
        lines.push(match self.ola_port {
            Some(p) => format!("dmx:       olad on port {p}"),
            None => "dmx:       no olad (null client only)".to_string(),
        });
        lines
    }

    /// Skips discovered during the probe itself.
    pub fn probe_skips(&self) -> &[Skip] {
        &self.skips
    }

    /// Whether the chosen output device has enough channels to prove that
    /// distinct tracks land on distinct physical outputs.
    pub fn has_multichannel_output(&self) -> bool {
        self.audio_out.as_ref().is_some_and(|d| d.max_channels >= 4)
    }

    /// Whether both MIDI directions are available, which is what byte-exact
    /// assertions (beat clock, note timing) require.
    pub fn has_midi_loopback(&self) -> bool {
        self.midi_out.is_some() && self.midi_in.is_some()
    }

    /// Prints what the suite is about to exercise. Called once at startup so a
    /// human watching the run knows the coverage before it begins.
    pub fn print_banner(&self) {
        println!("\n=== mtrack hardware e2e: detected hardware ===");

        match &self.audio_out {
            Some(d) => println!(
                "  audio out : {} ({} ch, {}) {}",
                d.name,
                d.max_channels,
                d.host,
                if self.has_multichannel_output() {
                    "[multichannel routing checks ON]"
                } else {
                    "[<4 ch: routing checks limited]"
                }
            ),
            None => println!("  audio out : NONE"),
        }
        match &self.audio_in {
            Some(d) => println!(
                "  audio in  : {} ({} ch, {}) [loopback capture available]",
                d.name, d.max_channels, d.host
            ),
            None => println!("  audio in  : NONE [no loopback capture]"),
        }
        match (&self.midi_out, &self.midi_in) {
            (Some(o), Some(i)) => {
                println!("  midi out  : {}", o.name);
                println!("  midi in   : {} [byte-level MIDI checks ON]", i.name);
            }
            (Some(o), None) => println!("  midi out  : {} [send only, no verification]", o.name),
            (None, _) => println!("  midi      : NONE"),
        }
        match self.ola_port {
            Some(p) => println!("  dmx       : olad on port {p} [DMX output checks ON]"),
            None => println!("  dmx       : no olad [null client only]"),
        }

        if !self.skips.is_empty() {
            println!("\n  Cannot exercise:");
            for skip in &self.skips {
                println!("    - {}: {}", skip.area, skip.reason);
            }
        }
        println!("=============================================\n");
    }
}

/// Lists output devices the player can actually open.
///
/// Deliberately intersects the two enumerations. `list_device_info` is what the
/// web UI offers, but `list_devices` is what `Device::get` searches when
/// opening, and on real hardware the first is a superset of the second. Picking
/// from the advertised list alone produces a profile naming a device the player
/// then refuses with "no device found" -- which is exactly what happened
/// against a WING console, wasting a full run on 45-second timeouts.
///
/// The discrepancy itself is a defect, reported by the `devices` area; this is
/// the harness declining to trip over it.
fn probe_audio_outputs(skips: &mut Vec<Skip>) -> Vec<AudioOutput> {
    let openable: Vec<String> = match mtrack::audio::list_devices() {
        Ok(devices) => devices
            .into_iter()
            .map(|d| {
                let display = d.to_string();
                display
                    .split(" (Channels=")
                    .next()
                    .unwrap_or(&display)
                    .to_string()
            })
            .collect(),
        Err(e) => {
            skips.push(Skip {
                area: "audio",
                reason: format!("could not list openable audio devices: {e}"),
            });
            Vec::new()
        }
    };

    match mtrack::audio::list_device_info() {
        Ok(infos) => infos
            .into_iter()
            .filter(|i| openable.iter().any(|o| o == &i.name))
            .map(|i| AudioOutput {
                name: i.name,
                max_channels: i.max_channels,
                host: i.host_name,
                sample_rates: i.supported_sample_rates,
            })
            .collect(),
        Err(e) => {
            skips.push(Skip {
                area: "audio",
                reason: format!("could not list audio output devices: {e}"),
            });
            Vec::new()
        }
    }
}

/// Lists input devices directly from cpal. mtrack's listing filters to devices
/// with output configs, so input-only interfaces would otherwise be invisible.
fn probe_audio_inputs() -> Vec<AudioInput> {
    // Device enumeration on ALSA writes directly to the process's stderr.
    let _stdout = shh::stdout();
    let _stderr = shh::stderr();

    let mut inputs = Vec::new();
    for host_id in cpal::available_hosts() {
        let Ok(host) = cpal::host_from_id(host_id) else {
            continue;
        };
        let Ok(devices) = host.input_devices() else {
            continue;
        };
        for device in devices {
            let Ok(id) = device.id() else { continue };
            let max_channels = device
                .supported_input_configs()
                .map(|configs| configs.map(|c| c.channels()).max().unwrap_or(0))
                .unwrap_or(0);
            if max_channels == 0 {
                continue;
            }
            inputs.push(AudioInput {
                name: id.to_string(),
                max_channels,
                host: host_id.name().to_string(),
            });
        }
    }
    inputs
}

/// Lists MIDI devices through mtrack's own enumeration.
fn probe_midi(skips: &mut Vec<Skip>) -> Vec<MidiPort> {
    match mtrack::midi::list_device_info() {
        Ok(infos) => infos
            .into_iter()
            .map(|i| MidiPort {
                name: i.name,
                has_input: i.has_input,
                has_output: i.has_output,
            })
            .collect(),
        Err(e) => {
            skips.push(Skip {
                area: "midi",
                reason: format!("MIDI subsystem unavailable: {e}"),
            });
            Vec::new()
        }
    }
}

/// Picks the output device with the most channels, since channel count is what
/// gates the routing checks. Honours `MTRACK_E2E_AUDIO_DEVICE`.
fn select_audio_output(all: &[AudioOutput], skips: &mut Vec<Skip>) -> Option<AudioOutput> {
    if let Some(name) = env_override("MTRACK_E2E_AUDIO_DEVICE") {
        return match all.iter().find(|d| d.name == name) {
            Some(d) => Some(d.clone()),
            None => {
                skips.push(Skip {
                    area: "audio",
                    reason: format!("MTRACK_E2E_AUDIO_DEVICE={name} matched no device"),
                });
                None
            }
        };
    }

    let chosen = all
        .iter()
        .filter(|d| !is_synthetic(&d.name))
        .max_by_key(|d| device_rank(&d.name, d.max_channels))
        .cloned();

    match &chosen {
        None => skips.push(Skip {
            area: "audio",
            reason: "no non-synthetic audio output device found".to_string(),
        }),
        Some(d) if d.max_channels < 4 => skips.push(Skip {
            area: "audio-routing",
            reason: format!(
                "{} has only {} output channels; per-track channel routing needs 4+",
                d.name, d.max_channels
            ),
        }),
        Some(_) => {}
    }
    chosen
}

/// Picks an input device for loopback capture. Honours
/// `MTRACK_E2E_AUDIO_INPUT`. Whether a cable is actually patched is proven at
/// runtime by the loopback probe, not assumed here.
fn select_audio_input(all: &[AudioInput], skips: &mut Vec<Skip>) -> Option<AudioInput> {
    if let Some(name) = env_override("MTRACK_E2E_AUDIO_INPUT") {
        return match all.iter().find(|d| d.name == name) {
            Some(d) => Some(d.clone()),
            None => {
                skips.push(Skip {
                    area: "audio-loopback",
                    reason: format!("MTRACK_E2E_AUDIO_INPUT={name} matched no device"),
                });
                None
            }
        };
    }

    let chosen = all
        .iter()
        .filter(|d| !is_synthetic(&d.name))
        .max_by_key(|d| device_rank(&d.name, d.max_channels))
        .cloned();

    if chosen.is_none() {
        skips.push(Skip {
            area: "audio-loopback",
            reason: "no audio input device found; cannot verify audio actually left the interface"
                .to_string(),
        });
    }
    chosen
}

/// Picks MIDI send and listen ports, preferring a single bidirectional device
/// (a physical DIN loop or an ALSA virtual port pair). Honours
/// `MTRACK_E2E_MIDI_DEVICE` and `MTRACK_E2E_MIDI_INPUT`.
fn select_midi(all: &[MidiPort], skips: &mut Vec<Skip>) -> (Option<MidiPort>, Option<MidiPort>) {
    let find = |name: &str| all.iter().find(|d| d.name == name).cloned();

    let out = match env_override("MTRACK_E2E_MIDI_DEVICE") {
        Some(name) => {
            // Must be able to transmit. The auto path filters on this; without
            // the same filter here an input-only port becomes the "output",
            // Need::MidiOut reports satisfied, and mtrack is blamed for a typo.
            let found = find(&name).filter(|d| d.has_output);
            if found.is_none() {
                skips.push(Skip {
                    area: "midi",
                    reason: format!(
                        "MTRACK_E2E_MIDI_DEVICE={name} matched no device with an output port"
                    ),
                });
            }
            found
        }
        None => all
            .iter()
            .filter(|d| d.has_output && !is_synthetic(&d.name))
            // Prefer a device that can also receive, so we can verify what we
            // sent; then real hardware over the OS through-port, which loops
            // back with no cable and is not what an operator would drive. The
            // second key matters once a rig has a real DIN loop: without it the
            // through-port wins on enumeration order, and the checks report
            // running on hardware while the configured device says otherwise.
            .max_by_key(|d| {
                (
                    d.has_input as u8,
                    !crate::discovery::is_through_port(&d.name) as u8,
                )
            })
            .cloned(),
    };

    let in_ = match env_override("MTRACK_E2E_MIDI_INPUT") {
        Some(name) => {
            // Must be able to receive, mirroring the output override above.
            // An output-only port here yields a midi_in that cannot listen,
            // and has_midi_loopback() would then claim a loopback exists.
            let found = find(&name).filter(|d| d.has_input);
            if found.is_none() {
                skips.push(Skip {
                    area: "midi",
                    reason: format!(
                        "MTRACK_E2E_MIDI_INPUT={name} matched no device with an input port"
                    ),
                });
            }
            found
        }
        None => out.as_ref().filter(|o| o.has_input).cloned().or_else(|| {
            all.iter()
                .find(|d| d.has_input && !is_synthetic(&d.name))
                .cloned()
        }),
    };

    if out.is_none() {
        skips.push(Skip {
            area: "midi",
            reason: "no MIDI output device found".to_string(),
        });
    } else if in_.is_none() {
        skips.push(Skip {
            area: "midi-verification",
            reason:
                "no MIDI input port; can send MIDI but cannot verify beat clock or note timing \
                     (try `sudo modprobe snd-virmidi`)"
                    .to_string(),
        });
    }

    (out, in_)
}

/// Checks whether an OLA daemon is listening. Honours `MTRACK_E2E_OLA_PORT`.
fn probe_ola(skips: &mut Vec<Skip>) -> Option<u16> {
    let port = env_override("MTRACK_E2E_OLA_PORT")
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(mtrack::config::DEFAULT_OLA_PORT);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
        Ok(_) => Some(port),
        Err(e) => {
            skips.push(Skip {
                area: "dmx-output",
                reason: format!(
                    "no OLA daemon on 127.0.0.1:{port} ({e}); lighting runs against the null \
                     client, so DMX frames are not verified"
                ),
            });
            None
        }
    }
}

/// Groups devices by host for readable diagnostics.
#[allow(dead_code)]
pub fn by_host(devices: &[AudioOutput]) -> BTreeMap<String, Vec<&AudioOutput>> {
    let mut map: BTreeMap<String, Vec<&AudioOutput>> = BTreeMap::new();
    for d in devices {
        map.entry(d.host.clone()).or_default().push(d);
    }
    map
}
