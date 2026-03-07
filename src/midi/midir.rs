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
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt, mem,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use midir::{MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputPort};
use midly::live::LiveEvent;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info, span, warn, Level};

use crate::{
    config,
    dmx::{self, engine::Engine},
    playsync::CancelHandle,
    songs::Song,
};
use std::sync::Barrier;

use super::transform::{ControlChangeMapper, MidiTransformer, NoteMapper};

/// Trait abstracting MIDI output so we can test without hardware.
pub(crate) trait MidiSender: Send {
    fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>>;
}

/// Real midir implementation.
impl MidiSender for midir::MidiOutputConnection {
    fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        midir::MidiOutputConnection::send(self, bytes)?;
        Ok(())
    }
}

/// Return type for `build_transformers`: DMX channel mappings and MIDI transformers per channel.
type TransformerConfig = (HashMap<u8, String>, HashMap<u8, Vec<MidiTransformer>>);

pub struct Device {
    name: String,
    playback_delay: Duration,
    input_port: Option<MidiInputPort>,
    output_port: Option<MidiOutputPort>,
    event_connection: Box<Mutex<Option<MidiInputConnection<()>>>>,
    midi_to_dmx_mappings: HashMap<u8, String>,
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    dmx_midi_transformers: HashMap<u8, Vec<MidiTransformer>>,
}

impl Device {
    fn new_default(name: String) -> Self {
        Device {
            name,
            playback_delay: Duration::ZERO,
            input_port: None,
            output_port: None,
            event_connection: Box::new(Mutex::new(None)),
            midi_to_dmx_mappings: HashMap::new(),
            dmx_engine: None,
            dmx_midi_transformers: HashMap::new(),
        }
    }
}

impl super::Device for Device {
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "wait for event (midir)");
        let _enter = span.enter();
        let dmx_engine = self.dmx_engine.clone();
        let midi_to_dmx_mappings = self.midi_to_dmx_mappings.clone();
        let dmx_midi_transformers = self.dmx_midi_transformers.clone();

        let mut event_connection = self.event_connection.lock().expect("unable to get lock");
        if event_connection.is_some() {
            return Err("Already watching events.".into());
        }

        info!("Watching MIDI events.");

        let input_port = match self.input_port.as_ref() {
            Some(input_port) => input_port,
            None => {
                warn!("No MIDI output device configured, cannot listen for events.");
                return Ok(());
            }
        };

        // Spawn a thread to handle parsing and sending of MIDI events to the DMX engine if we're configured to do so.
        let dmx_sender = dmx_engine.map(|dmx_engine| {
            let (dmx_sender, dmx_receiver) = mpsc::channel::<Vec<u8>>();
            thread::spawn(move || {
                info!("Passing MIDI events to the DMX engine.");
                loop {
                    match dmx_receiver.recv() {
                        Ok(event) => {
                            if let Ok(LiveEvent::Midi { channel, message }) =
                                LiveEvent::parse(&event)
                            {
                                route_midi_to_dmx(
                                    channel.as_int(),
                                    message,
                                    &midi_to_dmx_mappings,
                                    &dmx_midi_transformers,
                                    &|universe, msg| {
                                        dmx_engine.handle_midi_event(universe, msg);
                                    },
                                );
                            }
                        }
                        Err(_) => return,
                    }
                }
            });

            dmx_sender
        });

        let input = MidiInput::new("mtrack player input")?;
        *event_connection = Some(input.connect(
            input_port,
            "mtrack input watcher",
            move |_, raw_event, _| {
                if let Some(dmx_sender) = &dmx_sender {
                    if let Err(e) = dmx_sender.send(raw_event.into()) {
                        error!(
                            err = format!("{:?}", e),
                            "Error sending MIDI event to DMX engine."
                        );
                    }
                }
                if let Err(e) = sender.blocking_send(Vec::from(raw_event)) {
                    error!(
                        err = format!("{:?}", e),
                        "Error sending MIDI event to receiver."
                    );
                }
            },
            (),
        )?);

        Ok(())
    }

    /// Stops watching events.
    fn stop_watch_events(&self) {
        // Explicitly drop the connection.
        let event_connection = self
            .event_connection
            .lock()
            .expect("error getting mutex")
            .take();

        mem::drop(event_connection);
    }

    /// Plays the given song through the MIDI interface, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
        start_time: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (midir)");
        let _enter = span.enter();

        let output_port = match self.output_port.as_ref() {
            Some(output_port) => output_port,
            None => {
                warn!(
                    song = song.name(),
                    "No MIDI output device configured, cannot play song."
                );
                return Ok(());
            }
        };

        let midi_playback = match song.midi_playback() {
            Some(midi_playback) => midi_playback,
            None => {
                info!(song = song.name(), "Song has no MIDI sheet.");
                return Ok(());
            }
        };
        let midi_sheet = midi_playback.midi_sheet()?;
        let output = MidiOutput::new("mtrack player output")?;

        let exclude_midi_channels = HashSet::from_iter(midi_playback.exclude_midi_channels());

        info!(
            device = self.name,
            song = song.name(),
            duration = song.duration_string(),
            start_time = ?start_time,
            "Playing song MIDI."
        );

        let finished = Arc::new(AtomicBool::new(false));
        let playback_delay = self.playback_delay;
        let mut connection = output.connect(output_port, "mtrack player")?;

        let join_handle = {
            let cancel_handle = cancel_handle.clone();
            let finished = finished.clone();

            thread::spawn(move || {
                run_playback(
                    &mut connection,
                    PlaybackContext {
                        precomputed: &midi_sheet.precomputed,
                        start_time,
                        playback_delay,
                        cancel_handle: &cancel_handle,
                        play_barrier,
                        finished,
                        exclude_channels: &exclude_midi_channels,
                    },
                );
            })
        };

        cancel_handle.wait(finished);

        if cancel_handle.is_cancelled() {
            info!("MIDI playback has been cancelled.");
        }

        if join_handle.join().is_err() {
            return Err("Error while joining thread!".into());
        }

        info!("MIDI playback stopped.");

        Ok(())
    }

    fn emit(&self, midi_event: Option<LiveEvent<'static>>) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "emit (midir)");
        let _enter = span.enter();

        let event = match midi_event {
            Some(midi_event) => midi_event,
            // If there's no event, return early.
            None => return Ok(()),
        };

        let output_port = match &self.output_port {
            Some(output_port) => output_port,
            None => {
                warn!("No MIDI output device configured, cannot emit event.");
                return Ok(());
            }
        };

        let output = MidiOutput::new("mtrack emit output")?;

        debug!(
            device = self.name,
            event = format!("{:?}", event),
            "Emitting event."
        );

        let mut buf: Vec<u8> = Vec::with_capacity(8);
        event.write(&mut buf)?;
        let mut connection = output.connect(output_port, "mtrack player")?;

        connection.send(&buf)?;

        Ok(())
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<super::mock::Device>, Box<dyn Error>> {
        Err("not a mock".into())
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut capabilities: Vec<String> = Vec::new();
        if self.input_port.is_some() {
            capabilities.push(String::from("Input"));
        }
        if self.output_port.is_some() {
            capabilities.push(String::from("Output"));
        }

        write!(f, "{} ({})", self.name, capabilities.join("/"))
    }
}

