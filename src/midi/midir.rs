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
    clock::PlaybackClock,
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
    beat_clock_enabled: bool,
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
            beat_clock_enabled: false,
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
        ready_tx: std::sync::mpsc::Sender<()>,
        start_time: Duration,
        clock: PlaybackClock,
        loop_break: Arc<AtomicBool>,
        active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
        section_loop_break: Arc<AtomicBool>,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (midir)");
        let _enter = span.enter();

        let midi_playback = match song.midi_playback() {
            Some(midi_playback) => midi_playback,
            None => {
                info!(song = song.name(), "Song has no MIDI sheet.");
                let _ = ready_tx.send(());
                return Ok(());
            }
        };

        let output_port = match self.output_port.as_ref() {
            Some(output_port) => output_port,
            None => {
                warn!(
                    song = song.name(),
                    "No MIDI output device configured, cannot play song."
                );
                let _ = ready_tx.send(());
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
            beat_clock = self.beat_clock_enabled,
            "Playing song MIDI."
        );

        let finished = Arc::new(AtomicBool::new(false));
        let playback_delay = self.playback_delay;
        let mut connection = output.connect(output_port, "mtrack player")?;

        // Optionally set up beat clock on a second connection
        let beat_clock_handle = if self.beat_clock_enabled {
            if let Some(ref beat_clock) = midi_sheet.beat_clock {
                let clock_output = MidiOutput::new("mtrack beat clock output")?;
                let mut clock_connection =
                    clock_output.connect(output_port, "mtrack beat clock")?;
                let clock_cancel = cancel_handle.clone();
                let clock_playback_delay = playback_delay;
                let clock_start_time = start_time;
                // Use a dedicated wall clock for beat clock timing instead of
                // the audio-derived PlaybackClock. The audio clock only updates
                // once per buffer callback (~23ms at 1024/44.1kHz), causing the
                // beat clock's spin_sleep to overshoot by variable amounts.
                // A wall clock provides smooth, sub-millisecond precision.
                let clock_clock = PlaybackClock::wall();

                // Build a small wrapper that owns the PrecomputedBeatClock data we need.
                // We can't move midi_sheet into the closure since the note thread uses it,
                // so re-extract the tick data.
                let ticks: Vec<Duration> = beat_clock.ticks_from(Duration::ZERO).to_vec();

                // Internal barrier to synchronize the beat clock thread with the note
                // playback thread. The note thread will wait on this after the external
                // play_barrier releases, so both start sending at the same time.
                let internal_barrier = Arc::new(Barrier::new(2));
                let clock_internal_barrier = internal_barrier.clone();

                info!("Starting MIDI beat clock thread.");
                Some((
                    thread::spawn(move || {
                        // Elevate to real-time thread priority to minimize
                        // scheduling jitter on clock tick delivery.
                        promote_to_realtime_thread();

                        // Wait for the note thread to signal us after it passes the
                        // external play_barrier.
                        clock_internal_barrier.wait();

                        if clock_cancel.is_cancelled() {
                            return;
                        }

                        // Sleep the playback delay using a simple spin_sleep.
                        if !clock_playback_delay.is_zero() {
                            spin_sleep::sleep(clock_playback_delay);
                        }

                        if clock_cancel.is_cancelled() {
                            return;
                        }

                        // Start the wall clock right before we begin sending.
                        clock_clock.start();

                        run_beat_clock(
                            &mut clock_connection,
                            &ticks,
                            clock_start_time,
                            &clock_cancel,
                            &clock_clock,
                        );
                    }),
                    internal_barrier,
                ))
            } else {
                None
            }
        } else {
            None
        };

        // Extract the internal barrier (if any) for synchronizing the beat clock thread.
        let (beat_clock_join, beat_clock_internal_barrier) = match beat_clock_handle {
            Some((handle, barrier)) => (Some(handle), Some(barrier)),
            None => (None, None),
        };

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
                        ready_tx,
                        finished,
                        exclude_channels: &exclude_midi_channels,
                        beat_clock_barrier: beat_clock_internal_barrier,
                        clock: &clock,
                        loop_playback: song.loop_playback(),
                        loop_break: loop_break.clone(),
                        active_section: active_section.clone(),
                        section_loop_break: section_loop_break.clone(),
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

        if let Some(handle) = beat_clock_join {
            if handle.join().is_err() {
                return Err("Error while joining beat clock thread!".into());
            }
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

    fn emit_sysex(&self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "emit_sysex (midir)");
        let _enter = span.enter();

        let output_port = match &self.output_port {
            Some(output_port) => output_port,
            None => {
                warn!("No MIDI output device configured, cannot emit SysEx.");
                return Ok(());
            }
        };

        let output = MidiOutput::new("mtrack emit sysex output")?;

        debug!(device = self.name, len = bytes.len(), "Emitting SysEx.");

        let mut connection = output.connect(output_port, "mtrack player")?;
        connection.send(bytes)?;

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

/// Serializable info about a MIDI device for the web UI.
#[derive(serde::Serialize)]
pub struct MidiDeviceInfo {
    pub name: String,
    pub has_input: bool,
    pub has_output: bool,
}

/// Lists MIDI devices as simple info structs (no trait objects).
pub fn list_device_info() -> Result<Vec<MidiDeviceInfo>, Box<dyn Error>> {
    let input = MidiInput::new("mtrack input listing")?;
    let output = MidiOutput::new("mtrack output listing")?;
    let input_ports = input.ports();
    let output_ports = output.ports();

    let mut devices: HashMap<String, (bool, bool)> = HashMap::new();

    for port in input_ports {
        let name = input.port_name(&port)?;
        devices.entry(name).or_insert((false, false)).0 = true;
    }
    for port in output_ports {
        let name = output.port_name(&port)?;
        devices.entry(name).or_insert((false, false)).1 = true;
    }

    let mut infos: Vec<MidiDeviceInfo> = devices
        .into_iter()
        .map(|(name, (has_input, has_output))| MidiDeviceInfo {
            name,
            has_input,
            has_output,
        })
        .collect();
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
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
    midi_device.beat_clock_enabled = config.beat_clock();
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
    ready_tx: std::sync::mpsc::Sender<()>,
    finished: Arc<AtomicBool>,
    exclude_channels: &'a HashSet<u8>,
    beat_clock_barrier: Option<Arc<Barrier>>,
    clock: &'a PlaybackClock,
    loop_playback: bool,
    loop_break: Arc<AtomicBool>,
    active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    section_loop_break: Arc<AtomicBool>,
}

/// Runs the MIDI playback thread body: signals readiness, waits for the clock
/// to start, sleeps through the playback delay (checking for cancellation),
/// then plays events.
fn run_playback(sender: &mut dyn MidiSender, ctx: PlaybackContext<'_>) {
    let _ = ctx.ready_tx.send(());

    // Wait for the clock to start (the "go" signal from play_files).
    while ctx.clock.elapsed() == Duration::ZERO {
        if ctx.cancel_handle.is_cancelled() {
            ctx.finished.store(true, Ordering::Relaxed);
            ctx.cancel_handle.notify();
            return;
        }
        std::hint::spin_loop();
    }

    // Signal the beat clock thread (if any) to start in sync with us.
    if let Some(ref barrier) = ctx.beat_clock_barrier {
        barrier.wait();
    }

    if ctx.cancel_handle.is_cancelled() {
        ctx.finished.store(true, Ordering::Relaxed);
        ctx.cancel_handle.notify();
        return;
    }

    // Sleep the playback delay in small increments so we can
    // respond to cancellation promptly.
    {
        while ctx.clock.elapsed() < ctx.playback_delay {
            if ctx.cancel_handle.is_cancelled() {
                ctx.finished.store(true, Ordering::Relaxed);
                ctx.cancel_handle.notify();
                return;
            }
            let remaining = ctx.playback_delay.saturating_sub(ctx.clock.elapsed());
            spin_sleep::sleep(remaining.min(Duration::from_millis(50)));
        }
    }

    play_precomputed(
        &MidiPlaybackParams {
            precomputed: ctx.precomputed,
            start_time: ctx.start_time,
            end_time: None,
            clock_base: ctx.clock.elapsed(),
            cancel_handle: ctx.cancel_handle,
            exclude_channels: ctx.exclude_channels,
            clock: ctx.clock,
            active_section: Some(&ctx.active_section),
        },
        sender,
    );

    // Section loop: if active_section is set, loop the section region.
    // The initial play_precomputed exits when it reaches section.end_time
    // (via the active_section check). This loop then replays the section.
    let mut section_trigger = crate::section_loop::SectionLoopTrigger::new();

    loop {
        if ctx.cancel_handle.is_cancelled() || ctx.loop_break.load(Ordering::Relaxed) {
            break;
        }

        // Check for section loop.
        let section = ctx.active_section.read().clone();
        if let Some(ref section) = section {
            if !ctx.section_loop_break.load(Ordering::Relaxed) {
                let crossfade_margin = crate::audio::crossfade::DEFAULT_CROSSFADE_DURATION;
                let elapsed = ctx.clock.elapsed();

                if let Some(_trigger_time) =
                    section_trigger.check(section, elapsed, crossfade_margin)
                {
                    if ctx.cancel_handle.is_cancelled() {
                        break;
                    }

                    if !ctx.section_loop_break.load(Ordering::Relaxed) {
                        info!(section = section.name, "MIDI section loop: restarting");
                        play_precomputed(
                            &MidiPlaybackParams {
                                precomputed: ctx.precomputed,
                                start_time: section.start_time,
                                end_time: Some(section.end_time),
                                clock_base: ctx.clock.elapsed(),
                                cancel_handle: ctx.cancel_handle,
                                exclude_channels: ctx.exclude_channels,
                                clock: ctx.clock,
                                active_section: None,
                            },
                            sender,
                        );
                        continue;
                    }
                }
            }
        } else {
            section_trigger.reset();
        }

        // Whole-song loop check.
        if ctx.loop_playback
            && !ctx.cancel_handle.is_cancelled()
            && !ctx.loop_break.load(Ordering::Relaxed)
        {
            info!("MIDI loop: restarting from beginning");
            play_precomputed(
                &MidiPlaybackParams {
                    precomputed: ctx.precomputed,
                    start_time: Duration::ZERO,
                    end_time: None,
                    clock_base: ctx.clock.elapsed(),
                    cancel_handle: ctx.cancel_handle,
                    exclude_channels: ctx.exclude_channels,
                    clock: ctx.clock,
                    active_section: Some(&ctx.active_section),
                },
                sender,
            );
            continue;
        }

        break;
    }

    ctx.finished.store(true, Ordering::Relaxed);
    ctx.cancel_handle.notify();
}

/// Serializes a MIDI System Real-Time event to bytes.
fn realtime_bytes(msg: midly::live::SystemRealtime) -> Vec<u8> {
    let event = LiveEvent::Realtime(msg);
    let mut buf = Vec::with_capacity(1);
    event
        .write_std(&mut buf)
        .expect("realtime events are always valid");
    buf
}

/// Runs the beat clock on a MIDI sender, sending START/CONTINUE, timing clocks, and STOP.
fn run_beat_clock(
    sender: &mut dyn MidiSender,
    ticks: &[Duration],
    start_time: Duration,
    cancel_handle: &CancelHandle,
    clock: &PlaybackClock,
) {
    use midly::live::SystemRealtime;

    // Send START or CONTINUE
    let start_msg = if start_time == Duration::ZERO {
        realtime_bytes(SystemRealtime::Start)
    } else {
        realtime_bytes(SystemRealtime::Continue)
    };
    if let Err(e) = sender.send(&start_msg) {
        debug!("MIDI beat clock start send failed: {:?}", e);
    }

    // Find ticks from start_time
    let idx = ticks.partition_point(|t| *t < start_time);
    let remaining_ticks = &ticks[idx..];

    let clock_bytes = realtime_bytes(SystemRealtime::TimingClock);
    let stop_bytes = realtime_bytes(SystemRealtime::Stop);

    for tick_time in remaining_ticks {
        if cancel_handle.is_cancelled() {
            let _ = sender.send(&stop_bytes);
            return;
        }

        let target_wall = *tick_time - start_time;
        let elapsed = clock.elapsed();
        if target_wall > elapsed {
            spin_sleep::sleep(target_wall - elapsed);
        }

        if cancel_handle.is_cancelled() {
            let _ = sender.send(&stop_bytes);
            return;
        }

        if let Err(e) = sender.send(&clock_bytes) {
            debug!("MIDI beat clock send failed: {:?}", e);
        }
    }

    // Send STOP when finished
    let _ = sender.send(&stop_bytes);
}

/// Parameters for playing pre-computed MIDI events.
struct MidiPlaybackParams<'a> {
    precomputed: &'a super::playback::PrecomputedMidi,
    /// Start time in the event timeline to begin playback from.
    start_time: Duration,
    /// If set, stop playback when events reach this time.
    end_time: Option<Duration>,
    /// What the clock reads at the logical start of this playback iteration.
    /// Events are timed relative to this so looped iterations work correctly
    /// even though the clock keeps advancing.
    clock_base: Duration,
    cancel_handle: &'a CancelHandle,
    exclude_channels: &'a HashSet<u8>,
    clock: &'a PlaybackClock,
    /// If set, stop playback when this section becomes active and event time
    /// reaches the section end. Allows the section loop to take over.
    active_section: Option<&'a Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>>,
}

/// Plays pre-computed MIDI events through a MIDI sender.
/// Sleeps between events using spin_sleep for precision without busy-waiting.
fn play_precomputed(params: &MidiPlaybackParams<'_>, sender: &mut dyn MidiSender) {
    let events = params.precomputed.events_from(params.start_time);
    let mut buf = Vec::with_capacity(8);

    for event in events {
        if params.cancel_handle.is_cancelled() {
            return;
        }

        // Stop at section end time if specified.
        if let Some(end) = params.end_time {
            if event.time >= end {
                return;
            }
        }

        // Stop if a section loop became active and we've reached its end.
        // This lets the section loop in run_playback take over.
        if let Some(active) = params.active_section {
            if let Some(ref section) = *active.read() {
                if event.time >= section.end_time {
                    return;
                }
            }
        }

        // target_wall is relative to the section start, then offset by
        // clock_base so it aligns with the continuously-advancing clock.
        let target_wall = params.clock_base + (event.time - params.start_time);
        let elapsed = params.clock.elapsed();
        if target_wall > elapsed {
            spin_sleep::sleep(target_wall - elapsed);
        }
        if params.cancel_handle.is_cancelled() {
            return;
        }

        if let Some(bytes) = serialize_midi_event(event, params.exclude_channels, &mut buf) {
            if let Err(e) = sender.send(&bytes) {
                debug!("MIDI send failed: {:?}", e);
            }
        }
    }
}

/// Promotes the current thread to real-time priority for low-jitter MIDI clock output.
///
/// Uses the shared thread priority utility that sets a high crossplatform priority
/// and, on Unix systems (Linux and macOS), requests SCHED_FIFO real-time scheduling.
/// Failures are logged but not fatal — the beat clock will still work, just with
/// potentially more jitter from OS thread scheduling.
fn promote_to_realtime_thread() {
    use crate::thread_priority::{callback_thread_priority, promote_to_realtime, rt_audio_enabled};

    let priority = callback_thread_priority();
    let rt_enabled = rt_audio_enabled();
    let mut priority_set = false;

    promote_to_realtime(priority, rt_enabled, &mut priority_set);

    if priority_set {
        info!("Elevated MIDI beat clock thread priority");
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
            assert!(!device.beat_clock_enabled);
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
            let config: config::Midi = ::config::Config::builder()
                .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
            let (mappings, transformers) = build_transformers(&config).unwrap();

            // MIDI channel 10 → internal channel 9 (1-indexed to 0-indexed in config)
            assert_eq!(mappings.get(&9), Some(&"main".to_string()));
            // No transformers configured.
            assert!(transformers.get(&9).is_none_or(|t| t.is_empty()));
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
            let config: config::Midi = ::config::Config::builder()
                .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
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
            let config: config::Midi = ::config::Config::builder()
                .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
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
            let config: config::Midi = ::config::Config::builder()
                .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
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
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = PlaybackClock::wall();
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                cancel,
                ready_tx,
                Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
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
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = PlaybackClock::wall();
            // The no-MIDI-sheet check happens before the output-port check
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                cancel,
                ready_tx,
                Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
        }

        #[test]
        fn play_from_without_midi_playback_sends_ready() {
            // When a MIDI device exists but the song has no MIDI sheet,
            // play_from must still send the ready signal so the playback
            // clock starts. Without this, all subsystems (audio, DMX)
            // spin forever waiting for the clock.
            let device = Device::new_default("test".to_string());
            let (_tmp_dir, song) = make_song();
            assert!(song.midi_playback().is_none());

            let cancel = CancelHandle::new();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = PlaybackClock::wall();
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                cancel,
                ready_tx,
                Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
            assert!(
                ready_rx.try_recv().is_ok(),
                "play_from must send ready signal even when the song has no MIDI playback"
            );
        }
    }

    mod play_precomputed_tests {
        use super::*;
        use crate::midi::playback::{PrecomputedMidi, TimedMidiEvent};
        use crate::playsync::CancelHandle;
        struct MockSender {
            sent: parking_lot::Mutex<Vec<Vec<u8>>>,
            should_fail: bool,
        }

        impl MockSender {
            fn new() -> Self {
                MockSender {
                    sent: parking_lot::Mutex::new(Vec::new()),
                    should_fail: false,
                }
            }

            fn failing() -> Self {
                MockSender {
                    sent: parking_lot::Mutex::new(Vec::new()),
                    should_fail: true,
                }
            }
        }

        impl MidiSender for MockSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if self.should_fail {
                    return Err("mock send failure".into());
                }
                self.sent.lock().push(bytes.to_vec());
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

        /// Helper: run play_precomputed with common defaults for tests.
        fn run_play(
            midi: &PrecomputedMidi,
            start: Duration,
            sender: &mut MockSender,
            cancel: &CancelHandle,
            exclude: &HashSet<u8>,
            clock: &PlaybackClock,
        ) {
            play_precomputed(
                &MidiPlaybackParams {
                    precomputed: midi,
                    start_time: start,
                    end_time: None,
                    clock_base: clock.elapsed(),
                    cancel_handle: cancel,
                    exclude_channels: exclude,
                    clock,
                    active_section: None,
                },
                sender,
            );
        }

        #[test]
        fn plays_all_events() {
            let midi = make_events(&[0, 0, 0]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_play(
                &midi,
                Duration::ZERO,
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );

            let sent = sender.sent.lock();
            assert_eq!(sent.len(), 3);
        }

        #[test]
        fn respects_start_time() {
            // Events at 0ms, 100ms, 200ms. Start from 100ms → skip first event.
            let midi = make_events(&[0, 100, 200]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_play(
                &midi,
                Duration::from_millis(100),
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );

            let sent = sender.sent.lock();
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
            let clock = PlaybackClock::wall();

            run_play(
                &midi,
                Duration::ZERO,
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );

            let sent = sender.sent.lock();
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

            let clock = PlaybackClock::wall();
            run_play(
                &midi,
                Duration::ZERO,
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );

            let sent = sender.sent.lock();
            assert_eq!(sent.len(), 0);
        }

        #[test]
        fn empty_events() {
            let midi = PrecomputedMidi::from_events(Vec::new());
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_play(
                &midi,
                Duration::ZERO,
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );

            let sent = sender.sent.lock();
            assert!(sent.is_empty());
        }

        #[test]
        fn send_failure_continues() {
            let midi = make_events(&[0, 0, 0]);
            let cancel = CancelHandle::new();
            let exclude = HashSet::new();
            let mut sender = MockSender::failing();
            let clock = PlaybackClock::wall();

            // Should not panic — errors are logged but playback continues
            run_play(
                &midi,
                Duration::ZERO,
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );
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
            let clock = PlaybackClock::wall();

            run_play(
                &midi,
                Duration::ZERO,
                &mut sender,
                &cancel,
                &exclude,
                &clock,
            );

            let sent = sender.sent.lock();
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
        struct MockSender {
            sent: parking_lot::Mutex<Vec<Vec<u8>>>,
        }

        impl MockSender {
            fn new() -> Self {
                MockSender {
                    sent: parking_lot::Mutex::new(Vec::new()),
                }
            }
        }

        impl MidiSender for MockSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.sent.lock().push(bytes.to_vec());
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
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();
            clock.start();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    beat_clock_barrier: None,
                    clock: &clock,
                    loop_playback: false,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_loop_break: Arc::new(AtomicBool::new(false)),
                },
            );

            assert!(finished.load(Ordering::Relaxed));
            assert_eq!(sender.sent.lock().len(), 2);
        }

        #[test]
        fn cancel_before_barrier_sets_finished() {
            let midi = make_events(&[0]);
            let cancel = CancelHandle::new();
            cancel.cancel();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();
            clock.start();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    beat_clock_barrier: None,
                    clock: &clock,
                    loop_playback: false,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_loop_break: Arc::new(AtomicBool::new(false)),
                },
            );

            assert!(finished.load(Ordering::Relaxed));
            // No events should be sent since we cancelled before playback
            assert!(sender.sent.lock().is_empty());
        }

        #[test]
        fn cancel_during_delay_sets_finished() {
            let midi = make_events(&[0]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();
            clock.start();

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
                    ready_tx,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    beat_clock_barrier: None,
                    clock: &clock,
                    loop_playback: false,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_loop_break: Arc::new(AtomicBool::new(false)),
                },
            );

            handle.join().unwrap();
            assert!(finished.load(Ordering::Relaxed));
            // Should have been cancelled during delay, no events sent
            assert!(sender.sent.lock().is_empty());
        }

        #[test]
        fn short_delay_completes_and_plays() {
            let midi = make_events(&[0, 0, 0]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();
            clock.start();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::from_millis(10), // Short delay
                    cancel_handle: &cancel,
                    ready_tx,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    beat_clock_barrier: None,
                    clock: &clock,
                    loop_playback: false,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_loop_break: Arc::new(AtomicBool::new(false)),
                },
            );

            assert!(finished.load(Ordering::Relaxed));
            assert_eq!(sender.sent.lock().len(), 3);
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
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::from([9]);
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();
            clock.start();

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx,
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    beat_clock_barrier: None,
                    clock: &clock,
                    loop_playback: false,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_loop_break: Arc::new(AtomicBool::new(false)),
                },
            );

            assert_eq!(sender.sent.lock().len(), 1);
        }

        #[test]
        fn ready_channel_synchronization_works() {
            // run_playback sends on ready_tx, then waits for the clock to start.
            // The test thread receives on ready_rx, then starts the clock.
            let midi = make_events(&[0]);
            let cancel = CancelHandle::new();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let finished = Arc::new(AtomicBool::new(false));
            let exclude = HashSet::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            let finished_clone = finished.clone();
            let cancel_clone = cancel.clone();
            let clock_clone = clock.clone();

            let handle = thread::spawn(move || {
                run_playback(
                    &mut sender,
                    PlaybackContext {
                        precomputed: &midi,
                        start_time: Duration::ZERO,
                        playback_delay: Duration::ZERO,
                        cancel_handle: &cancel_clone,
                        ready_tx,
                        finished: finished_clone,
                        exclude_channels: &exclude,
                        beat_clock_barrier: None,
                        clock: &clock_clone,
                        loop_playback: false,
                        loop_break: Arc::new(AtomicBool::new(false)),
                        active_section: Arc::new(parking_lot::RwLock::new(None)),
                        section_loop_break: Arc::new(AtomicBool::new(false)),
                    },
                );
                sender
            });

            // Wait for run_playback to signal readiness
            ready_rx.recv().unwrap();
            assert!(!finished.load(Ordering::Relaxed));

            // Start the clock to let playback proceed
            clock.start();

            let sender = handle.join().unwrap();
            assert!(finished.load(Ordering::Relaxed));
            assert_eq!(sender.sent.lock().len(), 1);
        }
    }

    mod run_beat_clock_tests {
        use super::*;
        use crate::playsync::CancelHandle;
        use midly::live::SystemRealtime;
        fn start_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::Start)
        }
        fn continue_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::Continue)
        }
        fn stop_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::Stop)
        }
        fn clock_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::TimingClock)
        }

        struct MockSender {
            sent: parking_lot::Mutex<Vec<Vec<u8>>>,
        }

        impl MockSender {
            fn new() -> Self {
                MockSender {
                    sent: parking_lot::Mutex::new(Vec::new()),
                }
            }
        }

        impl MidiSender for MockSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.sent.lock().push(bytes.to_vec());
                Ok(())
            }
        }

        #[test]
        fn sends_start_clocks_and_stop() {
            // 1 beat at default 120 BPM = 24 clock ticks
            let beat_clock = crate::midi::beat_clock::PrecomputedBeatClock::from_tempo_info(
                &[crate::midi::playback::TempoEntry {
                    tick: 0,
                    micros_per_tick: 500_000.0 / 480.0,
                }],
                480,
                480,
            );
            let ticks: Vec<Duration> = beat_clock.ticks_from(Duration::ZERO).to_vec();
            let cancel = CancelHandle::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_beat_clock(&mut sender, &ticks, Duration::ZERO, &cancel, &clock);

            let sent = sender.sent.lock();
            // START + 24 clock ticks + STOP
            assert_eq!(sent.len(), 26);
            assert_eq!(sent[0], start_bytes());
            for msg in &sent[1..25] {
                assert_eq!(*msg, clock_bytes());
            }
            assert_eq!(sent[25], stop_bytes());
        }

        #[test]
        fn sends_continue_when_seeking() {
            let ticks = vec![Duration::from_millis(500), Duration::from_millis(600)];
            let cancel = CancelHandle::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_beat_clock(
                &mut sender,
                &ticks,
                Duration::from_millis(100),
                &cancel,
                &clock,
            );

            let sent = sender.sent.lock();
            // CONTINUE + 2 ticks + STOP
            assert_eq!(sent.len(), 4);
            assert_eq!(sent[0], continue_bytes());
        }

        #[test]
        fn empty_ticks_sends_start_and_stop() {
            let ticks: Vec<Duration> = Vec::new();
            let cancel = CancelHandle::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_beat_clock(&mut sender, &ticks, Duration::ZERO, &cancel, &clock);

            let sent = sender.sent.lock();
            assert_eq!(sent.len(), 2);
            assert_eq!(sent[0], start_bytes());
            assert_eq!(sent[1], stop_bytes());
        }

        #[test]
        fn cancellation_sends_stop() {
            let ticks = vec![Duration::from_secs(10), Duration::from_secs(20)];
            let cancel = CancelHandle::new();
            cancel.cancel();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            run_beat_clock(&mut sender, &ticks, Duration::ZERO, &cancel, &clock);

            let sent = sender.sent.lock();
            // START + STOP (cancelled before any ticks)
            assert_eq!(sent.len(), 2);
            assert_eq!(sent[0], start_bytes());
            assert_eq!(sent[1], stop_bytes());
        }
    }
}
