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
                                // Take the MIDI and pass through to the DMX engine if the DMX engine is present.
                                if let Some(universe) = midi_to_dmx_mappings.get(&channel.as_int())
                                {
                                    let mut transformed = false;

                                    if let Some(transformers_for_channel) =
                                        dmx_midi_transformers.get(&channel.as_int())
                                    {
                                        for transformer in transformers_for_channel {
                                            if transformer.can_process(&message) {
                                                for transformed_message in
                                                    transformer.transform(&message)
                                                {
                                                    transformed = true;
                                                    dmx_engine.handle_midi_event(
                                                        universe.into(),
                                                        transformed_message,
                                                    );
                                                }
                                            }
                                        }
                                    }

                                    // Only send the original note if
                                    if !transformed {
                                        dmx_engine.handle_midi_event(universe.into(), message);
                                    }
                                }
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
                play_barrier.wait();

                if cancel_handle.is_cancelled() {
                    finished.store(true, Ordering::Relaxed);
                    cancel_handle.notify();
                    return;
                }

                // Sleep the playback delay in small increments so we can
                // respond to cancellation promptly.
                {
                    let start = std::time::Instant::now();
                    while start.elapsed() < playback_delay {
                        if cancel_handle.is_cancelled() {
                            finished.store(true, Ordering::Relaxed);
                            cancel_handle.notify();
                            return;
                        }
                        let remaining = playback_delay.saturating_sub(start.elapsed());
                        spin_sleep::sleep(remaining.min(Duration::from_millis(50)));
                    }
                }

                play_precomputed(
                    &midi_sheet.precomputed,
                    start_time,
                    &mut connection,
                    &cancel_handle,
                    &exclude_midi_channels,
                );

                finished.store(true, Ordering::Relaxed);
                cancel_handle.notify();
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

    if matches.is_empty() {
        return Err(format!("no device found with name {}", name).into());
    }
    if matches.len() > 1 {
        return Err(format!(
            "found too many devices that match ({}), use a less ambiguous device name",
            matches
                .iter()
                .map(|device| device.name.clone())
                .collect::<Vec<String>>()
                .join(", ")
        )
        .into());
    }

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

/// Plays pre-computed MIDI events through a hardware connection.
/// Sleeps between events using spin_sleep for precision without busy-waiting.
fn play_precomputed(
    precomputed: &super::playback::PrecomputedMidi,
    start_time: Duration,
    connection: &mut midir::MidiOutputConnection,
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

        if !exclude_channels.is_empty() && exclude_channels.contains(&event.channel) {
            continue;
        }
        let live_event = midly::live::LiveEvent::Midi {
            channel: event.channel.into(),
            message: event.message,
        };
        buf.clear();
        if live_event.write_std(&mut buf).is_ok() {
            if let Err(e) = connection.send(&buf) {
                debug!("MIDI send failed: {:?}", e);
            }
        }
    }
}