/// Lists midir devices and produces the Device trait.
pub fn list() -> Result<Vec<Box<dyn super::Device>>, Box<dyn Error>> {
    Ok(list_midir_devices()?
        .into_iter()
        .map(|device| {
            let device: Box<dyn super::Device> = Box::new(device);
            device
        })
        .collect())
}

/// Lists midir devices.
fn list_midir_devices() -> Result<Vec<Device>, Box<dyn Error>> {
    let input = MidiInput::new("mtrack input listing")?;
    let output = MidiOutput::new("mtrack output listing")?;
    let input_ports = input.ports();
    let output_ports = output.ports();

    let mut devices: HashMap<String, Device> = HashMap::new();

    for port in input_ports {
        let name = input.port_name(&port)?;
        devices.entry(name.clone()).or_insert_with(|| {
            let mut device = Device::new_default(name);
            device.input_port = Some(port);
            device
        });
    }

    for port in output_ports {
        let name = output.port_name(&port)?;
        match devices.get_mut(&name) {
            Some(device) => {
                device.output_port = Some(port);
            }
            None => {
                let mut device = Device::new_default(name.clone());
                device.output_port = Some(port);
                devices.insert(name, device);
            }
        }
    }

    let mut sorted_devices = devices
        .into_iter()
        .map(|entry| entry.1)
        .collect::<Vec<Device>>();
    sorted_devices.sort_by_key(|device| device.name.clone());
    Ok(sorted_devices)
}

/// Validates that exactly one device matches the given name.
/// Returns an error if no devices match or if the match is ambiguous.
fn validate_device_match<T: fmt::Display>(name: &str, matches: &[T]) -> Result<(), Box<dyn Error>> {
    if matches.is_empty() {
        return Err(format!("no device found with name {}", name).into());
    }
    if matches.len() > 1 {
        return Err(format!(
            "found too many devices that match ({}), use a less ambiguous device name",
            matches
                .iter()
                .map(|device| format!("{}", device))
                .collect::<Vec<String>>()
                .join(", ")
        )
        .into());
    }
    Ok(())
}

/// Gets the given midir device.
pub fn get(
    config: &config::Midi,
    dmx_engine: Option<Arc<Engine>>,
) -> Result<Device, Box<dyn Error>> {
    let playback_delay = config.playback_delay()?;
    let name = config.device();
    let mut matches = list_midir_devices()?
        .into_iter()
        .filter(|device| device.name.contains(name))
        .collect::<Vec<Device>>();

    validate_device_match(name, &matches)?;

    let (midi_to_dmx_mappings, dmx_midi_transformers) = build_transformers(config)?;

    let mut midi_device = matches.swap_remove(0);
    midi_device.playback_delay = playback_delay;
    midi_device.midi_to_dmx_mappings = midi_to_dmx_mappings;
    midi_device.dmx_engine = dmx_engine;
    midi_device.dmx_midi_transformers = dmx_midi_transformers;

    // We've verified that there's only one element in the vector, so this should be safe.
    Ok(midi_device)
}

