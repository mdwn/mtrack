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
use tokio::sync::mpsc::{error::TrySendError, Sender};
use tracing::{debug, error, info, span, warn, Level};

use crate::{clock::PlaybackClock, config, playsync::CancelHandle, songs::Song};

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

pub struct Device {
    name: String,
    playback_delay: Duration,
    beat_clock_enabled: bool,
    input_port: Option<MidiInputPort>,
    output_port: Option<MidiOutputPort>,
    event_connection: Box<Mutex<Option<MidiInputConnection<()>>>>,
    /// Device-owned, always-on beat clock. Present when `beat_clock_enabled` is
    /// set and the device has an output port. Songs retune it by submitting their
    /// tick schedule; between songs it either free-runs the last tempo (when
    /// `persist_tempo` is set) or stays silent.
    beat_clock_engine: Option<Arc<BeatClockEngine>>,
    /// For a test: refuses to watch events, as a device whose input
    /// cannot be connected would.
    #[cfg(test)]
    refuse_watch: bool,
    /// For a test: run when the device is dropped.
    #[cfg(test)]
    on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
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
            beat_clock_engine: None,
            #[cfg(test)]
            refuse_watch: false,
            #[cfg(test)]
            on_drop: None,
        }
    }

    /// The tempo its beat clock holds between songs, if it holds one.
    fn held_tempo(&self) -> Option<Duration> {
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
        });
        true
    }
}

impl super::Device for Device {
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
        };
        let span = span!(Level::INFO, "play song (midir)");
        let _enter = span.enter();

        // A song's tempo drives the beat clock without a MIDI file.
        let Some(midi_playback) = song.midi_playback() else {
            // A song's tempo drives the beat clock without a MIDI file.
            if self.beat_clock_engine.is_some() && song.has_tempo() {
                if self.play_beat_clock(&song, None, start_time, &cancel_handle, &clock, &sections)
                {
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
        self.play_beat_clock(
            &song,
            Some(&midi_sheet),
            start_time,
            &cancel_handle,
            &clock,
            &sections,
        );

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
    device: std::sync::RwLock<Arc<Device>>,
    /// The sender of the watch going, if any, to keep it going on a device
    /// opened afresh. Its lock also keeps a watch from starting or stopping
    /// midway through a reopen.
    watch: Mutex<Option<Sender<Vec<u8>>>>,
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

    fn with_device(config: config::Midi, device: Device) -> Reopenable {
        Reopenable {
            config,
            device: std::sync::RwLock::new(Arc::new(device)),
            watch: Mutex::new(None),
            reopening: Mutex::new(()),
        }
    }

    /// The device as last opened. Taken out of the lock, so that a song
    /// playing on it does not hold a reopen off.
    fn current(&self) -> Arc<Device> {
        self.device.read().expect("unable to get lock").clone()
    }

    /// Reopens the device as `open` opens it, given the tempo the current
    /// one holds.
    fn reopen_with(
        &self,
        open: impl FnOnce(Option<Duration>) -> Result<Device, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        use super::Device as _;

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

impl super::Device for Reopenable {
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
    fn to_mock(&self) -> Result<Arc<super::mock::Device>, Box<dyn Error>> {
        Err("not a mock".into())
    }
}

impl fmt::Display for Reopenable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.current().fmt(f)
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
    ready_tx: crate::playsync::ReadyGuard,
    finished: Arc<AtomicBool>,
    exclude_channels: &'a HashSet<u8>,
    clock: &'a PlaybackClock,
    loop_break: Arc<AtomicBool>,
    active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    /// The playback's section loop jumps, which the notes follow.
    section_jumps: Arc<crate::playsync::SectionJumpLog>,
}

/// Runs the MIDI playback thread body: signals readiness, waits for the clock
/// to start, sleeps through the playback delay (checking for cancellation),
/// then plays events.
fn run_playback(sender: &mut dyn MidiSender, mut ctx: PlaybackContext<'_>) {
    ctx.ready_tx.send();

    // Wait for the clock to start (the "go" signal from play_files).
    while ctx.clock.elapsed() == Duration::ZERO {
        if ctx.cancel_handle.is_cancelled() {
            ctx.finished.store(true, Ordering::Relaxed);
            ctx.cancel_handle.notify();
            return;
        }
        std::hint::spin_loop();
    }

    // The beat clock engine (if any) keys off this same shared clock's start
    // signal, so it begins in sync with us without an explicit rendezvous.

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

    // The song plays from where it starts, and on from each loop jump
    // committed to -- by the audio, or the playback's own publisher without
    // it -- sections and a looping song's passes alike: the notes go back
    // where and when the audio and lights do, the pass a jump leaves playing
    // on under the next for its overlap, and play on once a loop is broken.
    // While the log is open they follow it, even past their last note: a
    // section armed then, or one looping past it, plays them again (#606).
    let first_clock_base = ctx.clock.elapsed();
    let mut jumps = crate::playsync::JumpCursor::new(ctx.section_jumps.clone());
    play_streams(
        &NoteStreams {
            precomputed: ctx.precomputed,
            start_time: ctx.start_time,
            first_clock_base,
            cancel_handle: ctx.cancel_handle,
            loop_break: &ctx.loop_break,
            active_section: &ctx.active_section,
            exclude_channels: ctx.exclude_channels,
            clock: ctx.clock,
        },
        &mut jumps,
        sender,
    );

    ctx.finished.store(true, Ordering::Relaxed);
    ctx.cancel_handle.notify();
}

/// What [`play_streams`] plays.
struct NoteStreams<'a> {
    precomputed: &'a super::playback::PrecomputedMidi,
    /// Where in the song playback began; the jumps are timed on the
    /// playback timeline, which starts here.
    start_time: Duration,
    /// What the clock read when the first note's stream began (after the
    /// playback delay).
    first_clock_base: Duration,
    cancel_handle: &'a CancelHandle,
    loop_break: &'a AtomicBool,
    active_section: &'a parking_lot::RwLock<Option<crate::player::SectionBounds>>,
    exclude_channels: &'a HashSet<u8>,
    clock: &'a PlaybackClock,
}

/// One run through the song's notes: from where playback began, or from a
/// jump's target.
struct NoteStream {
    /// When the stream's song time 0 falls on the clock, in seconds (negative
    /// for one that began part way in).
    origin: f64,
    /// Its next event.
    index: usize,
    /// Where in the song it stops, once a section's jump has left it: its
    /// `from`, or, playing on under the next time round, its `from` plus the
    /// overlap. A whole-song pass plays all its notes, to the end of its
    /// MIDI.
    end: Option<Duration>,
    /// The notes it has on, by channel and key: let go if it is cut.
    held: Vec<(u8, u8)>,
}

impl NoteStream {
    /// Notes what the stream sends of `event`, held or let go.
    fn sent(&mut self, event: &super::playback::TimedMidiEvent) {
        use midly::MidiMessage;
        let note = match event.message {
            MidiMessage::NoteOn { key, vel } if vel > 0 => Some((key.as_int(), true)),
            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                Some((key.as_int(), false))
            }
            _ => None,
        };
        if let Some((key, on)) = note {
            self.held.retain(|&held| held != (event.channel, key));
            if on {
                self.held.push((event.channel, key));
            }
        }
    }

    /// When the stream next does something, in song time, and whether that
    /// is to let go of its notes where it is cut; `None` once it is done.
    fn next(&self, events: &[super::playback::TimedMidiEvent]) -> Option<(Duration, bool)> {
        match events.get(self.index) {
            Some(event) if self.end.is_none_or(|end| event.time < end) => Some((event.time, false)),
            _ => self
                .end
                .filter(|_| !self.held.is_empty())
                .map(|end| (end, true)),
        }
    }

    /// Lets go of the notes the stream has on, cut where it ends.
    fn let_go(&mut self, sender: &mut dyn MidiSender) {
        for (channel, key) in self.held.drain(..) {
            let off = LiveEvent::Midi {
                channel: channel.into(),
                message: midly::MidiMessage::NoteOff {
                    key: key.into(),
                    vel: 0.into(),
                },
            };
            let mut bytes = Vec::with_capacity(3);
            if off.write_std(&mut bytes).is_ok() {
                if let Err(e) = sender.send(&bytes) {
                    debug!("MIDI send failed: {:?}", e);
                }
            }
        }
    }
}

/// Plays the song's notes as one merged run over its streams: the first from
/// where playback began, and another from each jump's target as it falls
/// due. A jump ends the stream it leaves at its `from`, or, with an overlap
/// (a looping song's pass crossfading into the next), lets it play on to
/// `from + overlap`, its notes in time with the new stream's. Events of two
/// streams due at the same moment go older stream first. Returns on cancel
/// or loop break, or when the notes have run out and no more jumps can come.
fn play_streams(
    params: &NoteStreams<'_>,
    jumps: &mut crate::playsync::JumpCursor,
    sender: &mut dyn MidiSender,
) {
    // Waits on the clock in steps this long while a jump may be committed.
    const JUMP_CHECK: Duration = Duration::from_millis(10);
    // Otherwise in steps still shorter than the 30ms a jump is committed
    // ahead, so one armed meanwhile is not missed.
    const IDLE_CHECK: Duration = Duration::from_millis(25);
    let events = params.precomputed.events();
    let song_start = params.start_time.as_secs_f64();
    let mut streams = vec![NoteStream {
        origin: params.first_clock_base.as_secs_f64() - song_start,
        index: events.partition_point(|e| e.time < params.start_time),
        end: None,
        held: Vec::new(),
    }];
    let mut buf = Vec::with_capacity(8);
    // The newest stream's origin, kept for a jump that comes once every
    // stream has run out of notes.
    let mut last_origin = streams[0].origin;

    loop {
        if params.cancel_handle.is_cancelled() || params.loop_break.load(Ordering::Relaxed) {
            return;
        }
        if let Some(current) = streams.last() {
            last_origin = current.origin;
        }
        // A stream is done once out of notes, or at its end, cut there once
        // it has let go of the notes it has on. The newest, out of notes with
        // some still on, stays for a section's jump to cut.
        let newest = streams.len().saturating_sub(1);
        let mut i = 0;
        streams.retain(|stream| {
            let keep = stream.next(events).is_some()
                || (i == newest && stream.end.is_none() && !stream.held.is_empty());
            i += 1;
            keep
        });
        // The earliest note due, older stream first at the same moment (to
        // the microsecond, past float rounding); `streams` is in age order.
        let next_event = streams
            .iter()
            .enumerate()
            .filter_map(|(i, stream)| {
                stream
                    .next(events)
                    .map(|(at, _)| (stream.origin + at.as_secs_f64(), i))
            })
            .min_by_key(|(at, i)| ((at * 1e6).round() as i64, *i));
        let next_jump = jumps.peek();
        // A jump falling due before the next note (or with it): the song goes
        // on from there, at its time -- on the schedule of the stream it
        // leaves, where its `from` falls, so the streams keep one grid (the
        // playback delay included) rather than each starting a poll late.
        let jump_at = next_jump.as_ref().map(|jump| match streams.last() {
            Some(current) => current.origin + jump.from.as_secs_f64(),
            None => last_origin + jump.from.as_secs_f64(),
        });
        let now = params.clock.elapsed().as_secs_f64();
        match (next_event, jump_at) {
            (_, Some(at)) if next_event.is_none_or(|(due, _)| at <= due) => {
                if now >= at {
                    let jump = jumps.due(Duration::MAX).expect("the jump peeked at is due");
                    debug!(
                        from = ?jump.from,
                        to = ?jump.to,
                        overlap = ?jump.overlap,
                        "MIDI loop: going back"
                    );
                    if let Some(current) = streams.last() {
                        last_origin = current.origin;
                    }
                    // A section's jump cuts the stream it leaves; a pass of
                    // the whole song plays out its own notes, a note-off on
                    // or past the loop point included.
                    if let Some(current) = streams.last_mut().filter(|_| !jump.pass) {
                        let end = jump.from + jump.overlap;
                        current.end = Some(current.end.map_or(end, |e| e.min(end)));
                    }
                    streams.push(NoteStream {
                        origin: at - jump.to.as_secs_f64(),
                        index: events.partition_point(|e| e.time < jump.to),
                        end: None,
                        held: Vec::new(),
                    });
                    continue;
                }
                spin_sleep::sleep(Duration::from_secs_f64(at - now).min(JUMP_CHECK));
            }
            (Some((due, stream)), _) => {
                if now >= due {
                    let stream = &mut streams[stream];
                    if stream.next(events).is_some_and(|(_, cut)| cut) {
                        stream.let_go(sender);
                        continue;
                    }
                    let event = &events[stream.index];
                    if let Some(bytes) =
                        serialize_midi_event(event, params.exclude_channels, &mut buf)
                    {
                        if let Err(e) = sender.send(&bytes) {
                            debug!("MIDI send failed: {:?}", e);
                        }
                        stream.sent(event);
                    }
                    stream.index += 1;
                    continue;
                }
                // In short steps while a jump could come (one committed, a
                // section armed, or the log open); otherwise longer, still
                // short of the time a jump is committed ahead.
                let step = if next_jump.is_some()
                    || params.active_section.read().is_some()
                    || !jumps.log().is_closed()
                {
                    JUMP_CHECK
                } else {
                    IDLE_CHECK
                };
                spin_sleep::sleep(Duration::from_secs_f64(due - now).min(step));
            }
            // Out of notes: on while a jump may yet come, which plays them
            // again.
            (None, _) => {
                if !jumps.may_jump() {
                    return;
                }
                thread::sleep(JUMP_CHECK / 2);
            }
        }
    }
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
///
/// Returns the spacing between the final two clock ticks it actually delivered —
/// i.e. the instantaneous tempo at the point playback ended (natural end or
/// cancellation). Returns `None` if fewer than two ticks were sent, in which case
/// the tempo can't be determined. Callers use this to keep the clock free-running
/// at the last known tempo between songs.
#[allow(clippy::too_many_arguments)]
fn run_beat_clock(
    sender: &mut dyn MidiSender,
    ticks: &[Duration],
    start_time: Duration,
    sections: Option<&BeatClockSections>,
    reference: Option<&ReferenceClock>,
    cancel_handle: &CancelHandle,
    clock: &PlaybackClock,
    rx: &mpsc::Receiver<BeatClockCommand>,
) -> ScheduleOutcome {
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
    // The MIDI spec asks for a millisecond between Start and the first clock,
    // which with a pulse due at once would follow it back to back.
    let start_sent = std::time::Instant::now();
    let mut first_clock = true;

    let clock_bytes = realtime_bytes(SystemRealtime::TimingClock);
    let stop_bytes = realtime_bytes(SystemRealtime::Stop);

    // Track the spacing between consecutive delivered ticks so we can report the
    // final tempo when we stop.
    let mut prev_tick: Option<Duration> = None;
    let mut last_interval: Option<Duration> = None;

    let mut schedule = PassTicks::new(ticks, start_time);
    // How far section loops have put the song back: a tick at song time `t`
    // falls due `t + looped - start_time` into the schedule.
    let mut looped = Duration::ZERO;
    let mut jumped = false;
    let mut discipline = Discipline::default();
    let mut follow = SectionFollow::new(sections);
    'ticks: loop {
        let Some(tick_time) = schedule.next() else {
            // Out of ticks with a section still going round (its end at or
            // past the last of them): its jump is still to come.
            loop {
                if !follow.pending(ticks) {
                    break 'ticks;
                }
                if cancel_handle.is_cancelled() {
                    let _ = sender.send(&stop_bytes);
                    return ScheduleOutcome::Finished(last_interval);
                }
                match rx.try_recv() {
                    Ok(command) => {
                        let _ = sender.send(&stop_bytes);
                        return ScheduleOutcome::Preempted(command, last_interval);
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let _ = sender.send(&stop_bytes);
                        return ScheduleOutcome::Finished(last_interval);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                // A jump committed is taken at once, however far ahead it
                // falls: no tick is left before it, and the schedule then
                // sleeps to its first tick as to any other. Waiting for it to
                // fall due polled the next pass's first pulse, up to a poll
                // late.
                if let Some(jump) = follow.due(Duration::MAX) {
                    if lands_in(ticks, &jump) {
                        looped = schedule.go_back(&jump);
                        jumped = true;
                        continue 'ticks;
                    }
                    continue;
                }
                thread::sleep(Duration::from_millis(5));
            }
        };
        // A loop's jump due by this tick takes the song back, as it takes the
        // notes and audio: the tick due there is the start's. Due within half
        // a pulse of it, too: a loop's length is whole samples, so a loop of
        // whole pulses can come out a hair long, which put the pass's last
        // pulse microseconds before the next's first -- a pulse too many at
        // every seam.
        let half_pulse = schedule.pulse() / 2;
        while let Some(jump) = follow.due(tick_time + looped + half_pulse) {
            if lands_in(ticks, &jump) {
                looped = schedule.go_back(&jump);
                jumped = true;
                continue 'ticks;
            }
        }
        let seam = std::mem::take(&mut jumped);
        // A broken loop ends the clock, as it ends the notes: the player
        // stops the song with it.
        if sections.is_some_and(|sections| sections.loop_break.load(Ordering::Relaxed))
            || cancel_handle.is_cancelled()
        {
            let _ = sender.send(&stop_bytes);
            return ScheduleOutcome::Finished(last_interval);
        }
        // The gap across a seam is no tempo: a stop just past it would hold
        // that as the tempo to keep.
        if seam {
            prev_tick = None;
        }

        // A newer command supersedes this schedule. Nothing cancels the handle
        // when a song ends naturally -- the cleanup path only clears the join
        // and advances the playlist -- so a schedule that outlasts its audio
        // (a MIDI file longer than the song) would otherwise keep running,
        // dequeue the next song late, and shift its Start and every tick by the
        // residual. This is also how Shutdown gets observed mid-schedule, so
        // dropping the device does not block for the rest of the song.
        match rx.try_recv() {
            Ok(command) => {
                let _ = sender.send(&stop_bytes);
                return ScheduleOutcome::Preempted(command, last_interval);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let _ = sender.send(&stop_bytes);
                return ScheduleOutcome::Finished(last_interval);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        // Due on the wall clock, steered toward the playback clock the notes
        // and audio follow, so a long loop does not drift off them.
        let target_wall = discipline.steer((tick_time + looped).saturating_sub(start_time));
        let elapsed = clock.elapsed();
        if target_wall > elapsed {
            discipline.sleep_until(target_wall, clock, reference);
        }
        // A jump the audio committed to while this waited: the tick due at it
        // is the section start's.
        while let Some(jump) = follow.due(tick_time + looped + half_pulse) {
            if lands_in(ticks, &jump) {
                looped = schedule.go_back(&jump);
                jumped = true;
                continue 'ticks;
            }
        }

        if cancel_handle.is_cancelled() {
            let _ = sender.send(&stop_bytes);
            return ScheduleOutcome::Finished(last_interval);
        }

        if std::mem::take(&mut first_clock) {
            let since = start_sent.elapsed();
            if since < START_TO_CLOCK {
                spin_sleep::sleep(START_TO_CLOCK - since);
            }
        }
        if let Err(e) = sender.send(&clock_bytes) {
            debug!("MIDI beat clock send failed: {:?}", e);
        }

        let due = tick_time + looped;
        if let Some(prev) = prev_tick {
            last_interval = Some(due.saturating_sub(prev));
        }
        prev_tick = Some(due);
    }

    // Send STOP when finished
    let _ = sender.send(&stop_bytes);
    ScheduleOutcome::Finished(last_interval)
}

/// The least time between a Start or Continue and the first clock after it.
const START_TO_CLOCK: Duration = Duration::from_millis(1);

/// When each of a schedule's ticks falls due, as time on the song's timeline
/// from a start at `start` (so due `time - start` after it). A loop's jumps,
/// sections and a looping song's passes alike, take it back
/// ([`PassTicks::go_back`]): it keeps no passes of its own, but follows the
/// playback's log, as the notes and lights do.
#[derive(Clone)]
struct PassTicks<'a> {
    ticks: &'a [Duration],
    /// Ticks before this song time are skipped: the start, until a seek.
    skip_before: Duration,
    /// Its next tick.
    index: usize,
}

impl<'a> PassTicks<'a> {
    fn new(ticks: &'a [Duration], start: Duration) -> Self {
        PassTicks {
            ticks,
            skip_before: start,
            index: 0,
        }
    }
}

impl PassTicks<'_> {
    /// Goes back (or on) to the first tick at or after `position`, for a
    /// loop: what comes next is played however early it lies in the song.
    fn seek(&mut self, position: Duration) {
        self.index = self.ticks.partition_point(|tick| *tick < position);
        self.skip_before = Duration::ZERO;
    }

    /// Takes a loop's jump: back to its target, giving how far the song is
    /// then put back (`looped`), so that the target falls due at the jump's
    /// time. A pass's overlap is no matter to the clock: its pulses switch to
    /// the new pass at the jump.
    fn go_back(&mut self, jump: &crate::section_loop::SectionJump) -> Duration {
        self.seek(jump.to);
        jump.at.saturating_sub(jump.to)
    }

    /// The gap from the tick last given to the next (or, at the end, from the
    /// one before it).
    fn pulse(&self) -> Duration {
        let index = self.index.max(1);
        match (self.ticks.get(index - 1), self.ticks.get(index)) {
            (Some(last), Some(next)) => next.saturating_sub(*last),
            (Some(last), None) if index >= 2 => last.saturating_sub(self.ticks[index - 2]),
            _ => Duration::ZERO,
        }
    }
}

impl Iterator for PassTicks<'_> {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let tick = *self.ticks.get(self.index)?;
            self.index += 1;
            if tick >= self.skip_before {
                return Some(tick);
            }
        }
    }
}

