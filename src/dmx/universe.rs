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

use std::sync::Arc;
use std::{sync::RwLock, time::Duration};
use dmx::DmxTransmitter;
use tracing::error;
use std::time::Instant;
use std::thread::{self, JoinHandle};
use spin_sleep;

const UNIVERSE_SIZE: usize = 512;

/// The target number of updates per second.
const TARGET_HZ: f64 = 44.0;

/// A DMX universe.
struct Universe {
    data: Arc<RwLock<Vec<u8>>>,
    device: String,
}

impl Universe {
    /// Creates a new universe.
    fn new(device: String) -> Universe {
        let universe = Universe {
            data: Arc::new(RwLock::new(vec![0; UNIVERSE_SIZE])),
            device,
        };

        universe
    }

    /// Updates the universe with the DMX channel/value.
    fn update_universe(&mut self, channel: u8, value: u8) {
        self.data.write().expect("Unable to get universe data write lock")[usize::from(channel)] = value;
    }

    /// Starts a thread that writes the universe data to the transmitter.
    fn start_thread(&self) -> JoinHandle<()> {
        let data = self.data.clone();
        let device = self.device.clone();

        thread::spawn(move || {
            let mut dmx_transmitter = dmx::open_serial(device.as_str()).expect("Failed to open DMX transmitter");
            let mut last_time = Instant::now();
            let tick_duration = Duration::from_secs(1).div_f64(TARGET_HZ);
            loop {
                let data_snapshot = {
                    data.read().expect("Unable to get universe data read lock").clone()
                };

                if let Err(e) = dmx_transmitter.send_dmx_packet(&data_snapshot) {
                    error!(err = e.to_string(), "Error sending DMX packet to {}", device);
                }

                last_time += tick_duration;
                spin_sleep::sleep(last_time - Instant::now());
            }
        })
    }
}