/// Builds MIDI-to-DMX channel mappings and transformers from config.
fn build_transformers(config: &config::Midi) -> Result<TransformerConfig, Box<dyn Error>> {
    let mut midi_to_dmx_mappings = HashMap::new();
    let mut dmx_midi_transformers: HashMap<u8, Vec<MidiTransformer>> = HashMap::new();

    for midi_to_dmx in config.midi_to_dmx() {
        let midi_channel = midi_to_dmx.midi_channel()?.as_int();
        midi_to_dmx_mappings.insert(midi_channel, midi_to_dmx.universe());

        let mut transformers = Vec::new();
        for transformer in midi_to_dmx.transformers() {
            transformers.push(match transformer {
                config::MidiTransformer::NoteMapper(note_mapper) => {
                    info!(
                        input_note = note_mapper.input_note()?.as_int(),
                        "Configuring note mapper transformer"
                    );
                    MidiTransformer::NoteMapper(NoteMapper::new(
                        note_mapper.input_note()?,
                        note_mapper.convert_to_notes()?,
                    ))
                }
                config::MidiTransformer::ControlChangeMapper(control_change_mapper) => {
                    info!(
                        input_controller = control_change_mapper.input_controller()?.as_int(),
                        "Configuring control change mapper transformer"
                    );
                    MidiTransformer::ControlChangeMapper(ControlChangeMapper::new(
                        control_change_mapper.input_controller()?,
                        control_change_mapper.convert_to_notes()?,
                    ))
                }
            })
        }

        dmx_midi_transformers
            .entry(midi_channel)
            .or_default()
            .extend(transformers);
    }

    Ok((midi_to_dmx_mappings, dmx_midi_transformers))
}

/// Routes a MIDI event to DMX by looking up the channel→universe mapping,
/// applying any configured transformers, and emitting the result via `emit`.
/// If transformers produce output, the original message is suppressed.
fn route_midi_to_dmx(
    channel: u8,
    message: midly::MidiMessage,
    midi_to_dmx_mappings: &HashMap<u8, String>,
    dmx_midi_transformers: &HashMap<u8, Vec<MidiTransformer>>,
    emit: &dyn Fn(String, midly::MidiMessage),
) {
    let universe = match midi_to_dmx_mappings.get(&channel) {
        Some(u) => u,
        None => return,
    };

    let mut transformed = false;

    if let Some(transformers_for_channel) = dmx_midi_transformers.get(&channel) {
        for transformer in transformers_for_channel {
            if transformer.can_process(&message) {
                for transformed_message in transformer.transform(&message) {
                    transformed = true;
                    emit(universe.clone(), transformed_message);
                }
            }
        }
    }

    // Only send the original message if no transformer produced output.
    if !transformed {
        emit(universe.clone(), message);
    }
}

/// Serializes a MIDI event to bytes, filtering out excluded channels.
/// Returns `Some(bytes)` if the event should be sent, `None` if it should be skipped.
fn serialize_midi_event(
    event: &super::playback::TimedMidiEvent,
    exclude_channels: &HashSet<u8>,
    buf: &mut Vec<u8>,
) -> Option<Vec<u8>> {
    if !exclude_channels.is_empty() && exclude_channels.contains(&event.channel) {
        return None;
    }
    let live_event = midly::live::LiveEvent::Midi {
        channel: event.channel.into(),
        message: event.message,
    };
    buf.clear();
    if live_event.write_std(&mut *buf).is_ok() {
        Some(buf.clone())
    } else {
        None
    }
}

/// Parameters for MIDI playback synchronization and timing.
struct PlaybackContext<'a> {
    precomputed: &'a super::playback::PrecomputedMidi,
    start_time: Duration,
    playback_delay: Duration,
    cancel_handle: &'a CancelHandle,
    play_barrier: Arc<Barrier>,
    finished: Arc<AtomicBool>,
    exclude_channels: &'a HashSet<u8>,
}

/// Runs the MIDI playback thread body: waits on the barrier, sleeps through
/// the playback delay (checking for cancellation), then plays events.
fn run_playback(sender: &mut dyn MidiSender, ctx: PlaybackContext<'_>) {
    ctx.play_barrier.wait();

    if ctx.cancel_handle.is_cancelled() {
        ctx.finished.store(true, Ordering::Relaxed);
        ctx.cancel_handle.notify();
        return;
    }

    // Sleep the playback delay in small increments so we can
    // respond to cancellation promptly.
    {
        let start = std::time::Instant::now();
        while start.elapsed() < ctx.playback_delay {
            if ctx.cancel_handle.is_cancelled() {
                ctx.finished.store(true, Ordering::Relaxed);
                ctx.cancel_handle.notify();
                return;
            }
            let remaining = ctx.playback_delay.saturating_sub(start.elapsed());
            spin_sleep::sleep(remaining.min(Duration::from_millis(50)));
        }
    }

    play_precomputed(
        ctx.precomputed,
        ctx.start_time,
        sender,
        ctx.cancel_handle,
        ctx.exclude_channels,
    );

    ctx.finished.store(true, Ordering::Relaxed);
    ctx.cancel_handle.notify();
}

