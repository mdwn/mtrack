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
use std::{
    cmp::min,
    collections::{HashMap, HashSet},
    error::Error,
    fmt, mem,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Barrier, Mutex,
    },
    thread,
    time::Duration,
};

use midir::{MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputPort};
use midly::live::LiveEvent;
use nodi::{Connection, Player, Timer};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info, span, warn, Level};

use crate::{
    config,
    dmx::{self, engine::Engine},
    playsync::CancelHandle,
    songs::Song,
};

use super::transform::{ControlChangeMapper, MidiTransformer, NoteMapper};

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

/// This is the maximum amount of ticks that the MIDI player can sleep for
/// before checking whether a thread is cancelled. Lowering this may result
/// in more frequent CPU spinning.
const MAX_TICK_SIZE_FOR_SLEEP: u32 = 200;

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
                            if let Ok(event) = LiveEvent::parse(&event) {
                                // Take the MIDI and pass through to the DMX engine if the DMX engine is present.
                                if let LiveEvent::Midi { channel, message } = event {
                                    if let Some(universe) =
                                        midi_to_dmx_mappings.get(&channel.as_int())
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

    /// Plays the given song through the MIDI interface.
    fn play(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
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
            "Playing song MIDI."
        );

        let finished = Arc::new(AtomicBool::new(false));
        let playback_delay = self.playback_delay;
        let join_handle = {
            let cancel_handle = cancel_handle.clone();
            let finished = finished.clone();

            // Wrap the midir connection in a cancel connection so that we can stop playback.
            let midir_connection = output.connect(output_port, "mtrack player")?;
            let connection = ExcludeConnection {
                connection: midir_connection,
                cancel_handle: cancel_handle.clone(),
                exclude_midi_channels,
            };
            let mut player = Player::new(
                CancelableTimer::new(midi_sheet.ticker, cancel_handle.clone()),
                connection,
            );

            thread::spawn(move || {
                play_barrier.wait();
                spin_sleep::sleep(playback_delay);
                player.play(&midi_sheet.sheet);
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

        // Choosing 8 here because that's what nodi does.
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
        if !devices.contains_key(&name) {
            devices.insert(
                name.clone(),
                Device {
                    name: name.clone(),
                    playback_delay: Duration::ZERO,
                    input_port: Some(port),
                    output_port: None,
                    event_connection: Box::new(Mutex::new(None)),
                    midi_to_dmx_mappings: HashMap::new(),
                    dmx_engine: None,
                    dmx_midi_transformers: HashMap::new(),
                },
            );
        }
    }

    for port in output_ports {
        let name = output.port_name(&port)?;
        match devices.get_mut(&name) {
            Some(device) => {
                device.output_port = Some(port);
            }
            None => {
                devices.insert(
                    name.clone(),
                    Device {
                        name: name.clone(),
                        playback_delay: Duration::ZERO,
                        input_port: None,
                        output_port: Some(port),
                        event_connection: Box::new(Mutex::new(None)),
                        midi_to_dmx_mappings: HashMap::new(),
                        dmx_engine: None,
                        dmx_midi_transformers: HashMap::new(),
                    },
                );
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

    let mut midi_to_dmx_mapping = HashMap::new();
    let mut dmx_midi_transformers: HashMap<u8, Vec<MidiTransformer>> = HashMap::new();
    for midi_to_dmx in config.midi_to_dmx() {
        let midi_channel = midi_to_dmx.midi_channel()?.as_int();
        midi_to_dmx_mapping.insert(midi_channel, midi_to_dmx.universe());

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

        if !dmx_midi_transformers.contains_key(&midi_channel) {
            dmx_midi_transformers.insert(midi_channel, Vec::new());
        }

        dmx_midi_transformers
            .get_mut(&midi_channel)
            .map(|current_transformers| current_transformers.extend(transformers));
    }

    let mut midi_device = matches.swap_remove(0);
    midi_device.playback_delay = playback_delay;
    midi_device.midi_to_dmx_mappings = midi_to_dmx_mapping;
    midi_device.dmx_engine = dmx_engine;
    midi_device.dmx_midi_transformers = dmx_midi_transformers;

    // We've verified that there's only one element in the vector, so this should be safe.
    Ok(midi_device)
}

/// CancelableTimer is a timer for the nodi player that allows cancelation.
pub(crate) struct CancelableTimer<T: Timer> {
    timer: T,
    cancel_handle: CancelHandle,
}

impl<T: Timer> CancelableTimer<T> {
    pub fn new(timer: T, cancel_handle: CancelHandle) -> CancelableTimer<T> {
        CancelableTimer {
            timer,
            cancel_handle,
        }
    }
}

impl<T: Timer> Timer for CancelableTimer<T> {
    fn sleep_duration(&mut self, n_ticks: u32) -> std::time::Duration {
        self.timer.sleep_duration(n_ticks)
    }

    fn change_tempo(&mut self, tempo: u32) {
        self.timer.change_tempo(tempo);
    }

    fn sleep(&mut self, n_ticks: u32) {
        // Sleep in chunks of MAX_TICK_SIZE_FOR_SLEEP or less.
        let mut remaining_ticks = n_ticks;
        loop {
            let num_ticks = min(remaining_ticks, MAX_TICK_SIZE_FOR_SLEEP);
            self.timer.sleep(num_ticks);
            if remaining_ticks == num_ticks {
                return;
            }
            remaining_ticks -= MAX_TICK_SIZE_FOR_SLEEP;

            // Make sure we react to cancellation.
            if self.cancel_handle.is_cancelled() {
                return;
            }
        }
    }
}

/// ExcludeConnection is a nodi connection that can be cancelled and will exclude the given MIDI channels..
struct ExcludeConnection<C: Connection> {
    connection: C,
    cancel_handle: CancelHandle,
    exclude_midi_channels: HashSet<u8>,
}

impl<C: Connection> Connection for ExcludeConnection<C> {
    fn play(&mut self, event: nodi::MidiEvent) -> bool {
        if self.cancel_handle.is_cancelled() {
            return false;
        };

        if self.exclude_midi_channels.is_empty()
            || !self.exclude_midi_channels.contains(&event.channel.as_int())
        {
            self.connection.play(event)
        } else {
            true
        }
    }
}
