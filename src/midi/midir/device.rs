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
//! The midir device: its ports, input watch and reopening, and the
//! playback of a song's MIDI on it.

pub use crate::device_info::MidiDeviceInfo;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt, mem,
    sync::{atomic::AtomicBool, Arc, Mutex},
    thread,
    time::Duration,
};

use midir::{MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputPort};
use midly::live::LiveEvent;
use tokio::sync::mpsc::{error::TrySendError, Sender};
use tracing::{debug, error, info, span, warn, Level};

use crate::{clock::PlaybackClock, config, playsync::CancelHandle, songs::Song};

use super::{
    beat_clock::{BeatClockEngine, BeatClockPlay, BeatClockSections},
    notes::{run_playback, PlaybackContext},
};

pub struct Device {
    pub(super) name: String,
    pub(super) playback_delay: Duration,
    pub(super) beat_clock_enabled: bool,
    /// What its beat clock does where a loop goes back.
    beat_clock_seams: config::BeatClockSeams,
    pub(super) input_port: Option<MidiInputPort>,
    pub(super) output_port: Option<MidiOutputPort>,
    event_connection: Box<Mutex<Option<MidiInputConnection<()>>>>,
    /// Device-owned, always-on beat clock. Present when `beat_clock_enabled` is
    /// set and the device has an output port. Songs retune it by submitting their
    /// tick schedule; between songs it either free-runs the last tempo (when
    /// `persist_tempo` is set) or stays silent.
    pub(super) beat_clock_engine: Option<Arc<BeatClockEngine>>,
    /// For a test: refuses to watch events, as a device whose input
    /// cannot be connected would.
    #[cfg(test)]
    pub(super) refuse_watch: bool,
    /// For a test: run when the device is dropped.
    #[cfg(test)]
    pub(super) on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl Device {
    pub(super) fn new_default(name: String) -> Self {
        Device {
            name,
            playback_delay: Duration::ZERO,
            beat_clock_enabled: false,
            beat_clock_seams: config::BeatClockSeams::default(),
            input_port: None,
            output_port: None,
            event_connection: Box::new(Mutex::new(None)),
            beat_clock_engine: None,
            #[cfg(test)]
            refuse_watch: false,
            #[cfg(test)]
            on_drop: None,
        }
    }

    /// The tempo its beat clock holds between songs, if it holds one.
    pub(super) fn held_tempo(&self) -> Option<Duration> {
        self.beat_clock_engine.as_ref()?.held_tempo()
    }

    /// Stops its beat clock and waits for it: a device about to be replaced
    /// goes quiet before its successor's clock starts, so the port never
    /// carries two clocks at once, and a schedule it was running has set
    /// the tempo it holds by then.
    fn stop_beat_clock(&self) {
        if let Some(engine) = &self.beat_clock_engine {
            engine.stop();
        }
    }
}

#[cfg(test)]
impl Drop for Device {
    fn drop(&mut self) {
        if let Some(on_drop) = self.on_drop.take() {
            on_drop();
        }
    }
}

impl Device {
    /// Hands this song's beat clock schedule to the always-on device engine,
    /// if there is one and the song has a tempo; whether it did. The engine (not a per-song
    /// thread) owns the clock output: it syncs to the shared playback clock's
    /// "go" signal, plays this schedule, and then holds the last tempo until
    /// the next song retunes it.
    #[allow(clippy::too_many_arguments)]
    fn play_beat_clock(
        &self,
        song: &Song,
        sheet: Option<&crate::songs::MidiSheet>,
        start_time: Duration,
        cancel: &CancelHandle,
        clock: &PlaybackClock,
        sections: &BeatClockSections,
        transport: Option<Arc<AtomicBool>>,
    ) -> bool {
        let Some(engine) = &self.beat_clock_engine else {
            return false;
        };
        let Some(ticks) = song.beat_clock(sheet) else {
            return false;
        };
        // A looping song's passes are jumps in the playback's log, as its
        // sections' are: the clock follows them all from there.
        engine.play(BeatClockPlay {
            ticks: Arc::new(ticks),
            start_time,
            playback_delay: self.playback_delay,
            cancel: cancel.clone(),
            clock: clock.clone(),
            sections: Some(sections.clone()),
            transport,
        });
        true
    }
}

impl crate::midi::Device for Device {
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "wait for event (midir)");
        let _enter = span.enter();