/// How a schedule ended, and the tempo it established.
enum ScheduleOutcome {
    /// Played to its end, or stopped by cancellation.
    Finished(Option<Duration>),
    /// Superseded by a command that arrived mid-schedule. The command has been
    /// taken off the channel and must be acted on by the caller.
    Preempted(BeatClockCommand, Option<Duration>),
}

impl ScheduleOutcome {
    /// The tempo this schedule established, if it delivered enough ticks to
    /// determine one.
    fn interval(&self) -> Option<Duration> {
        match self {
            ScheduleOutcome::Finished(interval) => *interval,
            ScheduleOutcome::Preempted(_, interval) => *interval,
        }
    }
}

/// A song's beat clock schedule, submitted to the [`BeatClockEngine`].
struct BeatClockPlay {
    /// Absolute tick timestamps for the song (from the song's tempo map).
    ticks: Arc<Vec<Duration>>,
    /// Position in the tick timeline to begin from (for seeks/section starts).
    start_time: Duration,
    /// Delay before the first clock is sent, matching note playback.
    playback_delay: Duration,
    /// Cancellation for this song.
    cancel: CancelHandle,
    /// The shared playback clock whose `start()` is the "go" signal. The engine
    /// waits on it (cancel-aware) so its `Start` lands with the first note.
    clock: PlaybackClock,
    /// The song's section loop, which the clock follows as its notes do.
    sections: Option<BeatClockSections>,
}

/// A song's section loop, which the clock follows as its notes and lights do:
/// the player's handles, and the playback's log of the jumps committed to.
#[derive(Clone)]
struct BeatClockSections {
    active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
    section_loop_break: Arc<AtomicBool>,
    jumps: Arc<crate::playsync::SectionJumpLog>,
    /// The song's loop broken: the clock ends, as the notes do.
    loop_break: Arc<AtomicBool>,
    /// Whether the song loops as a whole: out of ticks, its next pass is to
    /// come while the log is open.
    loops: bool,
}

impl BeatClockSections {
    /// The section looping now: armed, and not being broken out of.
    fn looping(&self) -> Option<crate::player::SectionBounds> {
        if self.section_loop_break.load(Ordering::Relaxed) {
            return None;
        }
        self.active_section.read().clone()
    }
}

/// A schedule's way through the playback's section loop jumps.
struct SectionFollow<'a> {
    sections: Option<&'a BeatClockSections>,
    cursor: Option<crate::playsync::JumpCursor>,
}

impl<'a> SectionFollow<'a> {
    fn new(sections: Option<&'a BeatClockSections>) -> Self {
        SectionFollow {
            sections,
            cursor: sections
                .map(|sections| crate::playsync::JumpCursor::new(sections.jumps.clone())),
        }
    }

    /// The jump falling due by `at` on the playback timeline, if one does.
    fn due(&mut self, at: Duration) -> Option<crate::section_loop::SectionJump> {
        self.cursor.as_mut()?.due(at)
    }

    /// Whether a jump the schedule could follow may yet come: one committed,
    /// or, while the log is open, a looping song's next pass, or a section
    /// looping with a tick in it to go back to.
    fn pending(&self, ticks: &[Duration]) -> bool {
        let Some(cursor) = self.cursor.as_ref() else {
            return false;
        };
        if cursor.peek().is_some() {
            return true;
        }
        if cursor.log().is_closed() {
            return false;
        }
        self.sections
            .is_some_and(|sections| sections.loops && !sections.loop_break.load(Ordering::Relaxed))
            || self
                .sections
                .and_then(BeatClockSections::looping)
                .is_some_and(|section| {
                    lands_in(
                        ticks,
                        &crate::section_loop::SectionJump {
                            at: Duration::ZERO,
                            from: section.end_time,
                            to: section.start_time,
                            overlap: Duration::ZERO,
                            pass: false,
                        },
                    )
                })
    }
}

/// Whether a jump lands on a tick: one in the stretch it goes back over. A
/// section with none in it (shorter than a pulse, between two) is not
/// followed: going back to it would find no tick to play, for ever.
fn lands_in(ticks: &[Duration], jump: &crate::section_loop::SectionJump) -> bool {
    let index = ticks.partition_point(|tick| *tick < jump.to);
    ticks.get(index).is_some_and(|tick| *tick < jump.from)
}

/// The playback clock the notes and audio follow (on a rig with audio, the
/// interface's sample counter), and the playback delay between its zero and
/// the schedule's.
struct ReferenceClock {
    clock: PlaybackClock,
    delay: Duration,
}

impl ReferenceClock {
    /// How far ahead of `wall` it reads, in seconds, for a schedule timed on
    /// `wall`.
    fn offset_from(&self, wall: &PlaybackClock) -> f64 {
        let reference = self.clock.elapsed().as_secs_f64() - self.delay.as_secs_f64();
        reference - wall.elapsed().as_secs_f64()
    }
}

/// Steers a schedule timed on the wall clock toward the playback clock, which
/// on a rig with audio counts the interface's samples and so runs at its
/// crystal's rate, not the computer's: tens of ppm apart, a few ms a minute,
/// which a loop played for minutes builds up against the audio.
///
/// The playback clock moves only once an audio buffer, so a reading of it
/// lags by up to a buffer; the most it reads ahead over a window is a fair
/// measure. The first window sets where it started, and from then on the
/// schedule follows how far that has moved, a little at a time so no tick
/// jumps.
#[derive(Default)]
struct Discipline {
    window_start: Option<Duration>,
    window_max: Option<f64>,
    baseline: Option<f64>,
    correction: f64,
    /// For where in each wait a reading is taken (xorshift; 0 until seeded).
    rng: u64,
}

impl Discipline {
    const WINDOW: Duration = Duration::from_secs(2);
    /// The most the correction moves a window: 0.5ms in 2s, 250ppm.
    const MAX_STEP: f64 = 0.0005;

    /// A pseudo-random fraction in [0, 1): where in a wait the next reading
    /// is taken.
    fn next_fraction(&mut self) -> f64 {
        if self.rng == 0 {
            self.rng = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9E37_79B9_7F4A_7C15, |since| since.as_nanos() as u64)
                | 1;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Sleeps until `target` on `wall`, taking a reading of `reference` at a
    /// random point on the way. Readings taken at the ticks themselves fell
    /// at one place in the playback clock's steps for a tempo whose pulse is
    /// a whole number of audio buffers (117.1875 BPM at 48kHz), so the most
    /// a window read was not the top of a step, and wandered as the two
    /// drifted: the steering then put the clock off by up to a buffer.
    fn sleep_until(
        &mut self,
        target: Duration,
        wall: &PlaybackClock,
        reference: Option<&ReferenceClock>,
    ) {
        let now = wall.elapsed();
        if target <= now {
            return;
        }
        let Some(reference) = reference else {
            spin_sleep::sleep(target - now);
            return;
        };
        spin_sleep::sleep((target - now).mul_f64(self.next_fraction()));
        self.observe(wall.elapsed(), reference.offset_from(wall));
        let now = wall.elapsed();
        if target > now {
            spin_sleep::sleep(target - now);
        }
    }

    /// Takes a reading, at `wall` time into the schedule, of how far ahead of
    /// it the playback clock reads.
    fn observe(&mut self, wall: Duration, ahead: f64) {
        if !ahead.is_finite() {
            return;
        }
        self.window_max = Some(self.window_max.map_or(ahead, |max| max.max(ahead)));
        let window_start = *self.window_start.get_or_insert(wall);
        if wall.saturating_sub(window_start) < Self::WINDOW {
            return;
        }
        let measured = self.window_max.take().unwrap_or(ahead);
        self.window_start = Some(wall);
        match self.baseline {
            None => self.baseline = Some(measured),
            Some(baseline) => {
                let drift = measured - baseline;
                self.correction += (drift - self.correction).clamp(-Self::MAX_STEP, Self::MAX_STEP);
            }
        }
    }

    /// When a tick due `due` into the schedule goes out on the wall clock:
    /// as much earlier as the playback clock has run ahead of it, or later as
    /// it has fallen behind.
    fn steer(&self, due: Duration) -> Duration {
        let correction = Duration::from_secs_f64(self.correction.abs());
        if self.correction >= 0.0 {
            due.saturating_sub(correction)
        } else {
            due + correction
        }
    }
}

/// A command sent to the beat clock engine thread.
enum BeatClockCommand {
    /// Play a song's schedule, then hold its final tempo.
    Play(BeatClockPlay),
    /// Terminate the engine thread.
    Shutdown,
}

/// A device-owned, always-on MIDI beat clock.
///
/// A single long-lived thread owns the clock output for the device's lifetime.
/// Songs don't spawn their own clock threads — they submit a [`BeatClockPlay`]
/// and the engine plays it (see [`run_beat_clock`]), then, if `persist_tempo` is
/// set, keeps emitting timing clocks (0xF8) at the song's final tempo so
/// downstream gear holds its tempo until the next song retunes the engine. With
/// `persist_tempo` off it stays silent between songs, matching the original
/// send-Start/clocks/Stop-then-silent behavior.
struct BeatClockEngine {
    tx: mpsc::Sender<BeatClockCommand>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
    /// The tempo it holds between songs, as the engine thread last set it.
    tempo: Arc<Mutex<Option<Duration>>>,
}

impl BeatClockEngine {
    /// Spawns the engine thread, which opens its own output connection to `port`
    /// and waits for schedules, holding `held` until one sets another tempo.
    fn new(port: MidiOutputPort, persist_tempo: bool, held: Option<Duration>) -> Self {
        let (tx, rx) = mpsc::channel::<BeatClockCommand>();
        let tempo = Arc::new(Mutex::new(held));
        let engine_tempo = tempo.clone();
        let join = thread::spawn(move || {
            let output = match MidiOutput::new("mtrack beat clock output") {
                Ok(output) => output,
                Err(e) => {
                    warn!("Unable to create beat clock output: {:?}", e);
                    return;
                }
            };
            let mut connection = match output.connect(&port, "mtrack beat clock") {
                Ok(connection) => connection,
                Err(e) => {
                    warn!("Unable to connect beat clock output: {:?}", e);
                    return;
                }
            };

            // Elevate to real-time thread priority to minimize scheduling jitter
            // on clock tick delivery.
            promote_to_realtime_thread();

            beat_clock_engine_loop(&mut connection, persist_tempo, &engine_tempo, &rx);
        });

        BeatClockEngine {
            tx,
            join: Mutex::new(Some(join)),
            tempo,
        }
    }

