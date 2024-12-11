// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
    error::Error,
    sync::{atomic::AtomicBool, Arc, Barrier, RwLock},
    thread::{self, JoinHandle},
    time::Duration,
};

use midly::num::u7;
use nodi::{Connection, Player};
use tracing::{debug, error, info, span, Level};

use crate::{playsync::CancelHandle, songs::Song};

use super::{universe::UniverseConfig, Universe};

/// The DMX engine. This is meant to control the current state of the
/// universe(s) that should be sent to our DMX interface(s).
pub struct Engine {
    universes: Vec<RwLock<Universe>>,
    cancel_handle: CancelHandle,
    join_handles: Vec<JoinHandle<()>>,
}

impl Engine {
    /// Creates a new DMX Engine.
    pub fn new(configs: Vec<UniverseConfig>) -> Engine {
        let cancel_handle = CancelHandle::new();
        let universes: Vec<Universe> = configs
            .into_iter()
            .map(|config| Universe::new(config, cancel_handle.clone()))
            .collect();
        let join_handles: Vec<JoinHandle<()>> = universes
            .iter()
            .map(|universe| universe.start_thread())
            .collect();
        Engine {
            universes: universes.into_iter().map(RwLock::new).collect(),
            cancel_handle,
            join_handles,
        }
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

        let dmx_midi_sheet = match song.dmx_midi_sheet()? {
            Some(dmx_midi_sheet) => dmx_midi_sheet,
            None => {
                info!(song = song.name, "Song has no DMX MIDI sheet.");
                return Ok(());
            }
        };

        info!(
            song = song.name,
            duration = song.duration_string(),
            "Playing song DMX."
        );

        let join_handle = {
            let cancel_handle = cancel_handle.clone();
            let dmx_engine = dmx_engine.clone();

            thread::spawn(move || {
                let connection = DMXConnection {
                    cancel_handle: cancel_handle.clone(),
                    dmx_engine,
                };
                let mut player = Player::new(
                    crate::midi::midir::AccurateTimer::new(
                        dmx_midi_sheet.ticker,
                        cancel_handle.clone(),
                    ),
                    connection,
                );

                let play_finished = Arc::new(AtomicBool::new(false));

                play_barrier.wait();
                player.play(&dmx_midi_sheet.sheet);
                cancel_handle.expire();
                play_finished.store(true, std::sync::atomic::Ordering::Relaxed);
            })
        };

        cancel_handle.wait();

        if cancel_handle.is_cancelled() {
            info!("DMX playback has been cancelled.");
        }

        if join_handle.join().is_err() {
            return Err("Error while joining thread!".into());
        }

        info!("DMX playback stopped.");

        Ok(())
    }

    /// Handles an incoming MIDI event.
    pub fn handle_midi_event(&mut self, midi_message: midly::MidiMessage) {
        match midi_message {
            midly::MidiMessage::NoteOn { key, vel } => {
                self.handle_key_velocity(0, key, vel);
            }
            midly::MidiMessage::NoteOff { key, vel } => {
                self.handle_key_velocity(0, key, vel);
            }
            midly::MidiMessage::ProgramChange { program } => {
                self.update_dimming(0, Duration::from_secs(program.as_int().into()));
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
    fn handle_key_velocity(&mut self, universe_number: usize, key: u7, velocity: u7) {
        self.update_universe(universe_number, key.as_int().into(), velocity.as_int() * 2)
    }

    // Updates the current dimming speed.
    fn update_dimming(&mut self, universe_number: usize, dimming_duration: Duration) {
        info!(
            dimming = dimming_duration.as_secs_f64(),
            "Dimming speed updated"
        );
        self.universes[universe_number]
            .write()
            .expect("Unable to get write lock for universe")
            .update_dim_speed(dimming_duration);
    }

    /// Updates the given universe.
    fn update_universe(&mut self, universe_number: usize, channel: u16, value: u8) {
        self.universes[universe_number]
            .write()
            .expect("Unable to get write lock for universe")
            .update_channel_data(channel, value);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel_handle.cancel();

        self.join_handles.drain(..).for_each(|join_handle| {
            if join_handle.join().is_err() {
                error!("Error joining handle")
            }
        });
    }
}

/// DMXConnection is a nodi connection that can be cancelled and will poutput to a
/// DMX interface.
struct DMXConnection {
    cancel_handle: CancelHandle,
    dmx_engine: Arc<RwLock<Engine>>,
}

impl Connection for DMXConnection {
    fn play(&mut self, event: nodi::MidiEvent) -> bool {
        if self.cancel_handle.is_cancelled() {
            return false;
        };

        self.dmx_engine
            .write()
            .expect("Unable to get DMX engine lock.")
            .handle_midi_event(event.message);

        true
    }
}