        let mut event_connection = self.event_connection.lock().expect("unable to get lock");
        if event_connection.is_some() {
            return Err("Already watching events.".into());
        }
        #[cfg(test)]
        if self.refuse_watch {
            return Err("refused to watch events".into());
        }

        info!("Watching MIDI events.");

        let input_port = match self.input_port.as_ref() {
            Some(input_port) => input_port,
            None => {
                warn!("No MIDI output device configured, cannot listen for events.");
                return Ok(());
            }
        };

        let input = MidiInput::new("mtrack player input")?;
        *event_connection = Some(input.connect(
            input_port,
            "mtrack input watcher",
            move |_, raw_event, _| {
                // This closure runs on the MIDI driver's input thread: it must
                // never block. If the consumer stalls (it awaits player calls),
                // a blocking send here would wedge the driver thread and with
                // it all incoming MIDI. Dropping an event under backpressure is
                // the lesser harm.
                match sender.try_send(Vec::from(raw_event)) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        warn!("MIDI control channel full; dropping incoming event");
                    }
                    Err(e @ TrySendError::Closed(_)) => {
                        error!(
                            err = format!("{:?}", e),
                            "Error sending MIDI event to receiver."
                        );
                    }
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

    /// Opened once, by [`get`]: [`Reopenable`] opens one afresh.
    fn reopen(&self) -> Result<(), Box<dyn Error>> {
        Err("a midir device cannot be reopened on its own".into())
    }

    /// Plays the given song through the MIDI interface, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        sync: crate::playsync::PlaybackSync,
    ) -> Result<(), Box<dyn Error>> {
        let crate::playsync::PlaybackSync {
            cancel_handle,
            mut ready_tx,
            clock,
            start_time,
            loop_control,
        } = sync;
        let crate::playsync::LoopControl {
            loop_break,
            active_section,
            section_loop_break,
            section_jumps,
            ..
        } = loop_control;
        let sections = BeatClockSections {
            active_section: active_section.clone(),
            section_loop_break: section_loop_break.clone(),
            jumps: section_jumps.clone(),
            loop_break: loop_break.clone(),
            loops: song.loop_playback(),
            seams: self.beat_clock_seams,
        };
        let span = span!(Level::INFO, "play song (midir)");
        let _enter = span.enter();

        // A song's tempo drives the beat clock without a MIDI file.
        let Some(midi_playback) = song.midi_playback() else {
            // A song's tempo drives the beat clock without a MIDI file.
            if self.beat_clock_engine.is_some() && song.has_tempo() {
                if self.play_beat_clock(
                    &song,
                    None,
                    start_time,
                    &cancel_handle,
                    &clock,
                    &sections,
                    None,
                ) {
                    info!(
                        song = song.name(),
                        "Song has no MIDI file; its beat clock runs from its tempo."
                    );
                }
            } else {
                info!(song = song.name(), "Song has no MIDI sheet.");
            }
            // Guard sends on drop, but explicit send is clearer.
            ready_tx.send();
            return Ok(());
        };

        let output_port = match self.output_port.as_ref() {
            Some(output_port) => output_port,
            None => {
                warn!(
                    song = song.name(),
                    "No MIDI output device configured, cannot play song."
                );
                ready_tx.send();
                return Ok(());
            }
        };
        let midi_sheet = midi_playback.midi_sheet()?;
        let playback_delay = self.playback_delay;
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
        let mut connection = output.connect(output_port, "mtrack player")?;
        // Once the notes can play: a song that fails here has no clock either.
        // Set once the beat clock has sent Start (or Continue): the notes
        // due with it follow it.
        let transport = Arc::new(AtomicBool::new(false));
        let clocked = self.play_beat_clock(
            &song,
            Some(&midi_sheet),
            start_time,
            &cancel_handle,
            &clock,
            &sections,
            Some(transport.clone()),
        );
        let transport = clocked.then_some(transport);

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
                        clock: &clock,
                        loop_break: loop_break.clone(),
                        active_section: active_section.clone(),
                        section_jumps: section_jumps.clone(),
                        transport,
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
    fn to_mock(&self) -> Result<Arc<crate::midi::mock::Device>, Box<dyn Error>> {
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

/// Lists MIDI devices as simple info structs (no trait objects).
pub fn list_device_info() -> Result<Vec<MidiDeviceInfo>, Box<dyn Error>> {
    Ok(list_midir_devices()?
        .into_iter()
        .map(|device| MidiDeviceInfo {
            has_input: device.input_port.is_some(),
            has_output: device.output_port.is_some(),
            name: device.name.clone(),
        })
        .collect())
}

/// Lists midir devices and produces the Device trait.
pub fn list() -> Result<Vec<Box<dyn crate::midi::Device>>, Box<dyn Error>> {
    Ok(list_midir_devices()?
        .into_iter()
        .map(|device| {
            let device: Box<dyn crate::midi::Device> = Box::new(device);
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
pub(super) fn validate_device_match<T: fmt::Display>(
    name: &str,
    matches: &[T],
) -> Result<(), Box<dyn Error>> {
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

/// Gets the given midir device, its beat clock holding `held` until a song
/// sets another tempo.
fn get_holding(config: &config::Midi, held: Option<Duration>) -> Result<Device, Box<dyn Error>> {
    let playback_delay = config.playback_delay()?;
    let name = config.device();
    let mut matches = list_midir_devices()?
        .into_iter()
        .filter(|device| device.name.contains(name))
        .collect::<Vec<Device>>();

    validate_device_match(name, &matches)?;

    let mut midi_device = matches.swap_remove(0);
    midi_device.playback_delay = playback_delay;
    midi_device.beat_clock_enabled = config.beat_clock();
    midi_device.beat_clock_seams = config.beat_clock_seams();

    // Spawn the always-on beat clock engine when the beat clock is enabled. Songs
    // retune it as they play; `persist_tempo` controls whether it free-runs the
    // last tempo or stays silent between songs.
    if midi_device.beat_clock_enabled {
        if let Some(ref output_port) = midi_device.output_port {
            midi_device.beat_clock_engine = Some(Arc::new(BeatClockEngine::new(
                output_port.clone(),
                config.persist_tempo(),
                held,
            )));
        } else {
            warn!("beat_clock is enabled but the MIDI device has no output port; ignoring.");
        }
    }

    // We've verified that there's only one element in the vector, so this should be safe.
    Ok(midi_device)
}

/// A midir device as the player holds it, which a hardware reload can open
/// afresh in place.
///
/// The connections a device holds, its input watch and its beat clock's
/// output, do not survive it being unplugged: plugged back in, it is not
/// heard, and does not hear the clock, until opened afresh. It often comes
/// back under the same port, so a reload cannot tell it was gone, and opens
/// it afresh each time. Done in place, the MIDI controllers watching it and
/// the notifiers sending to it keep it, and are not restarted.
pub struct Reopenable {
    config: config::Midi,
    pub(super) device: std::sync::RwLock<Arc<Device>>,
    /// The sender of the watch going, if any, to keep it going on a device
    /// opened afresh. Its lock also keeps a watch from starting or stopping
    /// midway through a reopen.
    pub(super) watch: Mutex<Option<Sender<Vec<u8>>>>,
    /// Held across a whole reopen: two at once would each stop the clock of
    /// the device they found, and the second could stop the one the first
    /// is replacing rather than the one it installs, leaving two clocks on
    /// the port.
    reopening: Mutex<()>,
}

impl Reopenable {
    /// Opens the device `config` names, its beat clock holding `held` until
    /// a song sets another tempo: a device opened in place of one that could
    /// not be reopened goes on with the tempo it held.
    pub fn open_holding(
        config: config::Midi,
        held: Option<Duration>,
    ) -> Result<Reopenable, Box<dyn Error>> {
        let device = get_holding(&config, held)?;
        Ok(Reopenable::with_device(config, device))
    }

    pub(super) fn with_device(config: config::Midi, device: Device) -> Reopenable {
        Reopenable {
            config,
            device: std::sync::RwLock::new(Arc::new(device)),
            watch: Mutex::new(None),
            reopening: Mutex::new(()),
        }
    }

    /// The device as last opened. Taken out of the lock, so that a song
    /// playing on it does not hold a reopen off.
    pub(super) fn current(&self) -> Arc<Device> {
        self.device.read().expect("unable to get lock").clone()
    }

    /// Reopens the device as `open` opens it, given the tempo the current
    /// one holds.
    pub(super) fn reopen_with(
        &self,
        open: impl FnOnce(Option<Duration>) -> Result<Device, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        use crate::midi::Device as _;

        // The old clock stops, and is waited on, before the fresh one starts
        // with the tempo it held: both at once double every clock pulse the
        // gear following them sees, and a schedule it was still running sets
        // its tempo as it ends. Outside the lock: a watch starting or
        // stopping need not wait on it. Should the fresh device not open,
        // the reload opens one anew holding the same tempo
        // (`Device::held_tempo`).
        let _reopening = self.reopening.lock().expect("unable to get lock");
        let current = self.current();
        current.stop_beat_clock();
        let held = current.held_tempo();
        drop(current);
        let mut watch = self.watch.lock().expect("unable to get lock");
        let fresh = open(held)?;
        // The old watch goes first: both at once would hear every event
        // twice for a moment, and a pedal press taken twice is worse than
        // one missed during a config save.
        if let Some(sender) = watch.as_ref() {
            self.current().stop_watch_events();
            if let Err(e) = fresh.watch_events(sender.clone()) {
                // Nothing watches now. Its sender dropped, the controller
                // hears its channel close and ends, rather than read as
                // running while deaf. The fresh device goes outside the lock,
                // as an old one does below.
                *watch = None;
                drop(watch);
                drop(fresh);
                return Err(e);
            }
        }
        info!(device = fresh.name, "Opened the MIDI device afresh.");
        let old = mem::replace(
            &mut *self.device.write().expect("unable to get lock"),
            Arc::new(fresh),
        );
        // Let go of outside the locks: the last of it joins its beat clock's
        // thread, which a watch starting or stopping need not wait on.
        drop(watch);
        drop(old);
        Ok(())
    }
}

impl crate::midi::Device for Reopenable {
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>> {
        let mut watch = self.watch.lock().expect("unable to get lock");
        self.current().watch_events(sender.clone())?;
        *watch = Some(sender);
        Ok(())
    }

    fn stop_watch_events(&self) {
        let mut watch = self.watch.lock().expect("unable to get lock");
        self.current().stop_watch_events();
        *watch = None;
    }

    fn reopen(&self) -> Result<(), Box<dyn Error>> {
        // The tempo held between songs carries over: a fresh beat clock
        // knows none, and gear following it would stop until the next song.
        self.reopen_with(|held| get_holding(&self.config, held))
    }

    fn held_tempo(&self) -> Option<Duration> {
        self.current().held_tempo()
    }

    fn play_from(
        &self,
        song: Arc<Song>,
        sync: crate::playsync::PlaybackSync,
    ) -> Result<(), Box<dyn Error>> {
        self.current().play_from(song, sync)
    }

    fn emit(&self, midi_event: Option<LiveEvent<'static>>) -> Result<(), Box<dyn Error>> {
        self.current().emit(midi_event)
    }

    fn emit_sysex(&self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        self.current().emit_sysex(bytes)
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<crate::midi::mock::Device>, Box<dyn Error>> {
        Err("not a mock".into())
    }
}

impl fmt::Display for Reopenable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.current().fmt(f)
    }
}
