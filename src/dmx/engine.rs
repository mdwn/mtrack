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
    collections::{HashMap, HashSet},
    error::Error,
    net::TcpStream,
    sync::{
        atomic::AtomicBool,
        mpsc::{self, Receiver},
        Arc, Barrier, RwLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use midly::num::u7;
use nodi::{Connection, Player};
use ola::{DmxBuffer, StreamingClient};
use tracing::{debug, error, info, span, Level};

use crate::{
    config, midi,
    playsync::CancelHandle,
    songs::{MidiSheet, Song},
};

use super::Universe;

/// The DMX engine. This is meant to control the current state of the
/// universe(s) that should be sent to our DMX interface(s).
pub struct Engine {
    dimming_speed_modifier: f64,
    playback_delay: Duration,
    universes: HashMap<String, RwLock<Universe>>,
    cancel_handle: CancelHandle,
    client_handle: Option<JoinHandle<()>>,
    join_handles: Vec<JoinHandle<()>>,
}

/// DmxMessage is a message that can be passed around between senders and receivers.
pub(super) struct DmxMessage {
    pub universe: u32,
    pub buffer: DmxBuffer,
}

impl Engine {
    /// Creates a new DMX Engine.
    pub fn new(config: &config::Dmx) -> Result<Engine, Box<dyn Error>> {
        let mut maybe_client = None;

        // Attempt to connect to OLA 10 times.
        for _ in 0..10 {
            thread::sleep(Duration::from_secs(5));

            if let Ok(ola_client) = ola::connect() {
                maybe_client = Some(ola_client);
                break;
            };

            debug!("Error connecting to OLA, waiting 5 seconds and trying again.");
        }
        let client = match maybe_client {
            Some(client) => client,
            None => return Err("unable to connect to OLA".into()),
        };
        let (sender, receiver) = mpsc::channel::<DmxMessage>();

        let client_handle = thread::spawn(move || {
            Self::ola_thread(client, receiver);
        });
        let cancel_handle = CancelHandle::new();
        let universes: HashMap<String, Universe> = config
            .universes()
            .into_iter()
            .map(|config| {
                (
                    config.name().to_string(),
                    Universe::new(config, cancel_handle.clone(), sender.clone()),
                )
            })
            .collect();
        let join_handles: Vec<JoinHandle<()>> = universes
            .values()
            .map(|universe| universe.start_thread())
            .collect();
        Ok(Engine {
            dimming_speed_modifier: config.dimming_speed_modifier(),
            playback_delay: config.playback_delay()?,
            universes: universes
                .into_iter()
                .map(|(name, universe)| (name, RwLock::new(universe)))
                .collect(),
            cancel_handle,
            client_handle: Some(client_handle),
            join_handles,
        })
    }

    /// Plays the given song through the DMX interface.
    pub fn play(
        dmx_engine: Arc<RwLock<Engine>>,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (dmx)");
        let _enter = span.enter();

        // No light shows in this song, so return early.
        let light_shows = song.light_shows();
        if light_shows.is_empty() {
            return Ok(());
        }

        info!(
            song = song.name(),
            duration = song.duration_string(),
            "Playing song DMX."
        );

        let (universe_names, playback_delay): (HashSet<String>, Duration) = {
            let dmx_engine = dmx_engine
                .read()
                .expect("Unable to get DMX engine read lock");

            (
                dmx_engine.universes.keys().cloned().collect(),
                dmx_engine.playback_delay,
            )
        };

        let mut dmx_midi_sheets: HashMap<String, (MidiSheet, Vec<u8>)> = HashMap::new();
        let mut empty_barrier_counter = 0;
        for light_show in song.light_shows().iter() {
            let universe_name = light_show.universe_name();
            if !universe_names.contains(&universe_name) {
                // Keep track of the number of threads that should just wait on the play barrier.
                empty_barrier_counter += 1;
                continue;
            }

            dmx_midi_sheets.insert(
                universe_name.clone(),
                (light_show.dmx_midi_sheet()?, light_show.midi_channels()),
            );
        }

        if dmx_midi_sheets.is_empty() {
            info!(song = song.name(), "Song has no matching light shows.");
            return Ok(());
        }

        let mut join_handles: Vec<JoinHandle<()>> = dmx_midi_sheets
            .into_iter()
            .map(|(universe_name, light_show_info)| {
                let dmx_midi_sheet = light_show_info.0;
                let midi_channels = HashSet::from_iter(light_show_info.1);
                let cancel_handle = cancel_handle.clone();
                let dmx_engine = dmx_engine.clone();
                let universe_name = universe_name.clone();
                let play_barrier = play_barrier.clone();

                thread::spawn(move || {
                    let connection = DMXConnection {
                        cancel_handle: cancel_handle.clone(),
                        universe_name,
                        midi_channels,
                        dmx_engine,
                    };
                    let mut player = Player::new(
                        midi::midir::CancelableTimer::new(
                            dmx_midi_sheet.ticker,
                            cancel_handle.clone(),
                        ),
                        connection,
                    );

                    let play_finished = Arc::new(AtomicBool::new(false));

                    play_barrier.wait();
                    spin_sleep::sleep(playback_delay);
                    player.play(&dmx_midi_sheet.sheet);
                    play_finished.store(true, std::sync::atomic::Ordering::Relaxed);
                })
            })
            .collect();

        // We need to make sure we wait on each available universe, even if it shouldn't
        // be played, to get to the appropriate barrier count, which is equal to the number
        // of universes available on the song.
        (0..empty_barrier_counter)
            .map(|_| {
                let play_barrier = play_barrier.clone();
                thread::spawn(move || {
                    play_barrier.wait();
                })
            })
            .for_each(|join_handle| {
                join_handle
                    .join()
                    .expect("Empty barrier join handle should join immediately");
            });

        if cancel_handle.is_cancelled() {
            info!("DMX playback has been cancelled.");
        }

        let results: Vec<Result<(), Box<dyn Error>>> = join_handles
            .drain(..)
            .map(|join_handle| {
                if join_handle.join().is_err() {
                    return Err("Error while joining thread!".into());
                }
                Ok(())
            })
            .collect();
        for result in results.into_iter() {
            result?;
        }

        info!("DMX playback stopped.");

        Ok(())
    }

    /// Handles an incoming MIDI event.
    pub fn handle_midi_event(&mut self, universe_name: String, midi_message: midly::MidiMessage) {
        match midi_message {
            midly::MidiMessage::NoteOn { key, vel } => {
                self.handle_key_velocity(universe_name, key, vel);
            }
            midly::MidiMessage::NoteOff { key, vel } => {
                self.handle_key_velocity(universe_name, key, vel);
            }
            midly::MidiMessage::ProgramChange { program } => {
                self.update_dimming(
                    universe_name,
                    Duration::from_secs_f64(
                        f64::from(program.as_int()) * self.dimming_speed_modifier,
                    ),
                );
            }
            midly::MidiMessage::Controller { controller, value } => {
                self.update_universe(
                    universe_name,
                    controller.as_int().into(),
                    value.as_int() * 2,
                    false,
                );
            }
            _ => {
                debug!(
                    midi_event = format!("{:?}", midi_message),
                    "Unrecognized MIDI event"
                );
            }
        }
    }

    /// Handles MIDI events that use a key and velocity.
    fn handle_key_velocity(&mut self, universe_name: String, key: u7, velocity: u7) {
        self.update_universe(
            universe_name,
            key.as_int().into(),
            velocity.as_int() * 2,
            true,
        )
    }

    // Updates the current dimming speed.
    fn update_dimming(&mut self, universe_name: String, dimming_duration: Duration) {
        debug!(
            dimming = dimming_duration.as_secs_f64(),
            "Dimming speed updated"
        );
        self.universes[&universe_name]
            .write()
            .expect("Unable to get write lock for universe")
            .update_dim_speed(dimming_duration);
    }

    /// Updates the given universe.
    fn update_universe(&mut self, universe_name: String, channel: u16, value: u8, dim: bool) {
        self.universes[&universe_name]
            .write()
            .expect("Unable to get write lock for universe")
            .update_channel_data(channel, value, dim);
    }

    /// Sends messages to OLA.
    fn ola_thread(mut client: StreamingClient<TcpStream>, receiver: Receiver<DmxMessage>) {
        loop {
            match receiver.recv() {
                Ok(message) => {
                    if let Err(err) = client.send_dmx(message.universe, &message.buffer) {
                        error!("error sending DMX to OLA: {}", err.to_string())
                    }
                }
                Err(err) => error!("error receiving DMX message: {}", err.to_string()),
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel_handle.cancel();

        self.join_handles.drain(..).for_each(|join_handle| {
            if join_handle.join().is_err() {
                error!("Error joining handle");
            }
        });
        if self
            .client_handle
            .take()
            .expect("Expected client handle")
            .join()
            .is_err()
        {
            error!("Error joining handle");
        }
    }
}

/// DMXConnection is a nodi connection that can be cancelled and will poutput to a
/// DMX interface.
struct DMXConnection {
    cancel_handle: CancelHandle,
    universe_name: String,
    midi_channels: HashSet<u8>,
    dmx_engine: Arc<RwLock<Engine>>,
}

impl Connection for DMXConnection {
    fn play(&mut self, event: nodi::MidiEvent) -> bool {
        if self.cancel_handle.is_cancelled() {
            return false;
        };

        if self.midi_channels.is_empty() || self.midi_channels.contains(&event.channel.as_int()) {
            self.dmx_engine
                .write()
                .expect("Unable to get DMX engine lock.")
                .handle_midi_event(self.universe_name.clone(), event.message);
        }

        true
    }
}