/// Plays pre-computed MIDI events through a MIDI sender.
/// Sleeps between events using spin_sleep for precision without busy-waiting.
fn play_precomputed(
    precomputed: &super::playback::PrecomputedMidi,
    start_time: Duration,
    sender: &mut dyn MidiSender,
    cancel_handle: &CancelHandle,
    exclude_channels: &HashSet<u8>,
) {
    let events = precomputed.events_from(start_time);
    let wall_start = std::time::Instant::now();
    let mut buf = Vec::with_capacity(8);

    for event in events {
        if cancel_handle.is_cancelled() {
            return;
        }

        let target_wall = event.time - start_time;
        let elapsed = wall_start.elapsed();
        if target_wall > elapsed {
            spin_sleep::sleep(target_wall - elapsed);
        }
        if cancel_handle.is_cancelled() {
            return;
        }

        if let Some(bytes) = serialize_midi_event(event, exclude_channels, &mut buf) {
            if let Err(e) = sender.send(&bytes) {
                debug!("MIDI send failed: {:?}", e);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use midly::num::u7;
    use midly::MidiMessage;

    mod display {
        use super::*;

        #[test]
        fn display_no_ports() {
            let device = Device::new_default("test-device".to_string());
            assert_eq!(format!("{}", device), "test-device ()");
        }

        #[test]
        fn display_input_only() {
            let device = Device::new_default("test-device".to_string());
            assert!(device.input_port.is_none());
            assert!(device.output_port.is_none());
            let display = format!("{}", device);
            assert!(display.contains("test-device"));
        }

        #[test]
        fn new_default_has_empty_fields() {
            let device = Device::new_default("my-midi".to_string());
            assert_eq!(device.name, "my-midi");
            assert_eq!(device.playback_delay, Duration::ZERO);
            assert!(device.input_port.is_none());
            assert!(device.output_port.is_none());
            assert!(device.midi_to_dmx_mappings.is_empty());
            assert!(device.dmx_engine.is_none());
            assert!(device.dmx_midi_transformers.is_empty());
        }
    }

    mod build_transformers_tests {
        use super::*;

        #[test]
        fn empty_config_produces_empty_mappings() {
            let config = config::Midi::new("dev", None);
            let (mappings, transformers) = build_transformers(&config).unwrap();
            assert!(mappings.is_empty());
            assert!(transformers.is_empty());
        }

        #[test]
        fn midi_to_dmx_mapping_without_transformers() {
            let yaml = r#"
                device: test
                midi_to_dmx:
                  - midi_channel: 10
                    universe: "main"
            "#;
            let config: config::Midi = serde_yml::from_str(yaml).unwrap();
            let (mappings, transformers) = build_transformers(&config).unwrap();

            // MIDI channel 10 → internal channel 9 (1-indexed to 0-indexed in config)
            assert_eq!(mappings.get(&9), Some(&"main".to_string()));
            // No transformers configured.
            assert!(transformers.get(&9).map_or(true, |t| t.is_empty()) || transformers.is_empty());
        }

        #[test]
        fn midi_to_dmx_with_note_mapper() {
            let yaml = r#"
                device: test
                midi_to_dmx:
                  - midi_channel: 1
                    universe: "main"
                    transformers:
                      - type: note_mapper
                        input_note: 60
                        convert_to_notes: [61, 62]
            "#;
            let config: config::Midi = serde_yml::from_str(yaml).unwrap();
            let (mappings, transformers) = build_transformers(&config).unwrap();

            assert_eq!(mappings.get(&0), Some(&"main".to_string()));
            let channel_transformers = transformers.get(&0).unwrap();
            assert_eq!(channel_transformers.len(), 1);

            // Verify the transformer processes note messages.
            let note_on = MidiMessage::NoteOn {
                key: u7::new(60),
                vel: u7::new(100),
            };
            assert!(channel_transformers[0].can_process(&note_on));

            // NoteMapper doesn't process control change messages.
            let cc = MidiMessage::Controller {
                controller: u7::new(1),
                value: u7::new(127),
            };
            assert!(!channel_transformers[0].can_process(&cc));
        }

        #[test]
        fn midi_to_dmx_with_control_change_mapper() {
            let yaml = r#"
                device: test
                midi_to_dmx:
                  - midi_channel: 2
                    universe: "lights"
                    transformers:
                      - type: control_change_mapper
                        input_controller: 1
                        convert_to_controllers: [60, 61]
            "#;
            let config: config::Midi = serde_yml::from_str(yaml).unwrap();
            let (mappings, transformers) = build_transformers(&config).unwrap();

            assert_eq!(mappings.get(&1), Some(&"lights".to_string()));
            let channel_transformers = transformers.get(&1).unwrap();
            assert_eq!(channel_transformers.len(), 1);

            // Verify it processes control change messages.
            let cc = MidiMessage::Controller {
                controller: u7::new(1),
                value: u7::new(127),
            };
            assert!(channel_transformers[0].can_process(&cc));
        }

        #[test]
        fn multiple_channels_and_transformers() {
            let yaml = r#"
                device: test
                midi_to_dmx:
                  - midi_channel: 1
                    universe: "universe_a"
                    transformers:
                      - type: note_mapper
                        input_note: 60
                        convert_to_notes: [61]
                      - type: note_mapper
                        input_note: 72
                        convert_to_notes: [73, 74]
                  - midi_channel: 10
                    universe: "universe_b"
            "#;
            let config: config::Midi = serde_yml::from_str(yaml).unwrap();
            let (mappings, transformers) = build_transformers(&config).unwrap();

            assert_eq!(mappings.len(), 2);
            assert_eq!(mappings.get(&0), Some(&"universe_a".to_string()));
            assert_eq!(mappings.get(&9), Some(&"universe_b".to_string()));

            // Channel 0 should have 2 transformers.
            assert_eq!(transformers.get(&0).unwrap().len(), 2);
        }
    }

    mod serialize_midi_event_tests {
        use super::*;
        use crate::midi::playback::TimedMidiEvent;

        fn make_event(channel: u8, key: u8) -> TimedMidiEvent {
            TimedMidiEvent {
                time: Duration::from_millis(100),
                channel,
                message: MidiMessage::NoteOn {
                    key: u7::new(key),
                    vel: u7::new(100),
                },
            }
        }

        #[test]
        fn serializes_event_with_no_exclusions() {
            let event = make_event(0, 60);
            let exclude = HashSet::new();
            let mut buf = Vec::new();

            let result = serialize_midi_event(&event, &exclude, &mut buf);
            assert!(result.is_some());
            let bytes = result.unwrap();
            assert!(!bytes.is_empty());
        }

        #[test]
        fn excludes_matching_channel() {
            let event = make_event(5, 60);
            let exclude = HashSet::from([5]);
            let mut buf = Vec::new();

            let result = serialize_midi_event(&event, &exclude, &mut buf);
            assert!(result.is_none());
        }

        #[test]
        fn passes_non_excluded_channel() {
            let event = make_event(3, 60);
            let exclude = HashSet::from([5, 9]);
            let mut buf = Vec::new();

            let result = serialize_midi_event(&event, &exclude, &mut buf);
            assert!(result.is_some());
        }

        #[test]
        fn serialized_bytes_are_valid_midi() {
            let event = make_event(0, 60);
            let exclude = HashSet::new();
            let mut buf = Vec::new();

            let bytes = serialize_midi_event(&event, &exclude, &mut buf).unwrap();
            // Standard MIDI note-on: status byte (0x90 | channel), key, velocity
            assert_eq!(bytes.len(), 3);
            assert_eq!(bytes[0], 0x90); // Note on, channel 0
            assert_eq!(bytes[1], 60); // Key
            assert_eq!(bytes[2], 100); // Velocity
        }

        #[test]
        fn different_channels_produce_correct_status_byte() {
            let exclude = HashSet::new();
            let mut buf = Vec::new();

            for ch in 0..16u8 {
                let event = make_event(ch, 60);
                let bytes = serialize_midi_event(&event, &exclude, &mut buf).unwrap();
                assert_eq!(bytes[0], 0x90 | ch);
            }
        }

        #[test]
        fn note_off_serialization() {
            let event = TimedMidiEvent {
                time: Duration::from_millis(200),
                channel: 0,
                message: MidiMessage::NoteOff {
                    key: u7::new(60),
                    vel: u7::new(64),
                },
            };
            let exclude = HashSet::new();
            let mut buf = Vec::new();

            let bytes = serialize_midi_event(&event, &exclude, &mut buf).unwrap();
            assert_eq!(bytes.len(), 3);
            assert_eq!(bytes[0], 0x80); // Note off, channel 0
            assert_eq!(bytes[1], 60);
            assert_eq!(bytes[2], 64);
        }

        #[test]
        fn empty_exclude_set_passes_all() {
            let exclude = HashSet::new();
            let mut buf = Vec::new();

            for ch in 0..16u8 {
                let event = make_event(ch, 60);
                assert!(serialize_midi_event(&event, &exclude, &mut buf).is_some());
            }
        }
    }

    mod route_midi_to_dmx_tests {
        use super::*;
        use std::cell::RefCell;

        fn note_on(key: u8, vel: u8) -> MidiMessage {
            MidiMessage::NoteOn {
                key: u7::new(key),
                vel: u7::new(vel),
            }
        }

        fn cc(controller: u8, value: u8) -> MidiMessage {
            MidiMessage::Controller {
                controller: u7::new(controller),
                value: u7::new(value),
            }
        }

        #[test]
        fn no_mapping_emits_nothing() {
            let mappings = HashMap::new();
            let transformers = HashMap::new();
            let emitted = RefCell::new(Vec::new());

            route_midi_to_dmx(0, note_on(60, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            assert!(emitted.borrow().is_empty());
        }

        #[test]
        fn mapped_channel_emits_original_without_transformers() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "main".to_string());
            let transformers = HashMap::new();
            let emitted = RefCell::new(Vec::new());

            route_midi_to_dmx(0, note_on(60, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            let emitted = emitted.borrow();
            assert_eq!(emitted.len(), 1);
            assert_eq!(emitted[0].0, "main");
            assert_eq!(emitted[0].1, note_on(60, 100));
        }

        #[test]
        fn unmapped_channel_emits_nothing() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "main".to_string());
            let transformers = HashMap::new();
            let emitted = RefCell::new(Vec::new());

            // Send on channel 5, which has no mapping.
            route_midi_to_dmx(5, note_on(60, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            assert!(emitted.borrow().is_empty());
        }

        #[test]
        fn transformer_replaces_original_message() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "main".to_string());

            // NoteMapper: note 60 → notes 61, 62
            let mut transformers = HashMap::new();
            transformers.insert(
                0u8,
                vec![MidiTransformer::NoteMapper(NoteMapper::new(
                    u7::new(60),
                    vec![u7::new(61), u7::new(62)],
                ))],
            );
            let emitted = RefCell::new(Vec::new());

            route_midi_to_dmx(0, note_on(60, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            let emitted = emitted.borrow();
            // Should emit 2 transformed messages, NOT the original.
            assert_eq!(emitted.len(), 2);
            assert_eq!(emitted[0].1, note_on(61, 100));
            assert_eq!(emitted[1].1, note_on(62, 100));
        }

        #[test]
        fn non_matching_transformer_passes_original() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "main".to_string());

            // NoteMapper configured for note 60, but we send note 72.
            let mut transformers = HashMap::new();
            transformers.insert(
                0u8,
                vec![MidiTransformer::NoteMapper(NoteMapper::new(
                    u7::new(60),
                    vec![u7::new(61)],
                ))],
            );
            let emitted = RefCell::new(Vec::new());

            // Note 72 doesn't match — but can_process returns true for any NoteOn,
            // and transform passes through non-matching notes. So transformer will
            // produce output (the passthrough), meaning transformed=true.
            route_midi_to_dmx(0, note_on(72, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            let emitted = emitted.borrow();
            assert_eq!(emitted.len(), 1);
            assert_eq!(emitted[0].1, note_on(72, 100));
        }

        #[test]
        fn non_processable_message_type_passes_original() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "main".to_string());

            // NoteMapper can't process CC messages.
            let mut transformers = HashMap::new();
            transformers.insert(
                0u8,
                vec![MidiTransformer::NoteMapper(NoteMapper::new(
                    u7::new(60),
                    vec![u7::new(61)],
                ))],
            );
            let emitted = RefCell::new(Vec::new());

            route_midi_to_dmx(0, cc(1, 127), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            let emitted = emitted.borrow();
            // NoteMapper can't process CC, so original passes through.
            assert_eq!(emitted.len(), 1);
            assert_eq!(emitted[0].1, cc(1, 127));
        }

        #[test]
        fn multiple_transformers_on_same_channel() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "main".to_string());

            // Two NoteMappers on same channel: note 60→61 and note 72→73
            let mut transformers = HashMap::new();
            transformers.insert(
                0u8,
                vec![
                    MidiTransformer::NoteMapper(NoteMapper::new(u7::new(60), vec![u7::new(61)])),
                    MidiTransformer::NoteMapper(NoteMapper::new(u7::new(72), vec![u7::new(73)])),
                ],
            );
            let emitted = RefCell::new(Vec::new());

            // Send note 60 — first transformer matches and maps, second also processes
            // (NoteMapper.can_process is true for any NoteOn) and passes through as note 60.
            route_midi_to_dmx(0, note_on(60, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            let emitted = emitted.borrow();
            // First transformer: 60→61, second transformer: passes through 60 unchanged.
            assert_eq!(emitted.len(), 2);
            assert_eq!(emitted[0].1, note_on(61, 100));
            assert_eq!(emitted[1].1, note_on(60, 100));
        }

        #[test]
        fn emits_to_correct_universe() {
            let mut mappings = HashMap::new();
            mappings.insert(0u8, "universe_a".to_string());
            mappings.insert(1u8, "universe_b".to_string());
            let transformers = HashMap::new();
            let emitted = RefCell::new(Vec::new());

            route_midi_to_dmx(0, note_on(60, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });
            route_midi_to_dmx(1, note_on(72, 100), &mappings, &transformers, &|u, m| {
                emitted.borrow_mut().push((u, m));
            });

            let emitted = emitted.borrow();
            assert_eq!(emitted.len(), 2);
            assert_eq!(emitted[0].0, "universe_a");
            assert_eq!(emitted[1].0, "universe_b");
        }
    }

    mod validate_device_match_tests {
        use super::*;

        // Simple wrapper to provide Display for test strings.
        struct Named(String);
        impl fmt::Display for Named {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        #[test]
        fn single_match_succeeds() {
            let matches = vec![Named("device-1".to_string())];
            assert!(validate_device_match("device", &matches).is_ok());
        }

        #[test]
        fn no_matches_fails() {
            let matches: Vec<Named> = vec![];
            let err = validate_device_match("my-device", &matches).unwrap_err();
            assert!(err.to_string().contains("no device found"));
            assert!(err.to_string().contains("my-device"));
        }

        #[test]
        fn multiple_matches_fails() {
            let matches = vec![
                Named("midi-device-1".to_string()),
                Named("midi-device-2".to_string()),
            ];
            let err = validate_device_match("midi-device", &matches).unwrap_err();
            assert!(err.to_string().contains("too many devices"));
            assert!(err.to_string().contains("midi-device-1"));
            assert!(err.to_string().contains("midi-device-2"));
        }

        #[test]
        fn exactly_two_matches_fails() {
            let matches = vec![Named("a".to_string()), Named("b".to_string())];
            assert!(validate_device_match("x", &matches).is_err());
        }
    }

    #[test]
    fn to_mock_returns_error() {
        let device = Device::new_default("test".to_string());
        let result = <Device as crate::midi::Device>::to_mock(&device);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("not a mock"));
    }

    #[test]
    fn stop_watch_events_without_connection() {
        let device = Device::new_default("test".to_string());
        // Should not panic when no connection exists
        <Device as crate::midi::Device>::stop_watch_events(&device);
    }

    mod emit_tests {
        use super::*;

        #[test]
        fn emit_none_returns_ok() {
            let device = Device::new_default("test".to_string());
            let result = <Device as crate::midi::Device>::emit(&device, None);
            assert!(result.is_ok());
        }

        #[test]
        fn emit_without_output_port_returns_ok() {
            let device = Device::new_default("test".to_string());
            assert!(device.output_port.is_none());
            let event = LiveEvent::Midi {
                channel: 0.into(),
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
            };
            // Should return Ok (with a warning logged)
            let result = <Device as crate::midi::Device>::emit(&device, Some(event));
            assert!(result.is_ok());
        }
    }

    mod play_from_tests {
        use super::*;
        use crate::playsync::CancelHandle;

        fn make_song() -> (tempfile::TempDir, Arc<Song>) {
            let tmp_dir = tempfile::tempdir().unwrap();
            let wav_path = tmp_dir.path().join("test.wav");
            crate::testutil::write_wav(wav_path.clone(), vec![vec![1_i32; 44100]], 44100).unwrap();

            let song_config = config::Song::new(
                "test",
                None,
                None,
                None,
                None,
                None,
                vec![config::Track::new(
                    "test".to_string(),
                    wav_path.file_name().unwrap().to_str().unwrap(),
                    Some(1),
                )],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config).unwrap());
            (tmp_dir, song)
        }

        #[test]
        fn play_from_without_output_port_returns_ok() {
            let device = Device::new_default("test".to_string());
            let (_tmp_dir, song) = make_song();
            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(1));
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                cancel,
                barrier,
                Duration::ZERO,
            );
            assert!(result.is_ok());
        }

        #[test]
        fn play_from_without_midi_playback_returns_ok() {
            // Song has audio but no MIDI sheet
            let device = Device::new_default("test".to_string());
            let (_tmp_dir, song) = make_song();
            assert!(song.midi_playback().is_none());

            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(1));
            // Even with no output port, the no-output-port check happens first
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                cancel,
                barrier,
                Duration::ZERO,
            );
            assert!(result.is_ok());
        }
    }

    mod play_precomputed_tests {
        use super::*;
        use crate::midi::playback::{PrecomputedMidi, TimedMidiEvent};
        use crate::playsync::CancelHandle;
        use std::sync::Mutex;

        struct MockSender {
            sent: Mutex<Vec<Vec<u8>>>,
            should_fail: bool,
        }

        impl MockSender {
            fn new() -> Self {
                MockSender {
                    sent: Mutex::new(Vec::new()),
                    should_fail: false,
                }
            }

            fn failing() -> Self {
                MockSender {
                    sent: Mutex::new(Vec::new()),
                    should_fail: true,
                }
            }
        }

        impl MidiSender for MockSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if self.should_fail {
                    return Err("mock send failure".into());
                }
                self.sent.lock().unwrap().push(bytes.to_vec());
                Ok(())
            }
        }

        fn make_events(times_ms: &[u64]) -> PrecomputedMidi {
            let events: Vec<TimedMidiEvent> = times_ms
                .iter()
                .enumerate()
                .map(|(i, &t)| TimedMidiEvent {
                    time: Duration::from_millis(t),
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(60 + i as u8),
                        vel: u7::new(100),
                    },
                })
                .collect();
            PrecomputedMidi::from_events(events)
        }

        #[test]
        fn plays_all_events() {
            let midi = make_events(&[0, 0, 0]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            play_precomputed(&midi, Duration::ZERO, &mut sender, &cancel, &exclude);

            let sent = sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 3);
        }

        #[test]
        fn respects_start_time() {
            // Events at 0ms, 100ms, 200ms. Start from 100ms → skip first event.
            let midi = make_events(&[0, 100, 200]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            play_precomputed(
                &midi,
                Duration::from_millis(100),
                &mut sender,
                &cancel,
                &exclude,
            );

            let sent = sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 2);
        }

        #[test]
        fn excludes_channels() {
            let events = vec![
                TimedMidiEvent {
                    time: Duration::ZERO,
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(60),
                        vel: u7::new(100),
                    },
                },
                TimedMidiEvent {
                    time: Duration::ZERO,
                    channel: 5,
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(62),
                        vel: u7::new(100),
                    },
                },
                TimedMidiEvent {
                    time: Duration::ZERO,
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(64),
                        vel: u7::new(100),
                    },
                },
            ];
            let midi = PrecomputedMidi::from_events(events);
            let cancel = CancelHandle::new();
            let exclude = HashSet::from([5]);
            let mut sender = MockSender::new();

            play_precomputed(&midi, Duration::ZERO, &mut sender, &cancel, &exclude);

            let sent = sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 2); // Channel 5 excluded
        }

        #[test]
        fn stops_on_cancel() {
            // Create events spread over time — cancel before they all play
            let midi = make_events(&[0, 500, 1000]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            // Cancel immediately
            cancel.cancel();

            play_precomputed(&midi, Duration::ZERO, &mut sender, &cancel, &exclude);

            let sent = sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 0);
        }

        #[test]
        fn empty_events() {
            let midi = PrecomputedMidi::from_events(Vec::new());
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            play_precomputed(&midi, Duration::ZERO, &mut sender, &cancel, &exclude);

            let sent = sender.sent.lock().unwrap();
            assert!(sent.is_empty());
        }

        #[test]
        fn send_failure_continues() {
            let midi = make_events(&[0, 0, 0]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::failing();

            // Should not panic — errors are logged but playback continues
            play_precomputed(&midi, Duration::ZERO, &mut sender, &cancel, &exclude);
        }

        #[test]
        fn serialized_bytes_are_correct() {
            let events = vec![TimedMidiEvent {
                time: Duration::ZERO,
                channel: 3,
                message: midly::MidiMessage::NoteOn {
                    key: u7::new(72),
                    vel: u7::new(64),
                },
            }];
            let midi = PrecomputedMidi::from_events(events);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            play_precomputed(&midi, Duration::ZERO, &mut sender, &cancel, &exclude);

            let sent = sender.sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0], vec![0x93, 72, 64]); // NoteOn ch3, key 72, vel 64
        }
    }

    mod watch_events_tests {
        use super::*;

        #[test]
        fn watch_events_without_input_port_returns_ok() {
            let device = Device::new_default("test".to_string());
            assert!(device.input_port.is_none());
            let (tx, _rx) = tokio::sync::mpsc::channel(10);
            // Should return Ok (with a warning) when no input port
            let result = <Device as crate::midi::Device>::watch_events(&device, tx);
            assert!(result.is_ok());
        }
    }

    mod run_playback_tests {
        use super::*;
        use crate::midi::playback::PrecomputedMidi;
        use crate::playsync::CancelHandle;
        use std::sync::Mutex;

        struct MockSender {
            sent: Mutex<Vec<Vec<u8>>>,
        }

        impl MockSender {
            fn new() -> Self {
                MockSender {
                    sent: Mutex::new(Vec::new()),
                }
            }
        }

        impl MidiSender for MockSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.sent.lock().unwrap().push(bytes.to_vec());
                Ok(())
            }
        }

        fn make_events(times_ms: &[u64]) -> PrecomputedMidi {
            use crate::midi::playback::TimedMidiEvent;
            let events: Vec<TimedMidiEvent> = times_ms
                .iter()
                .enumerate()
                .map(|(i, &t)| TimedMidiEvent {
                    time: Duration::from_millis(t),
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(60 + i as u8),
                        vel: u7::new(100),
                    },
                })
                .collect();
            PrecomputedMidi::from_events(events)
        }

        #[test]
        fn normal_playback_sets_finished() {
            let midi = make_events(&[0, 0]);
            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(1));
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    play_barrier: barrier,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                },
            );

            assert!(finished.load(Ordering::Relaxed));
            assert_eq!(sender.sent.lock().unwrap().len(), 2);
        }

        #[test]
        fn cancel_before_barrier_sets_finished() {
            let midi = make_events(&[0]);
            let cancel = CancelHandle::new();
            cancel.cancel();
            let barrier = Arc::new(Barrier::new(1));
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    play_barrier: barrier,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                },
            );

            assert!(finished.load(Ordering::Relaxed));
            // No events should be sent since we cancelled before playback
            assert!(sender.sent.lock().unwrap().is_empty());
        }

        #[test]
        fn cancel_during_delay_sets_finished() {
            let midi = make_events(&[0]);
            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(1));
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            // Use a long delay but cancel from another thread
            let cancel_clone = cancel.clone();
            let handle = thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                cancel_clone.cancel();
            });

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::from_secs(10), // Very long delay
                    cancel_handle: &cancel,
                    play_barrier: barrier,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                },
            );

            handle.join().unwrap();
            assert!(finished.load(Ordering::Relaxed));
            // Should have been cancelled during delay, no events sent
            assert!(sender.sent.lock().unwrap().is_empty());
        }

        #[test]
        fn short_delay_completes_and_plays() {
            let midi = make_events(&[0, 0, 0]);
            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(1));
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::from_millis(10), // Short delay
                    cancel_handle: &cancel,
                    play_barrier: barrier,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                },
            );

            assert!(finished.load(Ordering::Relaxed));
            assert_eq!(sender.sent.lock().unwrap().len(), 3);
        }

        #[test]
        fn respects_exclude_channels() {
            use crate::midi::playback::TimedMidiEvent;
            let events = vec![
                TimedMidiEvent {
                    time: Duration::ZERO,
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(60),
                        vel: u7::new(100),
                    },
                },
                TimedMidiEvent {
                    time: Duration::ZERO,
                    channel: 9, // Excluded
                    message: midly::MidiMessage::NoteOn {
                        key: u7::new(62),
                        vel: u7::new(100),
                    },
                },
            ];
            let midi = PrecomputedMidi::from_events(events);
            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(1));
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::from([9]);
            let mut sender = MockSender::new();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    play_barrier: barrier,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                },
            );

            assert_eq!(sender.sent.lock().unwrap().len(), 1);
        }

        #[test]
        fn barrier_synchronization_works() {
            // Barrier with 2 parties — run_playback in a thread, release from main
            let midi = make_events(&[0]);
            let cancel = CancelHandle::new();
            let barrier = Arc::new(Barrier::new(2));
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();

            let barrier_clone = barrier.clone();
            let finished_clone = finished.clone();
            let cancel_clone = cancel.clone();

            let handle = thread::spawn(move || {
                run_playback(
                    &mut sender,
                    PlaybackContext {
                        precomputed: &midi,
                        start_time: Duration::ZERO,
                        playback_delay: Duration::ZERO,
                        cancel_handle: &cancel_clone,
                        play_barrier: barrier_clone,
                        finished: finished_clone,
                        exclude_channels: &exclude,
                    },
                );
                sender
            });

            // Small delay to ensure the thread reaches the barrier
            thread::sleep(Duration::from_millis(10));
            assert!(!finished.load(Ordering::Relaxed));

            // Release the barrier
            barrier.wait();

            let sender = handle.join().unwrap();
            assert!(finished.load(Ordering::Relaxed));
            assert_eq!(sender.sent.lock().unwrap().len(), 1);
        }
    }
}