    /// The tempo it holds between songs: the last a song set, if any.
    fn held_tempo(&self) -> Option<Duration> {
        *self.tempo.lock().expect("beat clock tempo lock")
    }

    /// Stops the engine thread and waits for it to end. The tempo it held
    /// stays readable.
    fn stop(&self) {
        let _ = self.tx.send(BeatClockCommand::Shutdown);
        if let Some(handle) = self.join.lock().expect("beat clock engine lock").take() {
            let _ = handle.join();
        }
    }

    /// Submits a song's schedule to the engine.
    fn play(&self, play: BeatClockPlay) {
        let _ = self.tx.send(BeatClockCommand::Play(play));
    }
}

impl Drop for BeatClockEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(BeatClockCommand::Shutdown);
        if let Some(handle) = self.join.lock().expect("beat clock engine lock").take() {
            let _ = handle.join();
        }
    }
}

/// The engine's main loop: alternately plays a submitted schedule and (when
/// `persist_tempo` is set) free-runs the last tempo until the next schedule
/// arrives. `tempo` holds that tempo, from before the engine started too: a
/// device opened afresh goes on with the one its predecessor held.
fn beat_clock_engine_loop(
    sender: &mut dyn MidiSender,
    persist_tempo: bool,
    tempo: &Mutex<Option<Duration>>,
    rx: &mpsc::Receiver<BeatClockCommand>,
) {
    // The inter-tick interval of the last schedule we played, used to hold tempo.
    let mut last_interval: Option<Duration> = *tempo.lock().expect("beat clock tempo lock");
    // The live free-run. Kept across the whole idle-to-startup stretch so its
    // tick phase is continuous: rebuilding it at each handover throws away the
    // part of the interval already elapsed and delays the next tick.
    let mut hold: Option<TempoHold> = None;
    // A command taken off the channel by a schedule it interrupted, waiting to
    // be acted on in place of the next receive.
    let mut pending: Option<BeatClockCommand> = None;

    loop {
        let command = match pending.take() {
            Some(command) => command,
            None => match next_command(sender, persist_tempo, last_interval, &mut hold, rx) {
                Some(command) => command,
                // Sender dropped: the device is going away.
                None => return,
            },
        };

        let play = match command {
            BeatClockCommand::Play(play) => play,
            BeatClockCommand::Shutdown => return,
        };

        // The tempo to hold through this song's startup, if there is one.
        let held = last_interval.filter(|_| persist_tempo);

        // Wait for the shared "go" signal so our Start lands with the first
        // note, holding the tempo across the wait rather than going silent for
        // however long the rest of the player takes to get ready. Cancel-aware,
        // so an early stop can't wedge the engine.
        let interrupted = wait_for_start_holding_tempo(
            sender,
            &play,
            held.map(|interval| hold_at(&mut hold, interval)),
            rx,
            &mut pending,
        );
        if interrupted || play.cancel.is_cancelled() {
            continue;
        }
        let started_at = play
            .clock
            .started_at()
            .unwrap_or_else(std::time::Instant::now);

        // Sleep the playback delay, matching the note thread -- still holding
        // the tempo, for the same reason as above, and hearing a command as
        // the wait for the start does: a stop for a reopen joins this thread.
        if !play.playback_delay.is_zero() {
            const COMMAND_CHECK: Duration = Duration::from_millis(50);
            let mut hold = held.map(|interval| hold_at(&mut hold, interval));
            // From the playback clock's start, as the notes count it, not
            // from when this thread noticed it (#600).
            let delay = PlaybackClock::wall_from(started_at);
            let mut interrupted = false;
            while delay.elapsed() < play.playback_delay && !play.cancel.is_cancelled() {
                let left = play.playback_delay.saturating_sub(delay.elapsed());
                let step = hold
                    .as_ref()
                    .map_or(COMMAND_CHECK, |hold| hold.until_next().min(COMMAND_CHECK));
                spin_sleep::sleep(step.min(left));
                if heard_command(rx, &mut pending) {
                    interrupted = true;
                    break;
                }
                if let Some(hold) = hold.as_deref_mut() {
                    hold.tick_if_due(sender);
                }
            }
            if interrupted {
                continue;
            }
        }
        if play.cancel.is_cancelled() {
            continue;
        }

        // The schedule owns the clock from here; the next idle stretch starts a
        // fresh hold from whatever tempo this song establishes.
        hold = None;

        // Use a dedicated wall clock for tick timing instead of the audio-derived
        // playback clock: the audio clock only updates once per buffer callback
        // (~23ms at 1024/44.1kHz), which would make the beat clock's spin_sleep
        // overshoot by variable amounts. A wall clock is smooth and sub-ms.
        //
        // Its zero is the playback clock's start plus the delay: the notes'
        // zero, not wherever this thread got to after noticing the start.
        let wall = PlaybackClock::wall_from(started_at + play.playback_delay);

        let reference = ReferenceClock {
            clock: play.clock.clone(),
            delay: play.playback_delay,
        };
        let outcome = run_beat_clock(
            sender,
            &play.ticks,
            play.start_time,
            play.sections.as_ref(),
            Some(&reference),
            &play.cancel,
            &wall,
            rx,
        );

        // Only replace the held tempo when this schedule actually established
        // one. `run_beat_clock` reports `None` when it delivered fewer than two
        // ticks -- a stop within ~42ms at 120 BPM, or a seek leaving one tick to
        // play -- and overwriting with that dropped a tempo the engine was
        // successfully holding, silencing gear mid-set for no reason. Keep the
        // last tempo we actually know.
        if let Some(interval) = outcome.interval() {
            last_interval = Some(interval);
            *tempo.lock().expect("beat clock tempo lock") = last_interval;
        }
        if let ScheduleOutcome::Preempted(command, _) = outcome {
            pending = Some(command);
        }
    }
}

/// Returns the next command to act on. If persistence is enabled and a tempo is
/// known, free-runs the clock at that tempo while waiting; otherwise blocks
/// silently. Returns `None` if the sender is dropped.
fn next_command(
    sender: &mut dyn MidiSender,
    persist_tempo: bool,
    last_interval: Option<Duration>,
    hold: &mut Option<TempoHold>,
    rx: &mpsc::Receiver<BeatClockCommand>,
) -> Option<BeatClockCommand> {
    // A schedule may already be queued (e.g. a rapid stop→play); take it now so we
    // don't emit an idle tempo blip between back-to-back songs.
    match rx.try_recv() {
        Ok(command) => return Some(command),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
    }

    match (persist_tempo, last_interval) {
        (true, Some(interval)) => run_idle_until_command(sender, hold_at(hold, interval), rx),
        // Not persisting (or no tempo yet): wait silently for the next schedule.
        _ => {
            *hold = None;
            rx.recv().ok()
        }
    }
}

/// Paces free-running timing clocks at a fixed interval against a wall clock,
/// so a held tempo neither drifts nor bursts.
struct TempoHold {
    clock: PlaybackClock,
    interval: Duration,
    /// When the next tick is due, measured from `clock`'s start.
    next_tick: Duration,
}

impl TempoHold {
    fn new(interval: Duration) -> TempoHold {
        let clock = PlaybackClock::wall();
        clock.start();
        TempoHold {
            clock,
            interval,
            next_tick: interval,
        }
    }

    /// How long until the next tick is due, for use as a wait timeout.
    fn until_next(&self) -> Duration {
        self.until_next_at(self.clock.elapsed())
    }

    /// [`TempoHold::until_next`] against a supplied elapsed time.
    ///
    /// The pacing decisions are split from the clock that feeds them so they can
    /// be tested by handing over the times to reason about, with no threads and
    /// no sleeping. Timing-derived tests of this logic were flaky on a loaded
    /// runner and proved nothing a table of elapsed values does not.
    fn until_next_at(&self, elapsed: Duration) -> Duration {
        self.next_tick.saturating_sub(elapsed)
    }

    /// Emits a tick if one is due.
    ///
    /// When more than a full interval late, the schedule is re-anchored to now
    /// instead of the backlog being replayed. A thread that loses the CPU (or a
    /// host that suspends) can come back seconds behind, and repaying that debt
    /// means firing every missed tick back to back with no spacing -- ten
    /// seconds at 120 BPM is ~480 clocks in a burst, which downstream gear
    /// reads as an enormous tempo spike. A held tempo is a tempo, not a debt.
    fn tick_if_due(&mut self, sender: &mut dyn MidiSender) {
        self.tick_if_due_at(sender, self.clock.elapsed());
    }

    /// [`TempoHold::tick_if_due`] against a supplied elapsed time. Returns
    /// whether a tick was emitted.
    fn tick_if_due_at(&mut self, sender: &mut dyn MidiSender, elapsed: Duration) -> bool {
        use midly::live::SystemRealtime;

        if elapsed < self.next_tick {
            return false;
        }
        if let Err(e) = sender.send(&realtime_bytes(SystemRealtime::TimingClock)) {
            debug!("MIDI idle beat clock send failed: {:?}", e);
        }
        self.next_tick = if elapsed.saturating_sub(self.next_tick) > self.interval {
            elapsed + self.interval
        } else {
            self.next_tick + self.interval
        };
        true
    }
}

/// Returns a hold running at `interval`, reusing `hold` if it already is.
///
/// Reused rather than rebuilt so the tick phase survives the handover from
/// waiting-for-a-command to waiting-for-the-go-signal. A fresh hold restarts the
/// count from now, which discards however much of the current interval had
/// already elapsed and pushes the next tick up to a full interval late -- worth
/// two missing pulses at the song boundary, measured, for no reason.
fn hold_at(hold: &mut Option<TempoHold>, interval: Duration) -> &mut TempoHold {
    match hold {
        Some(existing) if existing.interval == interval => {}
        _ => *hold = Some(TempoHold::new(interval)),
    }
    hold.as_mut().expect("just populated")
}

