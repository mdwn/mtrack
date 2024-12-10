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
    fmt::{self},
    sync::{atomic::AtomicBool, Arc, Barrier, Mutex},
    thread,
    time::Duration,
};

use nodi::{Connection, Player};
use rust_dmx::DmxPort;
use tracing::{error, info, span, Level};

use crate::{playsync::CancelHandle, songs::Song};

use super::engine::Engine;

pub struct Device {
    dmx_engine: Arc<Mutex<Engine>>,
}

impl super::Device for Device {
    /// Plays the given song through the DMX interface.
    fn play(
        &self,
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

        let dmx_engine = self.dmx_engine.clone();
        let join_handle = {
            let cancel_handle = cancel_handle.clone();

            thread::spawn(move || {
                let connection = DMXConnection {
                    cancel_handle: cancel_handle.clone(),
                    dmx_engine: dmx_engine.clone(),
                };
                let mut player = Player::new(
                    crate::midi::midir::AccurateTimer::new(
                        dmx_midi_sheet.ticker,
                        cancel_handle.clone(),
                    ),
                    connection,
                );

                let play_finished = Arc::new(AtomicBool::new(false));

                let dmx_write_join_handle = {
                    let play_finished = play_finished.clone();
                    thread::spawn(move || {
                        let ports = rust_dmx::EnttecDmxPort::available_ports();
                        let mut dmx_port = match ports {
                            Ok(mut ports) => {
                                if ports.is_empty() {
                                    error!("No ports available");
                                    return;
                                }

                                // Choose the first available port that isn't at index 0, as rust_dmx
                                // uses the offline DMX port for this.
                                ports.swap_remove(0)
                            }
                            Err(e) => {
                                error!(err = e.to_string(), "Unable to find a DMX port!");
                                return;
                            }
                        };

                        // This write should open the DMX port. Without it the subsequent writes seem not to work as well.
                        // I'm not sure why this is.
                        let _ = dmx_port.write(&[0]);

                        loop {
                            // If playing is finished, return from the write thread.
                            {
                                if play_finished.load(std::sync::atomic::Ordering::Relaxed) {
                                    return;
                                }
                            }
                            let start_time = std::time::SystemTime::now();
                            {
                                let universe = dmx_engine.lock().expect("Unable to get DMX engine lock.");
                                let _ = dmx_port.write(&universe.get_universe(0));
                            }

                            // Sleep for 23 milliseconds if possible. This is roughly a little greater than 44 Hz, which is
                            // the DMX refresh rate.
                            let since = std::time::SystemTime::now()
                                .duration_since(start_time)
                                .expect("current time should not be earlier than start time");
                            let mut sleep_duration = Duration::from_millis(23);
                            if since < sleep_duration {
                                sleep_duration -= since;
                                thread::sleep(sleep_duration);
                            }
                        }
                    })
                };

                play_barrier.wait();
                player.play(&dmx_midi_sheet.sheet);
                cancel_handle.expire();
                play_finished.store(true, std::sync::atomic::Ordering::Relaxed);
                dmx_write_join_handle
                    .join()
                    .expect("Unable to join DMX write thread");
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
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DMX Device")
    }
}

/// Gets the given DMX device.
pub fn get(dmx_engine: Arc<Mutex<Engine>>) -> Device {
    Device {
        dmx_engine,
    }
}

/// DMXConnection is a nodi connection that can be cancelled and will poutput to a
/// DMX interface.
struct DMXConnection {
    cancel_handle: CancelHandle,
    dmx_engine: Arc<Mutex<Engine>>,
}

impl Connection for DMXConnection {
    fn play(&mut self, event: nodi::MidiEvent) -> bool {
        if self.cancel_handle.is_cancelled() {
            return false;
        };

        self.dmx_engine.lock().expect("Unable to get DMX Engine lock").handle_midi_event(event.message);

        true
    }
}