/// Free-runs the clock, emitting timing clocks until a command arrives
/// (returned) or the sender is dropped (`None`). Timing is anchored to a wall
/// clock so the tempo does not drift.
fn run_idle_until_command(
    sender: &mut dyn MidiSender,
    hold: &mut TempoHold,
    rx: &mpsc::Receiver<BeatClockCommand>,
) -> Option<BeatClockCommand> {
    use std::sync::mpsc::RecvTimeoutError;

    loop {
        match rx.recv_timeout(hold.until_next()) {
            Ok(command) => return Some(command),
            Err(RecvTimeoutError::Timeout) => hold.tick_if_due(sender),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// Waits for the shared playback clock's go signal, holding the tempo while it
/// waits.
///
/// The engine takes a queued `Play` as soon as it arrives, which ends the idle
/// free-run, and then has to wait for every subsystem -- audio device, sample
/// loading, DMX validation -- to report ready. Blocking silently through that
/// window drops the clock for exactly as long as startup takes, at the one
/// boundary `persist_tempo` is supposed to cover. So the wait is done in
/// tick-sized slices with the tempo still running underneath it.
///
/// Without a tempo to hold (persistence off, or no song has established one
/// yet) it sends nothing while it waits. Either way it waits in slices of at
/// most 50 ms, taking any command off the channel between them: a command
/// gives the wait up (see [`heard_command`]). True if one did.
fn wait_for_start_holding_tempo(
    sender: &mut dyn MidiSender,
    play: &BeatClockPlay,
    mut hold: Option<&mut TempoHold>,
    rx: &mpsc::Receiver<BeatClockCommand>,
    pending: &mut Option<BeatClockCommand>,
) -> bool {
    // A command arriving while the start is awaited gives the wait up for
    // it, as one arriving mid-schedule does: a shutdown must not wait on a
    // start that may never come -- stopping the clock for a reopen joins
    // this thread -- and a newer song replaces this one.
    const COMMAND_CHECK: Duration = Duration::from_millis(50);
    loop {
        let wait = hold
            .as_ref()
            .map_or(COMMAND_CHECK, |hold| hold.until_next().min(COMMAND_CHECK));
        if play
            .clock
            .wait_for_start_or_cancel_until(&play.cancel, wait)
        {
            return false;
        }
        if play.cancel.is_cancelled() {
            return false;
        }
        if heard_command(rx, pending) {
            return true;
        }
        if let Some(hold) = hold.as_deref_mut() {
            hold.tick_if_due(sender);
        }
    }
}

/// Takes a command off the channel for the engine loop to act on in place of
/// the next receive, if one has come; a channel closed counts as a shutdown,
/// as it does there. True if there was one.
fn heard_command(
    rx: &mpsc::Receiver<BeatClockCommand>,
    pending: &mut Option<BeatClockCommand>,
) -> bool {
    match rx.try_recv() {
        Ok(command) => *pending = Some(command),
        Err(mpsc::TryRecvError::Disconnected) => *pending = Some(BeatClockCommand::Shutdown),
        Err(mpsc::TryRecvError::Empty) => return false,
    }
    true
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

    /// Runs a section loop jump publisher for a test playback, as
    /// `play_files` runs one for a song without audio: the jumps for the
    /// armed section (on `clock`, from `start_time`) go into the log given
    /// back, until the flag given back is set.
    fn publish_jumps_for_test(
        clock: &PlaybackClock,
        start_time: Duration,
        active_section: &Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
        section_loop_break: &Arc<AtomicBool>,
    ) -> (Arc<crate::playsync::SectionJumpLog>, Arc<AtomicBool>) {
        let mut loop_control = crate::playsync::LoopControl::new();
        loop_control.active_section = active_section.clone();
        loop_control.section_loop_break = section_loop_break.clone();
        let log = loop_control.section_jumps.clone();
        let done = Arc::new(AtomicBool::new(false));
        let publisher = crate::section_loop::JumpPublisher {
            clock: clock.clone(),
            start_time,
            cancel: CancelHandle::new(),
            done: done.clone(),
            audio_gone: None,
            loop_control,
            song: None,
        };
        thread::spawn(move || crate::section_loop::publish_jumps(publisher));
        // The audio this stands in for plays on as long as the notes: the
        // log closes once the section is broken or cleared (the notes then
        // end with their own), or the test is done.
        {
            let log = log.clone();
            let done = done.clone();
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            thread::spawn(move || {
                while !done.load(Ordering::Relaxed)
                    && !section_loop_break.load(Ordering::Relaxed)
                    && active_section.read().is_some()
                {
                    thread::sleep(Duration::from_millis(5));
                }
                // A jump committed just before still falls due.
                thread::sleep(Duration::from_millis(50));
                log.close();
            });
        }
        (log, done)
    }
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
            assert!(device.beat_clock_engine.is_none());
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

    /// A device that cannot be opened afresh fails to reopen, for the
    /// reload to open a new one, and is left as it was, its watch included.
    #[test]
    fn reopen_of_a_device_gone_fails_and_leaves_it() {
        use crate::midi::Device as _;

        let device = Reopenable::with_device(
            config::Midi::new("mtrack-test-no-such-midi-device", None),
            Device::new_default("mtrack-test-no-such-midi-device".to_string()),
        );
        let before = device.current();
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        device.watch_events(tx).unwrap();

        assert!(device.reopen().is_err());
        assert!(Arc::ptr_eq(&before, &device.current()));
        assert!(device.watch.lock().unwrap().is_some());
        device.stop_watch_events();
        assert!(device.watch.lock().unwrap().is_none());
    }

    /// A device whose beat clock holds `tempo`, without the engine's thread
    /// or an output port.
    fn device_holding(name: &str, tempo: Option<Duration>) -> Device {
        let (tx, _) = mpsc::channel();
        let mut device = Device::new_default(name.to_string());
        device.beat_clock_engine = Some(Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(None),
            tempo: Arc::new(Mutex::new(tempo)),
        }));
        device
    }

    /// A device opened afresh started its beat clock knowing no tempo, so
    /// one free-running the tempo held between songs went silent until the
    /// next song. The fresh one is opened holding it.
    /// The fresh device starts its beat clock free-running the held tempo,
    /// so the old one's is stopped before it opens: both at once would send
    /// timing clocks to the same port.
    #[test]
    fn reopen_stops_the_old_beat_clock_before_the_fresh_one_runs() {
        let (tx, rx) = mpsc::channel();
        let mut old = Device::new_default("test".to_string());
        old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(None),
            tempo: Arc::new(Mutex::new(Some(Duration::from_millis(20)))),
        }));
        let device = Reopenable::with_device(config::Midi::new("test", None), old);
        let mut old_running_at_open = None;
        device
            .reopen_with(|tempo| {
                old_running_at_open =
                    Some(!matches!(rx.try_recv(), Ok(BeatClockCommand::Shutdown)));
                Ok(device_holding("test", tempo))
            })
            .unwrap();
        assert_eq!(
            old_running_at_open,
            Some(false),
            "the fresh engine (holding {:?}) started while the old one still ran",
            device.current().held_tempo()
        );
    }

    /// A fresh device that cannot take the watch over, and its beat clock
    /// thread's join, are dropped outside the watch lock.
    #[test]
    fn a_refused_fresh_device_is_dropped_outside_the_watch_lock() {
        use crate::midi::Device as _;
        let reopenable: Arc<std::sync::OnceLock<std::sync::Weak<Reopenable>>> = Arc::default();
        let free = Arc::new(Mutex::new(None));
        let device = Arc::new(Reopenable::with_device(
            config::Midi::new("test", None),
            Device::new_default("test".to_string()),
        ));
        reopenable.set(Arc::downgrade(&device)).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        device.watch_events(tx).unwrap();
        let _ = device.reopen_with(|_| {
            let mut fresh = Device::new_default("test".to_string());
            fresh.refuse_watch = true;
            fresh.on_drop = Some(Box::new({
                let reopenable = reopenable.clone();
                let free = free.clone();
                move || {
                    let device = reopenable.get().and_then(std::sync::Weak::upgrade);
                    *free.lock().unwrap() = device.map(|d| d.watch.try_lock().is_ok());
                }
            }));
            Ok(fresh)
        });
        assert_eq!(
            *free.lock().unwrap(),
            Some(true),
            "dropped under the watch lock"
        );
    }

    /// A reopen whose fresh device cannot be opened has stopped the old
    /// clock already, but the tempo it held stays readable through the
    /// device (`Device::held_tempo`), for the device the reload opens in its
    /// place to go on with.
    #[test]
    fn a_failed_reopen_keeps_the_held_tempo_for_the_device_opened_instead() {
        let (tx, rx) = mpsc::channel();
        let mut old = Device::new_default("test".to_string());
        old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(None),
            tempo: Arc::new(Mutex::new(Some(Duration::from_millis(20)))),
        }));
        let device = Reopenable::with_device(config::Midi::new("test", None), old);
        assert!(device.reopen_with(|_| Err("device gone".into())).is_err());
        assert!(
            matches!(rx.try_recv(), Ok(BeatClockCommand::Shutdown)),
            "the old clock was not stopped before the fresh device was tried"
        );
        assert_eq!(
            crate::midi::Device::held_tempo(&device),
            Some(Duration::from_millis(20))
        );
    }

    /// The playback delay counts from the playback clock's start, as the
    /// notes count it, not from when the engine noticed the start (#600): an
    /// engine told of a song well after its clock started still has its first
    /// clock out a delay after that start.
    #[test]
    fn the_playback_delay_counts_from_the_clocks_start() {
        struct FirstClock {
            clock: PlaybackClock,
            at: Arc<Mutex<Option<Duration>>>,
        }
        impl MidiSender for FirstClock {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == [0xF8] {
                    let mut at = self.at.lock().unwrap();
                    at.get_or_insert(self.clock.elapsed());
                }
                Ok(())
            }
        }
        let clock = PlaybackClock::wall();
        clock.start();
        // The engine hears of the song 600ms into it.
        std::thread::sleep(Duration::from_millis(600));
        let at = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        let cancel = CancelHandle::new();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new((0..48).map(|k| Duration::from_millis(k * 20)).collect()),
            start_time: Duration::ZERO,
            playback_delay: Duration::from_secs(1),
            cancel: cancel.clone(),
            clock: clock.clone(),
            sections: None,
        }))
        .unwrap();
        let join = std::thread::spawn({
            let at = at.clone();
            let clock = clock.clone();
            move || {
                let tempo = Mutex::new(None);
                beat_clock_engine_loop(&mut FirstClock { clock, at }, false, &tempo, &rx);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while at.lock().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        cancel.cancel();
        drop(tx);
        join.join().unwrap();
        let first = at.lock().unwrap().expect("no clock was sent");
        assert!(first >= Duration::from_secs(1), "{first:?}");
        // Counted from the notice, it would be 1.6s in.
        assert!(first < Duration::from_millis(1_450), "{first:?}");
    }

    /// A reopen while a song's beat clock schedule still runs (a clock
    /// outlasting the song's audio) waits for the old clock to stop, so the
    /// fresh one is handed the tempo that schedule set, not the one before.
    #[test]
    fn a_reopen_mid_schedule_carries_the_schedule_tempo() {
        // Counts the timing clocks sent, so the test waits for the schedule
        // to have set its tempo rather than for a time that a slow runner
        // (CI runs these under coverage) need not have used to send two.
        struct Counting(Arc<std::sync::atomic::AtomicUsize>);
        impl MidiSender for Counting {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == [0xF8] {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            }
        }
        let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        // The previous song left 20ms; this schedule ticks every 10ms.
        let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(20))));
        let engine_tempo = tempo.clone();
        let join = std::thread::spawn({
            let sent = sent.clone();
            move || {
                beat_clock_engine_loop(&mut Counting(sent), true, &engine_tempo, &rx);
            }
        });
        let clock = PlaybackClock::wall();
        clock.start();
        let ticks: Vec<Duration> = (0..1000).map(|i| Duration::from_millis(10 * i)).collect();
        let cancel = CancelHandle::new();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new(ticks),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel: cancel.clone(),
            clock,
            sections: None,
        }))
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while sent.load(std::sync::atomic::Ordering::SeqCst) < 3 {
            assert!(
                std::time::Instant::now() < deadline,
                "the schedule never ticked"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut old = Device::new_default("test".to_string());
        old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(Some(join)),
            tempo,
        }));
        let device = Reopenable::with_device(config::Midi::new("test", None), old);
        let mut handed = None;
        device
            .reopen_with(|held| {
                handed = Some(held);
                Ok(device_holding("test", held))
            })
            .unwrap();
        assert_eq!(
            handed,
            Some(Some(Duration::from_millis(10))),
            "the fresh clock was handed the tempo from before the running schedule"
        );
    }

    /// Two reopens racing take turns, so the second stops the clock of the
    /// device the first installs, not of the one the first replaced, and the
    /// port never carries two clocks.
    #[test]
    fn racing_reopens_stop_the_clock_actually_replaced() {
        let device = Arc::new(Reopenable::with_device(
            config::Midi::new("test", None),
            device_holding("old", Some(Duration::from_millis(20))),
        ));
        let (fresh1_tx, fresh1_rx) = mpsc::channel();
        let (opening_tx, opening_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let first = std::thread::spawn({
            let device = device.clone();
            move || {
                device
                    .reopen_with(|held| {
                        opening_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        let mut fresh = Device::new_default("fresh1".to_string());
                        fresh.beat_clock_engine = Some(Arc::new(BeatClockEngine {
                            tx: fresh1_tx,
                            join: Mutex::new(None),
                            tempo: Arc::new(Mutex::new(held)),
                        }));
                        Ok(fresh)
                    })
                    .unwrap();
            }
        });
        // The first holds the watch lock, opening its fresh device.
        opening_rx.recv().unwrap();
        let second = std::thread::spawn({
            let device = device.clone();
            move || {
                let mut fresh1_running = None;
                device
                    .reopen_with(|held| {
                        fresh1_running = Some(!matches!(
                            fresh1_rx.try_recv(),
                            Ok(BeatClockCommand::Shutdown)
                        ));
                        Ok(device_holding("fresh2", held))
                    })
                    .unwrap();
                fresh1_running
            }
        });
        // Let the second reach the watch lock, then let the first finish.
        std::thread::sleep(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        first.join().unwrap();
        assert_eq!(
            second.join().unwrap(),
            Some(false),
            "the second reopen opened its device while the first one's clock still ran"
        );
    }

    /// A reopen that fails mid-schedule has stopped the old clock, and the
    /// schedule has set its tempo by then: that, not the one before it, is
    /// what the device the reload opens instead goes on with.
    #[test]
    fn the_fallback_tempo_is_read_after_the_old_clock_stops() {
        use crate::midi::Device as _;
        struct Null;
        impl MidiSender for Null {
            fn send(&mut self, _: &[u8]) -> Result<(), Box<dyn Error>> {
                Ok(())
            }
        }
        let (tx, rx) = mpsc::channel();
        let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(20))));
        let engine_tempo = tempo.clone();
        let join = std::thread::spawn(move || {
            beat_clock_engine_loop(&mut Null, true, &engine_tempo, &rx);
        });
        let clock = PlaybackClock::wall();
        clock.start();
        let ticks: Vec<Duration> = (0..1000).map(|i| Duration::from_millis(10 * i)).collect();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new(ticks),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel: CancelHandle::new(),
            clock,
            sections: None,
        }))
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut old = Device::new_default("test".to_string());
        old.beat_clock_engine = Some(Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(Some(join)),
            tempo,
        }));
        let device = Reopenable::with_device(config::Midi::new("test", None), old);
        // As the reload's MIDI phase does it: the reopen, then the tempo for
        // the device opened instead, once the old clock has stopped.
        assert!(device.reopen_with(|_| Err("device gone".into())).is_err());
        assert_eq!(
            device.held_tempo(),
            Some(Duration::from_millis(10)),
            "the fallback would be handed the tempo from before the running schedule"
        );
    }

    /// A schedule waiting on a go signal that never comes, and is never
    /// cancelled, still hears a stop: stopping the clock for a reopen joins
    /// its thread, and would otherwise never return.
    #[test]
    fn stop_returns_while_a_schedule_waits_for_its_start() {
        struct Null;
        impl MidiSender for Null {
            fn send(&mut self, _: &[u8]) -> Result<(), Box<dyn Error>> {
                Ok(())
            }
        }
        let (tx, rx) = mpsc::channel();
        let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(20))));
        let engine_tempo = tempo.clone();
        let join = std::thread::spawn(move || {
            beat_clock_engine_loop(&mut Null, true, &engine_tempo, &rx);
        });
        let cancel = CancelHandle::new();
        tx.send(BeatClockCommand::Play(BeatClockPlay {
            ticks: Arc::new(vec![Duration::ZERO]),
            start_time: Duration::ZERO,
            playback_delay: Duration::ZERO,
            cancel: cancel.clone(),
            clock: PlaybackClock::wall(),
            sections: None,
        }))
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let engine = Arc::new(BeatClockEngine {
            tx,
            join: Mutex::new(Some(join)),
            tempo,
        });
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn({
            let engine = engine.clone();
            move || {
                engine.stop();
                let _ = done_tx.send(());
            }
        });
        let stopped = done_rx.recv_timeout(Duration::from_secs(1)).is_ok();
        cancel.cancel(); // let the thread go either way
        assert!(stopped, "stop() hung on a schedule waiting for its start");
    }

    #[test]
    fn reopen_carries_the_held_tempo_over() {
        let held = Duration::from_millis(20);
        let device = Reopenable::with_device(
            config::Midi::new("test", None),
            device_holding("test", Some(held)),
        );
        let mut given = None;
        device
            .reopen_with(|tempo| {
                given = Some(tempo);
                Ok(device_holding("test", tempo))
            })
            .unwrap();
        assert_eq!(given, Some(Some(held)));
        assert_eq!(device.current().held_tempo(), Some(held));
    }

    /// The device a reopen replaces was dropped under the device and watch
    /// locks, so its beat clock's thread was joined holding both. It is let
    /// go of once they are released.
    #[test]
    fn reopen_drops_the_old_device_outside_its_locks() {
        let reopenable: Arc<std::sync::OnceLock<std::sync::Weak<Reopenable>>> = Arc::default();
        let free = Arc::new(Mutex::new(None));
        let mut old = Device::new_default("test".to_string());
        old.on_drop = Some(Box::new({
            let reopenable = reopenable.clone();
            let free = free.clone();
            move || {
                let device = reopenable.get().and_then(std::sync::Weak::upgrade);
                *free.lock().unwrap() = device.map(|device| {
                    device.device.try_write().is_ok() && device.watch.try_lock().is_ok()
                });
            }
        }));
        let device = Arc::new(Reopenable::with_device(
            config::Midi::new("test", None),
            old,
        ));
        reopenable.set(Arc::downgrade(&device)).unwrap();

        device
            .reopen_with(|_| Ok(Device::new_default("test".to_string())))
            .unwrap();
        assert_eq!(*free.lock().unwrap(), Some(true), "dropped under its locks");
    }

    /// A reopen whose fresh device could not take over the watch returned
    /// the error with the sender still kept, so the controller read as
    /// running while nothing watched. The sender is dropped, and the
    /// controller hears its channel close.
    #[test]
    fn reopen_that_cannot_watch_lets_the_watch_go() {
        use crate::midi::Device as _;

        let device = Reopenable::with_device(
            config::Midi::new("test", None),
            Device::new_default("test".to_string()),
        );
        let before = device.current();
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        device.watch_events(tx).unwrap();

        let result = device.reopen_with(|_| {
            let mut fresh = Device::new_default("test".to_string());
            fresh.refuse_watch = true;
            Ok(fresh)
        });
        assert!(result.is_err());
        assert!(Arc::ptr_eq(&before, &device.current()));
        assert!(device.watch.lock().unwrap().is_none(), "the watch was kept");
        assert_eq!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        );
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
                crate::playsync::PlaybackSync {
                    cancel_handle: cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    clock,
                    start_time: Duration::ZERO,
                    loop_control: crate::playsync::LoopControl::new(),
                },
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
                crate::playsync::PlaybackSync {
                    cancel_handle: cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    clock,
                    start_time: Duration::ZERO,
                    loop_control: crate::playsync::LoopControl::new(),
                },
            );
            assert!(result.is_ok());
        }

        /// A song with a tempo and no MIDI file still drives the beat clock:
        /// its schedule goes to the engine, and the song is ready at once.
        #[test]
        fn play_from_hands_a_tempo_only_songs_clock_to_the_engine() {
            let (tx, rx) = mpsc::channel();
            let mut device = Device::new_default("test".to_string());
            device.beat_clock_enabled = true;
            device.beat_clock_engine = Some(Arc::new(BeatClockEngine {
                tx,
                join: Mutex::new(None),
                tempo: Arc::new(Mutex::new(None)),
            }));
            let song = Arc::new(crate::songs::Song::new_for_test_with_tempo(
                "tempo only",
                &["click"],
                Duration::from_secs(1),
                120.0,
            ));
            assert!(song.midi_playback().is_none());

            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                crate::playsync::PlaybackSync {
                    cancel_handle: CancelHandle::new(),
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    clock: PlaybackClock::wall(),
                    start_time: Duration::from_millis(250),
                    loop_control: crate::playsync::LoopControl::new(),
                },
            );
            assert!(result.is_ok(), "{result:?}");
            assert!(ready_rx.try_recv().is_ok(), "the song was never ready");
            let Ok(BeatClockCommand::Play(play)) = rx.try_recv() else {
                panic!("no schedule reached the engine");
            };
            assert_eq!(play.ticks.len(), 48);
            assert_eq!(
                play.ticks[1] - play.ticks[0],
                Duration::from_nanos(20_833_333)
            );
            assert_eq!(play.start_time, Duration::from_millis(250));
        }

        /// Without the beat clock, a song with no MIDI file has nothing for
        /// the MIDI device to do.
        #[test]
        fn play_from_ignores_a_tempo_only_song_without_a_beat_clock() {
            let device = Device::new_default("test".to_string());
            let song = Arc::new(crate::songs::Song::new_for_test_with_tempo(
                "tempo only",
                &["click"],
                Duration::from_secs(1),
                120.0,
            ));
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                crate::playsync::PlaybackSync {
                    cancel_handle: CancelHandle::new(),
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    clock: PlaybackClock::wall(),
                    start_time: Duration::ZERO,
                    loop_control: crate::playsync::LoopControl::new(),
                },
            );
            assert!(result.is_ok());
            assert!(ready_rx.try_recv().is_ok());
        }

        #[test]
        fn play_from_without_midi_playback_sends_ready() {
            let device = Device::new_default("test".to_string());
            let (_tmp_dir, song) = make_song();
            assert!(song.midi_playback().is_none());

            let cancel = CancelHandle::new();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = PlaybackClock::wall();
            let result = <Device as crate::midi::Device>::play_from(
                &device,
                song,
                crate::playsync::PlaybackSync {
                    cancel_handle: cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    clock,
                    start_time: Duration::ZERO,
                    loop_control: crate::playsync::LoopControl::new(),
                },
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

        /// Helper: run play_streams with common defaults for tests.
        fn run_play(
            midi: &PrecomputedMidi,
            start: Duration,
            sender: &mut MockSender,
            cancel: &CancelHandle,
            exclude: &HashSet<u8>,
            clock: &PlaybackClock,
        ) {
            // The notes keep to the clock: running.
            if !clock.has_started() {
                clock.start();
            }
            // No loops: the log is closed from the start.
            let log = Arc::new(crate::playsync::SectionJumpLog::default());
            log.close();
            play_streams(
                &NoteStreams {
                    precomputed: midi,
                    start_time: start,
                    first_clock_base: clock.elapsed(),
                    cancel_handle: cancel,
                    loop_break: &AtomicBool::new(false),
                    active_section: &parking_lot::RwLock::new(None),
                    exclude_channels: exclude,
                    clock,
                },
                &mut crate::playsync::JumpCursor::new(log),
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

        /// Records when on the clock each note was sent, and which; the
        /// note-offs apart.
        struct TimedSender {
            clock: PlaybackClock,
            sent: Vec<(f64, u8)>,
            offs: Vec<(f64, u8)>,
        }

        impl TimedSender {
            fn new(clock: &PlaybackClock) -> Self {
                Self {
                    clock: clock.clone(),
                    sent: Vec::new(),
                    offs: Vec::new(),
                }
            }
        }

        impl MidiSender for TimedSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                let at = self.clock.elapsed().as_secs_f64();
                if bytes[0] & 0xF0 == 0x80 {
                    self.offs.push((at, bytes[1]));
                } else {
                    self.sent.push((at, bytes[1]));
                }
                Ok(())
            }
        }

        /// A looping song's passes as the audio commits them: a jump at each
        /// of `at_ms`, from `from_ms` back to the top, with `overlap_ms`. The
        /// log is closed: no more come.
        fn passes(
            at_ms: &[u64],
            from_ms: u64,
            overlap_ms: u64,
        ) -> Arc<crate::playsync::SectionJumpLog> {
            let log = Arc::new(crate::playsync::SectionJumpLog::default());
            for &at in at_ms {
                log.push(crate::section_loop::SectionJump {
                    at: Duration::from_millis(at),
                    from: Duration::from_millis(from_ms),
                    to: Duration::ZERO,
                    overlap: Duration::from_millis(overlap_ms),
                    pass: true,
                });
            }
            log.close();
            log
        }

        /// A crossfading loop's passes overlap: at each pass jump in the log,
        /// the pass it leaves plays on for the overlap, its notes in time
        /// with the new pass's, older pass first at the same moment.
        #[test]
        fn crossfading_loop_overlaps_its_passes() {
            // A 1 s song with a 0.4 s overlap: a pass every 0.6 s, notes
            // 60, 61, 62 at 0, 0.3 and 0.9 s into each.
            let midi = make_events(&[0, 300, 900]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let loop_break = Arc::new(AtomicBool::new(false));
            let breaker = {
                let loop_break = loop_break.clone();
                let clock = clock.clone();
                std::thread::spawn(move || {
                    while clock.elapsed() < Duration::from_millis(1_650) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    loop_break.store(true, Ordering::Relaxed);
                })
            };

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: loop_break.clone(),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[600, 1_200], 600, 400),
                },
            );
            breaker.join().unwrap();

            // Pass 0 at 0, pass 1 at 0.6, pass 2 at 1.2: the first pass's
            // last note (0.9) lands with the second's second (0.6 + 0.3).
            let expected = [
                (0.0, 60),
                (0.3, 61),
                (0.6, 60),
                (0.9, 62),
                (0.9, 61),
                (1.2, 60),
                (1.5, 62),
                (1.5, 61),
            ];
            let sent = &sender.sent;
            assert_eq!(sent.len(), expected.len(), "{sent:?}");
            for ((at, note), (want_at, want_note)) in sent.iter().zip(expected) {
                assert!((at - want_at).abs() < 0.03, "{sent:?}");
                assert_eq!(*note, want_note, "{sent:?}");
            }
        }

        /// A plain loop's passes (no overlap) start as the log says: on the
        /// song's length, not when its MIDI runs out.
        #[test]
        fn a_loop_restarts_on_the_songs_length_not_its_midis() {
            // A 0.5 s song whose MIDI is done by 0.1 s: each pass starts on
            // the half second, as the audio's does, rather than as soon as
            // the last note has played.
            let midi = make_events(&[0, 100]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let loop_break = Arc::new(AtomicBool::new(false));
            let breaker = {
                let loop_break = loop_break.clone();
                let clock = clock.clone();
                std::thread::spawn(move || {
                    while clock.elapsed() < Duration::from_millis(1_250) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    loop_break.store(true, Ordering::Relaxed);
                })
            };

            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: loop_break.clone(),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[500, 1_000], 500, 0),
                },
            );
            breaker.join().unwrap();

            let expected = [
                (0.0, 60),
                (0.1, 61),
                (0.5, 60),
                (0.6, 61),
                (1.0, 60),
                (1.1, 61),
            ];
            let sent = &sender.sent;
            assert_eq!(sent.len(), expected.len(), "{sent:?}");
            // Timed from the first note: wall-clock playback on a loaded
            // runner (CI runs these under coverage) starts tens of
            // milliseconds late. Restarting on the MIDI's length instead
            // would put the second and third passes 400 ms early.
            let first = sent[0].0;
            for ((at, note), (want_at, want_note)) in sent.iter().zip(expected) {
                assert!((at - first - want_at).abs() < 0.1, "{sent:?}");
                assert_eq!(*note, want_note, "{sent:?}");
            }
        }

        /// A looping song's pass plays all of its own notes: one on the
        /// pass's end (a MIDI-only song's length is its last event, and a
        /// DAW export's last note-off lands on the audio's end) or past it
        /// (a release tail) still goes out, as `play_looping_passes` and the
        /// restart loop sent it; dropping it leaves the note hanging every
        /// pass.
        #[test]
        fn a_pass_plays_its_notes_on_and_past_its_end() {
            // A 200 ms song looping without a crossfade, its MIDI 60 at 0,
            // 61 at 100, 62 on the end (200) and 63 past it (250).
            let midi = make_events(&[0, 100, 200, 250]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[200], 200, 0),
                },
            );
            let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
            // Each of the two passes sends 62 and 63 once.
            assert_eq!(notes.iter().filter(|n| **n == 62).count(), 2, "{notes:?}");
            assert_eq!(notes.iter().filter(|n| **n == 63).count(), 2, "{notes:?}");
        }

        /// A section's jump cuts the run of notes it leaves: the notes that
        /// run has on are let go there, rather than left hanging.
        #[test]
        fn a_section_jump_lets_go_of_the_notes_it_cuts() {
            // Notes 60 at 0 and 61 at 100, both held; the section 0-300
            // jumps back once, at 300, and the song ends at 400.
            let midi = make_events(&[0, 100]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let log = Arc::new(crate::playsync::SectionJumpLog::default());
            log.push(crate::section_loop::SectionJump {
                at: Duration::from_millis(300),
                from: Duration::from_millis(300),
                to: Duration::ZERO,
                overlap: Duration::ZERO,
                pass: false,
            });
            log.close();
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: log,
                },
            );
            let notes: Vec<u8> = sender.sent.iter().map(|(_, n)| *n).collect();
            assert_eq!(notes, vec![60, 61, 60, 61]);
            let mut offs: Vec<u8> = sender.offs.iter().map(|(_, n)| *n).collect();
            offs.sort();
            assert_eq!(offs, vec![60, 61], "let go at the cut, once each");
            for (at, _) in &sender.offs {
                assert!(*at >= 0.29, "not before the cut: {at}");
            }
        }

        #[test]
        fn an_armed_section_loops_its_midi_in_a_looping_song() {
            // A 1 s looping song with the section 0-0.3 s armed: its notes at
            // 0 and 0.1 s repeat every 0.3 s. The first play stops at the
            // note past the section (0.5 s), long before the clock reaches
            // the section end; MIDI used to take that as the end of the song.
            let midi = make_events(&[0, 100, 500]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let loop_break = Arc::new(AtomicBool::new(false));
            let breaker = {
                let loop_break = loop_break.clone();
                let clock = clock.clone();
                std::thread::spawn(move || {
                    while clock.elapsed() < Duration::from_millis(750) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    loop_break.store(true, Ordering::Relaxed);
                })
            };

            let active_section = Arc::new(parking_lot::RwLock::new(Some(
                crate::player::SectionBounds {
                    name: "intro".to_string(),
                    start_time: Duration::ZERO,
                    end_time: Duration::from_millis(300),
                },
            )));
            let section_loop_break = Arc::new(AtomicBool::new(false));
            let (log, done) = publish_jumps_for_test(
                &clock,
                Duration::ZERO,
                &active_section,
                &section_loop_break,
            );
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: loop_break.clone(),
                    active_section,
                    section_jumps: log,
                },
            );
            breaker.join().unwrap();
            done.store(true, Ordering::Relaxed);

            let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
            assert_eq!(notes, vec![60, 61, 60, 61, 60, 61], "{:?}", sender.sent);
            // The repeats come on the section's grid, from the clock's start:
            // 0.3 s and 0.6 s, where looping on the first play's end would put
            // them at 0.5 s and 1.0 s. A loaded runner (CI runs under
            // coverage) can send any note late, the first included, so the
            // bounds only rule that out; timing them from the first note
            // failed when it alone was late.
            let at = |i: usize| sender.sent[i].0;
            assert!(at(2) >= 0.28 && at(2) < 0.48, "{:?}", sender.sent);
            assert!(at(4) >= 0.58 && at(4) < 0.95, "{:?}", sender.sent);
        }

        /// Once a section loop is broken (as `stop_section_loop` breaks it:
        /// the break set, the section cleared), the notes play on from the
        /// section's end, as the audio and beat clock do; they used to stop
        /// there for good.
        #[test]
        fn notes_play_on_after_a_section_loop_is_broken() {
            // Notes at 0, 100 and 400ms; the section 0-300ms loops once
            // (300ms), is broken at 350ms, and the song goes on: the note at
            // 400ms comes at 700ms.
            let midi = make_events(&[0, 100, 400]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let active_section = Arc::new(parking_lot::RwLock::new(Some(
                crate::player::SectionBounds {
                    name: "intro".to_string(),
                    start_time: Duration::ZERO,
                    end_time: Duration::from_millis(300),
                },
            )));
            let section_loop_break = Arc::new(AtomicBool::new(false));
            let breaker = {
                let active_section = active_section.clone();
                let section_loop_break = section_loop_break.clone();
                let clock = clock.clone();
                std::thread::spawn(move || {
                    while clock.elapsed() < Duration::from_millis(350) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    section_loop_break.store(true, Ordering::Relaxed);
                    *active_section.write() = None;
                })
            };
            let (log, done) = publish_jumps_for_test(
                &clock,
                Duration::ZERO,
                &active_section,
                &section_loop_break,
            );
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section,
                    section_jumps: log,
                },
            );
            breaker.join().unwrap();
            done.store(true, Ordering::Relaxed);
            let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
            assert_eq!(notes, vec![60, 61, 60, 61, 62], "{:?}", sender.sent);
        }

        /// Runs `run_playback` over notes at `at_ms` (60, 61, ...) with
        /// `section` armed from the top, while `driver` changes the section
        /// loop state on the clock; returns the notes sent.
        fn play_with_sections(
            at_ms: &[u64],
            section: (u64, u64),
            driver: impl FnOnce(
                    PlaybackClock,
                    Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
                    Arc<AtomicBool>,
                ) + Send
                + 'static,
        ) -> Vec<(f64, u8)> {
            let midi = make_events(at_ms);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let active_section = Arc::new(parking_lot::RwLock::new(Some(
                crate::player::SectionBounds {
                    name: "a".to_string(),
                    start_time: Duration::from_millis(section.0),
                    end_time: Duration::from_millis(section.1),
                },
            )));
            let section_loop_break = Arc::new(AtomicBool::new(false));
            let driver = {
                let clock = clock.clone();
                let active_section = active_section.clone();
                let section_loop_break = section_loop_break.clone();
                std::thread::spawn(move || driver(clock, active_section, section_loop_break))
            };
            let (log, done) = publish_jumps_for_test(
                &clock,
                Duration::ZERO,
                &active_section,
                &section_loop_break,
            );
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section,
                    section_jumps: log,
                },
            );
            driver.join().unwrap();
            done.store(true, Ordering::Relaxed);
            sender.sent
        }

        /// While the log is open the notes follow it past their last note: a
        /// section armed after it, its jump committed then, plays them again
        /// (#606); they end once the log closes. They used to end with their
        /// last note.
        #[test]
        fn notes_replay_after_their_last_note_while_the_log_is_open() {
            let midi = make_events(&[0, 50]);
            let cancel = CancelHandle::new();
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let exclude = HashSet::new();
            let clock = PlaybackClock::wall();
            clock.start();
            let mut sender = TimedSender::new(&clock);
            let log = Arc::new(crate::playsync::SectionJumpLog::default());
            let audio = {
                let log = log.clone();
                let clock = clock.clone();
                std::thread::spawn(move || {
                    // The section 0-300ms armed at 200ms, past the last note:
                    // its jump at its end.
                    wait_until(&clock, 200);
                    log.push(crate::section_loop::SectionJump {
                        at: Duration::from_millis(300),
                        from: Duration::from_millis(300),
                        to: Duration::ZERO,
                        overlap: Duration::ZERO,
                        pass: false,
                    });
                    // Broken, and the song over: the log closes.
                    wait_until(&clock, 450);
                    log.close();
                })
            };
            run_playback(
                &mut sender,
                PlaybackContext {
                    precomputed: &midi,
                    start_time: Duration::ZERO,
                    playback_delay: Duration::ZERO,
                    cancel_handle: &cancel,
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: Arc::new(AtomicBool::new(false)),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: log.clone(),
                },
            );
            audio.join().unwrap();
            assert!(log.is_closed(), "the notes ended before the log closed");
            let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
            assert_eq!(notes, vec![60, 61, 60, 61], "{:?}", sender.sent);
        }

        /// A section armed in a song looping with a crossfade loops on its
        /// own seam, the notes with it, whether it reaches into the
        /// crossfade's tail or ends on the song's end: the whole-song pass
        /// never starts under it (it used to be refused in such a song). The
        /// jumps come from the playback's publisher, deciding as the audio
        /// does.
        #[test]
        fn a_section_in_a_crossfading_song_loops_without_a_pass_under_it() {
            // A 1s song crossfading over its last 0.4s (passes would start
            // at 0.6s); notes at 0, 0.45, 0.55 and 0.65s.
            for (start, end) in [(400, 700), (500, 1_000)] {
                let midi = make_events(&[0, 450, 550, 650]);
                let cancel = CancelHandle::new();
                let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
                let exclude = HashSet::new();
                let clock = PlaybackClock::wall();
                clock.start();
                let mut sender = TimedSender::new(&clock);
                let loop_control = crate::playsync::LoopControl::new();
                *loop_control.active_section.write() = Some(crate::player::SectionBounds {
                    name: "a".to_string(),
                    start_time: Duration::from_millis(start),
                    end_time: Duration::from_millis(end),
                });
                let log = loop_control.section_jumps.clone();
                let loop_break = loop_control.loop_break.clone();
                let done = Arc::new(AtomicBool::new(false));
                let song = Arc::new(crate::songs::Song::new_for_test_crossfading(
                    "Crossfading",
                    Duration::from_secs(1),
                    Duration::from_millis(400),
                ));
                let publisher = crate::section_loop::JumpPublisher {
                    clock: clock.clone(),
                    start_time: Duration::ZERO,
                    cancel: cancel.clone(),
                    done: done.clone(),
                    audio_gone: None,
                    loop_control: loop_control.clone(),
                    song: Some(song),
                };
                let publisher =
                    thread::spawn(move || crate::section_loop::publish_jumps(publisher));
                let breaker = {
                    let loop_break = loop_break.clone();
                    let clock = clock.clone();
                    thread::spawn(move || {
                        wait_until(&clock, 2_300);
                        loop_break.store(true, Ordering::Relaxed);
                    })
                };
                run_playback(
                    &mut sender,
                    PlaybackContext {
                        precomputed: &midi,
                        start_time: Duration::ZERO,
                        playback_delay: Duration::ZERO,
                        cancel_handle: &cancel,
                        ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                        finished: Arc::new(AtomicBool::new(false)),
                        exclude_channels: &exclude,
                        clock: &clock,
                        loop_break,
                        active_section: loop_control.active_section.clone(),
                        section_jumps: log.clone(),
                    },
                );
                breaker.join().unwrap();
                done.store(true, Ordering::Relaxed);
                publisher.join().unwrap();
                let mut cursor = crate::playsync::JumpCursor::new(log);
                let mut jumps = 0;
                while let Some(jump) = cursor.due(Duration::MAX) {
                    jumps += 1;
                    assert_eq!(
                        (jump.to, jump.overlap),
                        (Duration::from_millis(start), Duration::ZERO),
                        "section {start}-{end}ms: a pass under it: {jump:?}"
                    );
                }
                assert!(jumps >= 2, "section {start}-{end}ms: {jumps} jumps");
                let notes: Vec<u8> = sender.sent.iter().map(|(_, note)| *note).collect();
                // The top's note once, never again (no pass); the section's
                // notes round and round.
                assert_eq!(
                    notes.iter().filter(|note| **note == 60).count(),
                    1,
                    "section {start}-{end}ms: {notes:?}"
                );
                assert!(
                    notes.iter().filter(|note| **note == 62).count() >= 3,
                    "section {start}-{end}ms: {notes:?}"
                );
            }
        }

        fn wait_until(clock: &PlaybackClock, ms: u64) {
            while clock.elapsed() < Duration::from_millis(ms) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        /// Broken before it ever went round (as `stop_section_loop` breaks
        /// it), the section's end is played through, as the audio plays it:
        /// the notes past it still come.
        #[test]
        fn notes_play_on_after_a_section_broken_before_it_loops() {
            // Notes at 0, 100 and 400ms; section 0-300ms armed, broken at
            // 200ms: the note at 400ms is still due.
            let sent = play_with_sections(&[0, 100, 400], (0, 300), |clock, active, brk| {
                wait_until(&clock, 200);
                brk.store(true, Ordering::Relaxed);
                *active.write() = None;
            });
            let notes: Vec<u8> = sent.iter().map(|(_, note)| *note).collect();
            assert_eq!(notes, vec![60, 61, 62], "{sent:?}");
        }

        /// Another section armed while one loops (no break): the audio plays
        /// the rest of the song up to the new section's end; so do the notes.
        #[test]
        fn notes_play_on_into_another_section_armed_while_one_loops() {
            // A = 0-300ms, B = 400-600ms. A loops at 300ms; at 350ms B is
            // armed. The song plays on from A's end (clock 600ms) through
            // B, whose notes (400, 500) come at 700 and 800ms, and B goes
            // round at 900ms (notes again at 900, 1000). Broken at 950ms;
            // the note at 700ms in the song comes at 1200ms.
            let sent =
                play_with_sections(&[0, 100, 400, 500, 700], (0, 300), |clock, active, brk| {
                    wait_until(&clock, 350);
                    *active.write() = Some(crate::player::SectionBounds {
                        name: "b".to_string(),
                        start_time: Duration::from_millis(400),
                        end_time: Duration::from_millis(600),
                    });
                    wait_until(&clock, 950);
                    brk.store(true, Ordering::Relaxed);
                    *active.write() = None;
                });
            let notes: Vec<u8> = sent.iter().map(|(_, note)| *note).collect();
            assert_eq!(notes, vec![60, 61, 60, 61, 62, 63, 62, 63, 64], "{sent:?}");
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
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
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
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
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
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
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
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
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
                    ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                    finished: finished.clone(),
                    exclude_channels: &exclude,
                    clock: &clock,
                    loop_break: Arc::new(AtomicBool::new(false)),
                    active_section: Arc::new(parking_lot::RwLock::new(None)),
                    section_jumps: passes(&[], 0, 0),
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
                        ready_tx: crate::playsync::ReadyGuard::new(ready_tx),
                        finished: finished_clone,
                        exclude_channels: &exclude,
                        clock: &clock_clone,
                        loop_break: Arc::new(AtomicBool::new(false)),
                        active_section: Arc::new(parking_lot::RwLock::new(None)),
                        section_jumps: passes(&[], 0, 0),
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

        /// Records each clock as the song time it was due at, and breaks
        /// the loop once it has sent `break_after` of them.
        struct LoopingSender {
            clock: PlaybackClock,
            clocks: Vec<Duration>,
            break_after: usize,
            loop_break: Arc<AtomicBool>,
        }

        impl MidiSender for LoopingSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                if bytes == clock_bytes().as_slice() {
                    // A clock that ignored the break would go round for ever.
                    assert!(
                        self.clocks.len() <= self.break_after,
                        "the loop break was not heard"
                    );
                    self.clocks.push(self.clock.elapsed());
                    if self.clocks.len() == self.break_after {
                        self.loop_break.store(true, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
        }

        fn every_10ms(until_ms: u64) -> Vec<Duration> {
            (0..until_ms / 10)
                .map(|i| Duration::from_millis(i * 10))
                .collect()
        }

        fn ms(ms: u64) -> Duration {
            Duration::from_millis(ms)
        }

        /// Start and the first clock are a millisecond apart, as the MIDI spec
        /// asks, though the first is due at once.
        #[test]
        fn the_first_clock_follows_start_by_a_millisecond() {
            struct Timed {
                started: Option<std::time::Instant>,
                first_clock: Option<Duration>,
            }
            impl MidiSender for Timed {
                fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                    if bytes == realtime_bytes(SystemRealtime::Start).as_slice() {
                        self.started = Some(std::time::Instant::now());
                    } else if bytes == clock_bytes().as_slice() && self.first_clock.is_none() {
                        self.first_clock = self.started.map(|at| at.elapsed());
                    }
                    Ok(())
                }
            }
            let mut sender = Timed {
                started: None,
                first_clock: None,
            };
            let clock = PlaybackClock::wall();
            clock.start();
            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                &[Duration::ZERO, ms(10)],
                Duration::ZERO,
                None,
                None,
                &CancelHandle::new(),
                &clock,
                &rx,
            );
            assert!(
                sender.first_clock.unwrap() >= START_TO_CLOCK,
                "{:?}",
                sender.first_clock
            );
        }

        /// Plays `ticks` following the pass jumps of a looping song given in
        /// a closed log; the clocks that went out, on the playback clock.
        fn play_passes(
            ticks: &[Duration],
            jumps: &[(u64, u64)],
            break_after: usize,
        ) -> Vec<Duration> {
            let clock = PlaybackClock::wall();
            clock.start();
            let log = Arc::new(crate::playsync::SectionJumpLog::default());
            for &(at, overlap) in jumps {
                log.push(crate::section_loop::SectionJump {
                    at: ms(at),
                    from: ms(400),
                    to: Duration::ZERO,
                    overlap: ms(overlap),
                    pass: false,
                });
            }
            log.close();
            let loop_break = Arc::new(AtomicBool::new(false));
            let sections = BeatClockSections {
                active_section: Default::default(),
                section_loop_break: Default::default(),
                jumps: log,
                loop_break: loop_break.clone(),
                loops: true,
            };
            let mut sender = LoopingSender {
                clock: clock.clone(),
                clocks: Vec::new(),
                break_after,
                loop_break,
            };
            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                ticks,
                Duration::ZERO,
                Some(&sections),
                None,
                &CancelHandle::new(),
                &clock,
                &rx,
            );
            sender.clocks
        }

        /// A looping song's passes are jumps in the log, which the clock
        /// follows as it does a section's: back to the top at each, its
        /// pulses switching to the new pass there, the overlap no matter.
        /// (It used to keep a pass grid of its own.)
        #[test]
        fn the_clock_follows_a_looping_songs_passes_from_the_log() {
            // 500ms of clocks; passes start 100ms before the end (at 400ms
            // and 800ms on the clock), crossfading over the last 100ms.
            let clocks = play_passes(&every_10ms(500), &[(400, 100), (800, 100)], 1_000);
            // 0-390ms, twice, then the last pass to its end.
            assert_eq!(clocks.len(), 40 + 40 + 50, "{clocks:?}");
            // The seam is a pulse, not two at once.
            assert!(clocks[40] >= ms(400) && clocks[40] - clocks[39] >= ms(5));
            assert!(clocks[80] >= ms(800) && clocks[80] - clocks[79] >= ms(5));
        }

        /// DIAGNOSTIC (no assertion; prints timing): a plain loop's pass jump
        /// lies past the last pulse (pulses stop short of the song's end), so
        /// the clock reaches it through the out-of-ticks wait, which polls the
        /// wall clock every 5ms instead of sleeping until the jump's `at`.
        #[test]
        #[ignore]
        fn diagnostic_first_pulse_of_a_plain_loops_pass() {
            // 120 BPM: a pulse every 20.833ms; a 500ms song, 24 pulses.
            let ticks: Vec<Duration> = (0..24)
                .map(|i| Duration::from_nanos(i * 500_000_000 / 24))
                .collect();
            let jumps: Vec<(u64, u64)> = (1..=10).map(|k| (k * 500, 0)).collect();
            let clocks = play_passes(&ticks, &jumps, 24 * 11);
            let late: Vec<Duration> = (1..=10)
                .map(|k| clocks[24 * k].saturating_sub(ms(500 * k as u64)))
                .collect();
            let mid: Vec<Duration> = (1..=10)
                .map(|k| clocks[24 * k + 12].saturating_sub(ms(500 * k as u64 + 250)))
                .collect();
            eprintln!("seam lateness {late:?}\nmid-pass lateness {mid:?}");
        }

        /// A broken loop ends the clock with a Stop, as the player ends the
        /// song with it.
        #[test]
        fn a_looping_clock_runs_across_seams_until_the_loop_is_broken() {
            let clocks = play_passes(&every_10ms(500), &[(400, 0), (800, 0)], 55);
            assert_eq!(clocks.len(), 55);
        }

        /// A loop of whole pulses that comes out a hair long puts no pulse
        /// against the next pass's first.
        #[test]
        fn a_pass_a_hair_long_keeps_every_seam_a_pulse() {
            // Ticks every 10ms to 400ms inclusive: the pass jumps at 400ms
            // from 400ms, a hair after its last pulse.
            let mut ticks = every_10ms(400);
            ticks.push(Duration::from_micros(399_990));
            let clocks = play_passes(&ticks, &[(400, 0)], 1_000);
            assert_eq!(clocks.len(), 40 + 41, "{clocks:?}");
        }

        fn section(start: u64, end: u64) -> (BeatClockSections, Arc<AtomicBool>) {
            let section_loop_break = Arc::new(AtomicBool::new(false));
            (
                BeatClockSections {
                    active_section: Arc::new(parking_lot::RwLock::new(Some(
                        crate::player::SectionBounds {
                            name: "loop".to_string(),
                            start_time: ms(start),
                            end_time: ms(end),
                        },
                    ))),
                    section_loop_break: section_loop_break.clone(),
                    jumps: Default::default(),
                    loop_break: Default::default(),
                    loops: false,
                },
                section_loop_break,
            )
        }

        /// Plays `ticks` from `start` with a section loop, breaking it once
        /// `break_after` clocks have gone out; how many went out in all.
        fn play_section(
            ticks: &[Duration],
            start: u64,
            sections: &BeatClockSections,
            loop_break: Arc<AtomicBool>,
            break_after: usize,
        ) -> usize {
            /// Breaks the section loop at the `break_after`th clock; a clock
            /// that never stopped looping fails rather than hangs.
            struct SectionSender {
                sent: usize,
                break_after: usize,
                section_loop_break: Arc<AtomicBool>,
            }
            impl MidiSender for SectionSender {
                fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                    if bytes == clock_bytes().as_slice() {
                        self.sent += 1;
                        assert!(self.sent < 1_000, "the section break was not heard");
                        if self.sent == self.break_after {
                            self.section_loop_break.store(true, Ordering::Relaxed);
                        }
                    }
                    Ok(())
                }
            }
            let clock = PlaybackClock::wall();
            clock.start();
            // The jumps a test gives stand in for the audio's; otherwise the
            // playback's own publisher commits them.
            let given = crate::playsync::JumpCursor::new(sections.jumps.clone())
                .peek()
                .is_some();
            let (log, done) = publish_jumps_for_test(
                &clock,
                ms(start),
                &sections.active_section,
                &sections.section_loop_break,
            );
            let mut sections = sections.clone();
            if !given {
                sections.jumps = log;
            }
            let sections = &sections;
            let mut sender = SectionSender {
                sent: 0,
                break_after,
                section_loop_break: loop_break,
            };
            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                ticks,
                ms(start),
                Some(sections),
                None,
                &CancelHandle::new(),
                &clock,
                &rx,
            );
            done.store(true, Ordering::Relaxed);
            sender.sent
        }

        /// A section loop takes the clock back to the section's start at its
        /// end, as it takes the notes and audio, until it is broken; then the
        /// clock plays on from the section's end (#604).
        #[test]
        fn a_section_loop_takes_the_clock_back_until_it_is_broken() {
            // 10ms clocks to 500ms; the section is 100-300ms. 30 clocks to
            // its end, then round from 100ms; broken at the 40th clock (at
            // 190ms, the second time round: well before the next jump is
            // committed, 30ms ahead of it, as the audio commits), then on
            // from there to the end: 30 more.
            let (sections, section_loop_break) = section(100, 300);
            let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 40);
            assert_eq!(sent, 30 + 10 + 30);
        }

        /// A section armed after the clock has passed its end goes back at
        /// once, as the notes do.
        #[test]
        fn a_section_armed_past_its_end_goes_back_at_once() {
            // Started at 350ms, past the 100-300ms section: round from 100ms
            // straight away, 20 clocks a time round; broken at the 25th.
            let (sections, section_loop_break) = section(100, 300);
            let sent = play_section(&every_10ms(500), 350, &sections, section_loop_break, 25);
            // The 25th is at 140ms the second time round; then on to 490ms.
            // The jump falls when the publisher sees the section armed, a
            // poll after the start, so the 350ms clock may go out first. (The
            // player refuses a section armed past its end; this is the jump
            // at once that a late arm still gets.)
            assert!(sent == 25 + 35 || sent == 1 + 25 + 35, "{sent}");
        }

        /// A seek within the pass under way goes to its first tick at or after
        /// the position, however early in the song.
        #[test]
        fn pass_ticks_seek_goes_back_within_the_pass() {
            let ticks = every_10ms(500);
            let mut schedule = PassTicks::new(&ticks, ms(250));
            assert_eq!(schedule.next(), Some(ms(250)));
            schedule.seek(ms(95));
            assert_eq!(schedule.next(), Some(ms(100)));
        }

        /// The drift steering follows a playback clock running fast or slow
        /// against the wall, read through its buffer-sized steps, and holds
        /// still for one keeping time with it.
        #[test]
        fn the_schedule_is_steered_onto_the_playback_clocks_drift() {
            // Two minutes of readings every 20ms; the playback clock moves in
            // 5.33ms steps (256 frames at 48kHz), so a reading lags by up to
            // that much.
            let step = 256.0 / 48_000.0;
            for ppm in [-100.0, 0.0, 50.0, 100.0] {
                let mut discipline = Discipline::default();
                let mut previous = 0.0;
                for k in 0..6_000u64 {
                    let wall = k as f64 * 0.02;
                    let true_ahead = wall * ppm * 1e-6;
                    let reading = ((wall + true_ahead) / step).floor() * step;
                    discipline.observe(Duration::from_secs_f64(wall), reading - wall);
                    // No tick moves more than the most a window allows.
                    assert!(
                        (discipline.correction - previous).abs() <= Discipline::MAX_STEP + 1e-12
                    );
                    previous = discipline.correction;
                }
                let drift = 120.0 * ppm * 1e-6;
                assert!(
                    (discipline.correction - drift).abs() < 0.002,
                    "{ppm} ppm: corrected {:.4}s for a drift of {drift:.4}s",
                    discipline.correction
                );
            }
        }

        /// With audio, the clock goes back where and when the audio has
        /// committed to, not by a section reading of its own: here the armed
        /// section is 100-300ms, but the audio's one jump goes back from 300
        /// to 150ms, and that is the one made.
        #[test]
        fn a_clock_with_audio_follows_the_jumps_the_audio_commits_to() {
            let (mut sections, section_loop_break) = section(100, 300);
            let jumps = Arc::new(crate::playsync::SectionJumpLog::default());
            jumps.push(crate::section_loop::SectionJump {
                at: ms(300),
                from: ms(300),
                to: ms(150),
                overlap: Duration::ZERO,
                pass: false,
            });
            sections.jumps = jumps.clone();
            // Broken once the audio's jump has been taken: the section's own
            // end would loop again at 500ms, but the audio made no such jump.
            let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 31);
            // 30 to 300ms, back to 150ms, and on to the end: 35 more (a
            // jump of its own would have gone to 100ms: 40).
            assert_eq!(sent, 30 + 35);
        }

        /// A section with no pulse in it (shorter than a pulse, between two)
        /// does not spin the clock thread for ever, its jump finding the same
        /// tick again and again, never sending, never hearing a cancel.
        #[test]
        fn a_section_with_no_pulse_does_not_spin() {
            let (sections, section_loop_break) = section(103, 107);
            let cancel = CancelHandle::new();
            cancel.cancel();
            let handle = std::thread::spawn({
                let sections = sections.clone();
                let cancel = cancel.clone();
                move || {
                    let clock = PlaybackClock::wall();
                    clock.start();
                    let mut sender = MockSender::new();
                    let (_tx, rx) = mpsc::channel();
                    // Started at 110ms, past the section's end.
                    run_beat_clock(
                        &mut sender,
                        &every_10ms(500),
                        ms(110),
                        Some(&sections),
                        None,
                        &cancel,
                        &clock,
                        &rx,
                    );
                }
            });
            // A cancelled schedule returns at once; give it ample time.
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let returned = handle.is_finished();
            // Free a spinning thread either way, so nothing hangs.
            section_loop_break.store(true, Ordering::Relaxed);
            handle.join().unwrap();
            assert!(returned, "the clock spun on a section with no pulse in it");
        }

        /// The last section, ending at or past the last pulse (its end on
        /// the song's end), loops in the audio, and the clock waits for its
        /// jump rather than running out of ticks and sending Stop.
        #[test]
        fn a_section_ending_past_the_last_pulse_still_loops() {
            // Pulses 0-490ms; the section is 300-500ms. 50 to the end, back
            // to 300 (20 a time round); broken at the 60th (390ms the second
            // time round), then on to 490ms: 10 more.
            let (sections, section_loop_break) = section(300, 500);
            let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 60);
            assert_eq!(sent, 70);
        }

        /// a section with no pulse in it at all, past the last one
        /// (a clock that stops before the song does), can never be followed,
        /// yet the clock out of ticks waits for its jump for as long as the
        /// section stays armed: no Stop at the clock's end, and the engine
        /// cannot go on to hold the tempo. Here the section is broken after
        /// 1.5s; the clock (ticks to 490ms) should have stopped long before.
        #[test]
        fn a_section_past_the_last_pulse_does_not_hold_the_clock() {
            let (sections, section_loop_break) = section(600, 800);
            let clock = PlaybackClock::wall();
            clock.start();
            let breaker = std::thread::spawn({
                let section_loop_break = section_loop_break.clone();
                move || {
                    std::thread::sleep(Duration::from_millis(1_500));
                    section_loop_break.store(true, Ordering::Relaxed);
                }
            });
            let mut sender = MockSender::new();
            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                &every_10ms(500),
                Duration::ZERO,
                Some(&sections),
                None,
                &CancelHandle::new(),
                &clock,
                &rx,
            );
            let returned_at = clock.elapsed();
            let still_armed = !section_loop_break.load(Ordering::Relaxed);
            breaker.join().unwrap();
            assert!(
                still_armed,
                "the clock sent Stop only once the loop was broken, at {returned_at:?}"
            );
        }

        /// A section ending on the song's end, past its last pulse (as one in
        /// a looping song may, which no pass interrupts: the section takes
        /// precedence), goes back at its end with no silence and no spin.
        /// The clock used to keep passes of its own, which had rolled into
        /// the next when the jump came.
        #[test]
        fn a_section_ending_on_a_looping_songs_end_goes_back_in_its_pass() {
            let (sections, section_loop_break) = section(300, 500);
            let sent = play_section(&every_10ms(500), 0, &sections, section_loop_break, 52);
            // 0-490ms, round from 300ms (the break at the 52nd, 310ms), and
            // on to the end.
            assert_eq!(sent, 50 + 2 + 18);
        }

        /// a start noticed late drops the pulses already due. A
        /// slaved sequencer counts pulses from Start for its position, so
        /// every one dropped leaves it that many pulses behind the song for
        /// the rest of it (a burst kept the count). Every pulse of the
        /// schedule should go out.
        #[test]
        fn a_late_start_keeps_the_pulse_count() {
            let clock = PlaybackClock::wall();
            clock.start();
            std::thread::sleep(Duration::from_millis(200));
            let mut sender = MockSender::new();
            let (_tx, rx) = mpsc::channel();
            let ticks: Vec<Duration> = (0..30).map(|k| ms(k * 20)).collect();
            run_beat_clock(
                &mut sender,
                &ticks,
                Duration::ZERO,
                None,
                None,
                &CancelHandle::new(),
                &clock,
                &rx,
            );
            let clocks = sender
                .sent
                .lock()
                .iter()
                .filter(|bytes| **bytes == clock_bytes())
                .count();
            assert_eq!(clocks, ticks.len(), "pulses dropped at a late start");
        }

        /// At a tempo whose pulse is a whole number of audio buffers (or
        /// near one), readings taken at the ticks fell at one place in the
        /// playback clock's steps, which wandered as the two drifted, and a
        /// clock keeping perfect time was steered milliseconds off. Readings
        /// are taken at random points in each wait, as `sleep_until` takes
        /// them, and the steering holds to the true drift.
        #[test]
        fn steering_holds_at_a_tempo_matching_the_buffer_rate() {
            for (bpm, frames) in [
                (117.0, 1024.0),
                (117.1875, 1024.0),
                (117.1875, 256.0),
                (107.67, 1024.0),
            ] {
                let step = frames / 48_000.0;
                let pulse = 60.0 / bpm / 24.0;
                for ppm in [0.0, -50.0, 50.0] {
                    let mut discipline = Discipline::default();
                    let mut worst: f64 = 0.0;
                    let mut k = 0u64;
                    loop {
                        // Somewhere in the wait before tick k.
                        let wall = (k as f64 + discipline.next_fraction()) * pulse;
                        if wall > 120.0 {
                            break;
                        }
                        let true_ahead = wall * ppm * 1e-6;
                        let reading = ((wall + true_ahead) / step).floor() * step;
                        discipline.observe(Duration::from_secs_f64(wall), reading - wall);
                        worst = worst.max((discipline.correction - true_ahead).abs());
                        k += 1;
                    }
                    assert!(
                        worst < 0.002,
                        "{bpm} BPM, {frames} frames, {ppm} ppm: steered {:.1}ms off",
                        worst * 1000.0
                    );
                }
            }
        }

        /// A stop just past a seam hands back the song's tempo to hold, not
        /// the gap across the seam.
        #[test]
        fn a_stop_just_after_a_seam_holds_the_songs_tempo() {
            struct CancelAfter {
                n: usize,
                sent: usize,
                cancel: CancelHandle,
            }
            impl MidiSender for CancelAfter {
                fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                    if bytes == clock_bytes().as_slice() {
                        self.sent += 1;
                        if self.sent == self.n {
                            self.cancel.cancel();
                        }
                    }
                    Ok(())
                }
            }
            let cancel = CancelHandle::new();
            // Clocks every 10ms; a pass every 291ms: 30 clocks a pass, the
            // seam 1ms after the last.
            let mut sender = CancelAfter {
                n: 31,
                sent: 0,
                cancel: cancel.clone(),
            };
            let clock = PlaybackClock::wall();
            clock.start();
            let (_tx, rx) = mpsc::channel();
            let interval = run_beat_clock(
                &mut sender,
                &every_10ms(500),
                Duration::ZERO,
                None,
                None,
                &cancel,
                &clock,
                &rx,
            )
            .interval();
            assert_eq!(interval, Some(Duration::from_millis(10)));
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

            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                &ticks,
                Duration::ZERO,
                None,
                None,
                &cancel,
                &clock,
                &rx,
            );

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

            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                &ticks,
                Duration::from_millis(100),
                None,
                None,
                &cancel,
                &clock,
                &rx,
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

            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                &ticks,
                Duration::ZERO,
                None,
                None,
                &cancel,
                &clock,
                &rx,
            );

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

            let (_tx, rx) = mpsc::channel();
            run_beat_clock(
                &mut sender,
                &ticks,
                Duration::ZERO,
                None,
                None,
                &cancel,
                &clock,
                &rx,
            );

            let sent = sender.sent.lock();
            // START + STOP (cancelled before any ticks)
            assert_eq!(sent.len(), 2);
            assert_eq!(sent[0], start_bytes());
            assert_eq!(sent[1], stop_bytes());
        }

        #[test]
        fn returns_last_delivered_interval() {
            // The returned interval reflects the tempo at the end of playback:
            // the spacing between the final two delivered ticks.
            let ticks = vec![
                Duration::from_millis(1),
                Duration::from_millis(3),
                Duration::from_millis(6),
            ];
            let cancel = CancelHandle::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            let (_tx, rx) = mpsc::channel();
            let interval = run_beat_clock(
                &mut sender,
                &ticks,
                Duration::ZERO,
                None,
                None,
                &cancel,
                &clock,
                &rx,
            )
            .interval();

            // Last two ticks: 6ms - 3ms = 3ms.
            assert_eq!(interval, Some(Duration::from_millis(3)));
        }

        #[test]
        fn returns_none_for_single_tick() {
            // A single tick can't establish an interval, so no tempo is reported.
            let ticks = vec![Duration::from_millis(1)];
            let cancel = CancelHandle::new();
            let mut sender = MockSender::new();
            let clock = PlaybackClock::wall();

            let (_tx, rx) = mpsc::channel();
            let interval = run_beat_clock(
                &mut sender,
                &ticks,
                Duration::ZERO,
                None,
                None,
                &cancel,
                &clock,
                &rx,
            )
            .interval();

            assert_eq!(interval, None);
        }
    }

    mod beat_clock_engine_tests {
        use super::*;
        use crate::playsync::CancelHandle;
        use midly::live::SystemRealtime;
        use std::sync::mpsc;
        use std::sync::Arc;

        fn clock_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::TimingClock)
        }
        fn start_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::Start)
        }
        fn stop_bytes() -> Vec<u8> {
            realtime_bytes(SystemRealtime::Stop)
        }

        /// A sender whose buffer can be inspected from another thread while the
        /// engine loop runs.
        #[derive(Clone)]
        struct SharedSender {
            sent: Arc<parking_lot::Mutex<Vec<Vec<u8>>>>,
        }

        impl SharedSender {
            fn new() -> Self {
                SharedSender {
                    sent: Arc::new(parking_lot::Mutex::new(Vec::new())),
                }
            }
            fn len(&self) -> usize {
                self.sent.lock().len()
            }
        }

        impl MidiSender for SharedSender {
            fn send(&mut self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
                self.sent.lock().push(bytes.to_vec());
                Ok(())
            }
        }

        /// Builds a `BeatClockPlay` whose clock has already been started, so the
        /// engine's `wait_for_start_or_cancel` returns immediately.
        fn ready_play(ticks: Vec<Duration>, cancel: CancelHandle) -> BeatClockPlay {
            let clock = PlaybackClock::wall();
            clock.start();
            BeatClockPlay {
                ticks: Arc::new(ticks),
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel,
                clock,
                sections: None,
            }
        }

        /// A closed channel ends the engine while it awaits a start that never
        /// comes, as it does anywhere else.
        #[test]
        fn a_closed_channel_ends_the_engine_awaiting_a_start() {
            let (tx, rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let cancel = CancelHandle::new();
            let play = BeatClockPlay {
                ticks: Arc::new(vec![Duration::ZERO]),
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel: cancel.clone(),
                clock: PlaybackClock::wall(),
                sections: None,
            };
            thread::spawn(move || {
                let mut sender = SharedSender::new();
                beat_clock_engine_loop(
                    &mut sender,
                    true,
                    &Mutex::new(Some(Duration::from_millis(20))),
                    &rx,
                );
                let _ = done_tx.send(());
            });
            tx.send(BeatClockCommand::Play(play)).unwrap();
            thread::sleep(Duration::from_millis(50));
            drop(tx);
            let ended = done_rx.recv_timeout(Duration::from_secs(1)).is_ok();
            cancel.cancel();
            assert!(ended, "the engine kept waiting on a closed channel");
        }

        /// A stop during a song's playback delay is heard at once, not once
        /// the delay is over: a reopen joins this thread while it holds its
        /// lock.
        #[test]
        fn a_stop_during_the_playback_delay_is_heard_at_once() {
            let (tx, rx) = mpsc::channel();
            let engine = thread::spawn(move || {
                let mut sender = SharedSender::new();
                beat_clock_engine_loop(
                    &mut sender,
                    true,
                    &Mutex::new(Some(Duration::from_millis(20))),
                    &rx,
                );
            });
            let mut play = ready_play(vec![Duration::ZERO], CancelHandle::new());
            play.playback_delay = Duration::from_secs(2);
            tx.send(BeatClockCommand::Play(play)).unwrap();
            thread::sleep(Duration::from_millis(50));
            let stopping = std::time::Instant::now();
            tx.send(BeatClockCommand::Shutdown).unwrap();
            engine.join().unwrap();
            assert!(
                stopping.elapsed() < Duration::from_millis(500),
                "the stop waited {:?}",
                stopping.elapsed()
            );
        }

        /// Pacing, decided from a table of elapsed times rather than a clock.
        ///
        /// Nothing here sleeps, spawns, or reads the time, so these assertions
        /// mean the same thing on an idle laptop and a saturated CI runner.
        mod pacing {
            use super::*;

            /// Ticks come one per interval and do not drift.
            #[test]
            fn one_tick_per_interval() {
                let mut hold = TempoHold::new(Duration::from_millis(10));
                let mut sender = SharedSender::new();

                assert!(!hold.tick_if_due_at(&mut sender, Duration::from_millis(9)));
                assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(10)));
                assert!(!hold.tick_if_due_at(&mut sender, Duration::from_millis(19)));
                assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(20)));

                assert_eq!(sender.sent.lock().len(), 2);
                assert!(sender.sent.lock().iter().all(|m| *m == clock_bytes()));
            }

            /// A tick delivered late still schedules the next one on the
            /// original grid, so small jitter does not accumulate into drift.
            #[test]
            fn a_slightly_late_tick_keeps_the_grid() {
                let mut hold = TempoHold::new(Duration::from_millis(10));
                let mut sender = SharedSender::new();

                // Due at 10ms, delivered at 15ms: within one interval late.
                assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(15)));
                // The next is still due at 20ms, not 25ms.
                assert_eq!(
                    hold.until_next_at(Duration::from_millis(15)),
                    Duration::from_millis(5)
                );
            }

            /// Falling more than an interval behind re-anchors instead of
            /// replaying the backlog. This is the case that matters: a 10s stall
            /// at a 10ms interval is a thousand ticks owed, and firing them back
            /// to back reads downstream as an enormous tempo spike.
            #[test]
            fn a_long_stall_re_anchors_instead_of_bursting() {
                let mut hold = TempoHold::new(Duration::from_millis(10));
                let mut sender = SharedSender::new();

                // Gone for ten seconds.
                assert!(hold.tick_if_due_at(&mut sender, Duration::from_secs(10)));

                // Exactly one tick, and the next is a normal interval away --
                // not a thousand ticks owed at zero spacing.
                assert_eq!(sender.sent.lock().len(), 1);
                assert_eq!(
                    hold.until_next_at(Duration::from_secs(10)),
                    Duration::from_millis(10)
                );

                // And it keeps pacing normally from the new anchor.
                assert!(!hold.tick_if_due_at(&mut sender, Duration::from_millis(10_005)));
                assert!(hold.tick_if_due_at(&mut sender, Duration::from_millis(10_010)));
                assert_eq!(sender.sent.lock().len(), 2);
            }

            /// The wait timeout never goes negative or saturates oddly when the
            /// tick is already overdue.
            #[test]
            fn an_overdue_tick_asks_for_no_wait() {
                let hold = TempoHold::new(Duration::from_millis(10));
                assert_eq!(
                    hold.until_next_at(Duration::from_millis(50)),
                    Duration::ZERO
                );
            }
        }

        /// Polls until `cond` holds or the deadline passes.
        ///
        /// These tests assert that a free-running clock keeps running at all,
        /// not that it hits a precise count in a precise window. A shared CI
        /// runner can starve this thread for tens of milliseconds at a time, so
        /// a fixed sleep plus an exact count is a flake waiting to happen --
        /// which is exactly how both of these first landed. Polling makes a slow
        /// machine slower rather than red.
        fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
            let deadline = std::time::Instant::now() + timeout;
            while std::time::Instant::now() < deadline {
                if cond() {
                    return true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            cond()
        }

        /// Held ticks required before believing the clock is really running.
        ///
        /// At most one tick can slip out between sampling the baseline and the
        /// engine picking the queued command up, so anything above that
        /// separates "held the tempo" from "went silent and emitted a straggler".
        const HELD_TICKS: usize = 5;

        /// An engine started holding a tempo, for a device opened afresh,
        /// free-runs it before any song, and a song's tempo replaces it
        /// where the device can read it.
        #[test]
        fn an_engine_started_holding_a_tempo_runs_it_and_keeps_it_current() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let tempo = Arc::new(Mutex::new(Some(Duration::from_millis(10))));
            let handle = thread::spawn({
                let tempo = tempo.clone();
                move || {
                    let mut sender = loop_sender;
                    beat_clock_engine_loop(&mut sender, true, &tempo, &rx);
                }
            });

            assert!(
                wait_for(Duration::from_secs(10), || sender.len() >= HELD_TICKS),
                "the held tempo was not run before a song"
            );
            assert!(sender.sent.lock().iter().all(|msg| *msg == clock_bytes()));

            tx.send(BeatClockCommand::Play(ready_play(
                vec![
                    Duration::from_millis(20),
                    Duration::from_millis(40),
                    Duration::from_millis(60),
                ],
                CancelHandle::new(),
            )))
            .unwrap();
            let set = wait_for(Duration::from_secs(10), || {
                *tempo.lock().unwrap() == Some(Duration::from_millis(20))
            });
            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();
            assert!(
                set,
                "the song's tempo was not kept: {:?}",
                tempo.lock().unwrap()
            );
        }

        /// The gap between "user pressed play" and "every subsystem is ready"
        /// is the boundary persistence exists for, and the engine spends it
        /// waiting on the shared clock. It must keep the tempo running there.
        #[test]
        fn holds_tempo_while_waiting_for_the_go_signal() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);
            });

            // First song establishes a 20ms tempo and ends: START + 3 clocks +
            // STOP, then held ticks.
            tx.send(BeatClockCommand::Play(ready_play(
                vec![
                    Duration::from_millis(20),
                    Duration::from_millis(40),
                    Duration::from_millis(60),
                ],
                CancelHandle::new(),
            )))
            .unwrap();
            assert!(
                wait_for(Duration::from_secs(10), || sender.len() > 5),
                "the first song never established a held tempo"
            );
            let before = sender.len();

            // Second song arrives but its clock has NOT started: the player is
            // still opening devices and loading samples.
            let slow_clock = PlaybackClock::wall();
            tx.send(BeatClockCommand::Play(BeatClockPlay {
                ticks: Arc::new(vec![Duration::from_millis(20), Duration::from_millis(40)]),
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel: CancelHandle::new(),
                clock: slow_clock.clone(),
                sections: None,
            }))
            .unwrap();

            // The go signal is withheld for this whole window, so everything
            // arriving is the held tempo. Blocking silently on the clock -- the
            // behaviour this guards against -- emits nothing here at all.
            let held = wait_for(Duration::from_secs(10), || {
                sender.len() >= before + HELD_TICKS
            });
            let during = sender.len();
            assert!(
                held,
                "clock went silent while waiting for the go signal: {} messages in 10s",
                during - before
            );

            // Releasing the go signal must still let the schedule play.
            slow_clock.start();
            assert!(
                wait_for(Duration::from_secs(10), || sender.len() > during),
                "the schedule never ran once the clock started"
            );
            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            // The Start belongs with the first note, not with the wait.
            let sent = sender.sent.lock();
            for msg in &sent[before..during] {
                assert_eq!(*msg, clock_bytes(), "only timing clocks may be held");
            }
        }

        /// A schedule too short to establish a tempo must not erase the one
        /// already being held.
        #[test]
        fn a_schedule_with_too_few_ticks_keeps_the_previous_tempo() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);
            });

            // Establish a tempo.
            tx.send(BeatClockCommand::Play(ready_play(
                vec![
                    Duration::from_millis(20),
                    Duration::from_millis(40),
                    Duration::from_millis(60),
                ],
                CancelHandle::new(),
            )))
            .unwrap();
            assert!(
                wait_for(Duration::from_secs(10), || sender.len() > 5),
                "the first song never established a held tempo"
            );

            // A one-tick schedule: `run_beat_clock` cannot derive an interval
            // from it and reports None. Its own output is START + 1 clock + STOP.
            let before = sender.len();
            tx.send(BeatClockCommand::Play(ready_play(
                vec![Duration::from_millis(5)],
                CancelHandle::new(),
            )))
            .unwrap();
            assert!(
                wait_for(Duration::from_secs(10), || sender.len() >= before + 3),
                "the short schedule never played"
            );
            let after_short = sender.len();

            let still_running = wait_for(Duration::from_secs(10), || {
                sender.len() >= after_short + HELD_TICKS
            });
            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            assert!(
                still_running,
                "the short schedule wiped the held tempo: {} messages in the 10s after it",
                sender.len() - after_short
            );
        }

        /// Nothing cancels a song's schedule when it ends naturally -- the
        /// cleanup path only clears the join and advances the playlist -- so a
        /// MIDI file longer than its audio leaves the engine still running the
        /// old grid. The next song must take over at once rather than queue
        /// behind it.
        #[test]
        fn a_new_schedule_preempts_a_running_one() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
            });

            // Two seconds of schedule, far longer than this test waits.
            let long: Vec<Duration> = (1..=200).map(|i| Duration::from_millis(i * 10)).collect();
            tx.send(BeatClockCommand::Play(ready_play(
                long,
                CancelHandle::new(),
            )))
            .unwrap();
            thread::sleep(Duration::from_millis(40));

            // The next song, arriving while the first schedule is still going.
            tx.send(BeatClockCommand::Play(ready_play(
                vec![Duration::from_millis(5), Duration::from_millis(10)],
                CancelHandle::new(),
            )))
            .unwrap();
            thread::sleep(Duration::from_millis(80));

            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            // Two Starts within ~120ms. Queued behind the first schedule, the
            // second would not have started for another two seconds.
            let sent = sender.sent.lock();
            let starts = sent.iter().filter(|m| **m == start_bytes()).count();
            assert_eq!(
                starts,
                2,
                "the second song did not preempt the first: {starts} Start(s) in {} messages",
                sent.len()
            );
        }

        /// Shutdown must be observed mid-schedule. Device teardown runs on a
        /// tokio worker and joins this thread, so waiting out a whole song there
        /// blocks a runtime thread for the length of the song.
        #[test]
        fn shutdown_is_observed_during_a_schedule() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
            });

            let long: Vec<Duration> = (1..=200).map(|i| Duration::from_millis(i * 10)).collect();
            tx.send(BeatClockCommand::Play(ready_play(
                long,
                CancelHandle::new(),
            )))
            .unwrap();
            thread::sleep(Duration::from_millis(30));

            let sent_shutdown = std::time::Instant::now();
            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();
            let waited = sent_shutdown.elapsed();

            assert!(
                waited < Duration::from_millis(500),
                "shutdown waited {waited:?} for the schedule to finish"
            );
        }

        #[test]
        fn shutdown_before_any_schedule_sends_nothing() {
            let (tx, rx) = mpsc::channel();
            let mut sender = SharedSender::new();

            tx.send(BeatClockCommand::Shutdown).unwrap();
            // persist on, but no tempo known yet → must block on recv, then exit.
            beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);

            assert!(sender.sent.lock().is_empty());
        }

        #[test]
        fn plays_schedule_then_holds_tempo_when_persisting() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, true, &Mutex::default(), &rx);
            });

            // A 3-tick schedule at ~10ms spacing; final interval = 10ms.
            let cancel = CancelHandle::new();
            tx.send(BeatClockCommand::Play(ready_play(
                vec![
                    Duration::from_millis(10),
                    Duration::from_millis(20),
                    Duration::from_millis(30),
                ],
                cancel,
            )))
            .unwrap();

            // Wait for the schedule (START + 3 clocks + STOP = 5) plus some held
            // ticks to accumulate. Polled rather than slept: a starved thread
            // makes this slower, not wrong.
            let held_ticks = wait_for(Duration::from_secs(10), || sender.len() > 5);
            let held = sender.len();
            assert!(
                held_ticks,
                "expected schedule (5 msgs) plus held ticks, got {held}"
            );

            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            let sent = sender.sent.lock();
            // Schedule framing is intact at the front.
            assert_eq!(sent[0], start_bytes());
            assert_eq!(sent[1], clock_bytes());
            assert_eq!(sent[2], clock_bytes());
            assert_eq!(sent[3], clock_bytes());
            assert_eq!(sent[4], stop_bytes());
            // Everything after the STOP is a held timing clock.
            for msg in &sent[5..] {
                assert_eq!(*msg, clock_bytes(), "held tempo must be timing clocks");
            }
        }

        #[test]
        fn stays_silent_between_songs_without_persist() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
            });

            let cancel = CancelHandle::new();
            tx.send(BeatClockCommand::Play(ready_play(
                vec![Duration::from_millis(5), Duration::from_millis(10)],
                cancel,
            )))
            .unwrap();

            // Wait for the schedule to finish, then hold still and confirm
            // nothing follows it. The negative half stays a fixed window on
            // purpose: load can only make a silent engine emit *less*, so it
            // cannot turn this green when it should be red.
            assert!(
                wait_for(Duration::from_secs(10), || sender.len() >= 4),
                "the schedule never played, got {}",
                sender.len()
            );
            thread::sleep(Duration::from_millis(200));
            let count = sender.len();
            // START + 2 clocks + STOP = 4, and nothing more without persistence.
            assert_eq!(count, 4, "expected exactly the schedule, got {count}");

            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            let sent = sender.sent.lock();
            assert_eq!(sent[0], start_bytes());
            assert_eq!(sent[3], stop_bytes());
        }

        #[test]
        fn queued_schedule_preempts_idle_without_blip() {
            // With a schedule already queued, next_command must take it via
            // try_recv rather than emitting an idle tick first.
            let (tx, rx) = mpsc::channel();
            let mut sender = SharedSender::new();

            let cancel = CancelHandle::new();
            tx.send(BeatClockCommand::Play(ready_play(
                vec![Duration::from_millis(1)],
                cancel,
            )))
            .unwrap();

            let mut hold = None;
            let command = next_command(
                &mut sender,
                true,
                Some(Duration::from_millis(10)),
                &mut hold,
                &rx,
            );
            assert!(matches!(command, Some(BeatClockCommand::Play(_))));
            // No idle tick was emitted while a command was already waiting.
            assert!(sender.sent.lock().is_empty());
        }

        #[test]
        fn cancelled_schedule_is_skipped() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
            });

            // A schedule whose clock never starts and is already cancelled: the
            // engine's cancel-aware wait must let it through without wedging.
            let cancel = CancelHandle::new();
            cancel.cancel();
            let clock = PlaybackClock::wall();
            tx.send(BeatClockCommand::Play(BeatClockPlay {
                ticks: Arc::new(vec![Duration::from_millis(10)]),
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel,
                clock,
                sections: None,
            }))
            .unwrap();

            // The engine must remain responsive to Shutdown afterward.
            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();
            assert!(sender.sent.lock().is_empty());
        }

        #[test]
        fn withholds_start_until_the_clock_go_signal() {
            // Every other test uses `ready_play`, whose clock is already
            // started, so the engine's `wait_for_start_or_cancel` returns
            // immediately and the wait window is zero. This exercises a
            // non-zero window: the engine must take the Play command, then emit
            // nothing until the shared clock is started, so its Start lands with
            // the first note rather than ahead of it.
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                beat_clock_engine_loop(&mut sender, false, &Mutex::default(), &rx);
            });

            // A schedule whose clock has NOT been started yet.
            let cancel = CancelHandle::new();
            let clock = PlaybackClock::wall();
            tx.send(BeatClockCommand::Play(BeatClockPlay {
                ticks: Arc::new(vec![Duration::from_millis(5), Duration::from_millis(10)]),
                start_time: Duration::ZERO,
                playback_delay: Duration::ZERO,
                cancel,
                clock: clock.clone(),
                sections: None,
            }))
            .unwrap();

            // While the clock is unstarted the engine blocks in
            // `wait_for_start_or_cancel` and must emit nothing — not even Start.
            thread::sleep(Duration::from_millis(40));
            assert_eq!(sender.len(), 0, "engine emitted before the go signal");

            // Releasing the go signal lets the schedule play.
            clock.start();
            assert!(
                wait_for(Duration::from_secs(10), || sender.len() >= 4),
                "schedule should play once the clock starts, got {}",
                sender.len()
            );

            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            // START + 2 clocks + STOP, framed correctly, and (no persist) nothing
            // after.
            let sent = sender.sent.lock();
            assert_eq!(
                sent.len(),
                4,
                "expected exactly the schedule, got {}",
                sent.len()
            );
            assert_eq!(sent[0], start_bytes());
            assert_eq!(sent[1], clock_bytes());
            assert_eq!(sent[2], clock_bytes());
            assert_eq!(sent[3], stop_bytes());
        }

        #[test]
        fn run_idle_until_command_returns_command_and_emits_ticks() {
            let (tx, rx) = mpsc::channel();
            let sender = SharedSender::new();
            let loop_sender = sender.clone();
            let handle = thread::spawn(move || {
                let mut sender = loop_sender;
                let mut hold = TempoHold::new(Duration::from_millis(10));
                let cmd = run_idle_until_command(&mut sender, &mut hold, &rx);
                assert!(matches!(cmd, Some(BeatClockCommand::Shutdown)));
            });

            let ticked = wait_for(Duration::from_secs(10), || !sender.sent.lock().is_empty());
            tx.send(BeatClockCommand::Shutdown).unwrap();
            handle.join().unwrap();

            let sent = sender.sent.lock();
            assert!(ticked, "idle free-run should emit timing clocks");
            for msg in sent.iter() {
                assert_eq!(*msg, clock_bytes());
            }
        }
    }
}
