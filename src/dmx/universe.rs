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

use dmx::DmxTransmitter;
use dmx_serial::ErrorKind;
use spin_sleep;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::{sync::RwLock, time::Duration};
use tracing::{error, info};

use crate::playsync::CancelHandle;

/// A DMX universe is 512 channels.
const UNIVERSE_SIZE: usize = 512;

/// The target number of updates per second.
const TARGET_HZ: f64 = 44.0;

/// The configuration for a universe.
pub(crate) struct UniverseConfig {
    /// The path to the serial device that will handle DMX data.
    pub device: String,
    /// FTDI is true if this is attached to an FTDI device.
    pub ftdi: bool,
}

/// A DMX universe.
pub(crate) struct Universe {
    /// The current DMX state.
    current: Arc<RwLock<Vec<f64>>>,
    /// The target DMX state.
    target: Arc<RwLock<Vec<f64>>>,
    /// The current dimming rates.
    rates: Arc<RwLock<Vec<f64>>>,
    /// The current dim global rate.
    global_dim_rate: RwLock<f64>,
    /// Max channels
    max_channels: Arc<AtomicU16>,
    /// The configuration for this universe.
    config: UniverseConfig,
    // The cancel handle for the thread attached to this universe.
    cancel_handle: CancelHandle,
}

impl Universe {
    /// Creates a new universe.
    pub fn new(config: UniverseConfig, cancel_handle: CancelHandle) -> Universe {
        Universe {
            rates: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            current: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            target: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            global_dim_rate: RwLock::new(1.0),
            max_channels: Arc::new(AtomicU16::new(0)),
            config,
            cancel_handle,
        }
    }

    /// Updates the dim speed.
    pub fn update_dim_speed(&mut self, dim_rate: Duration) {
        let mut global_dim_rate = self
            .global_dim_rate
            .write()
            .expect("Unable to get global dim rate write lock");
        if dim_rate.is_zero() {
            *global_dim_rate = 1.0
        } else {
            *global_dim_rate = dim_rate.as_secs_f64() * TARGET_HZ
        }
    }

    /// Updates the universe with the DMX channel/value.
    pub fn update_channel_data(&mut self, channel: u16, value: u8) {
        let channel_index = usize::from(channel);
        let value = f64::from(value);
        self.target
            .write()
            .expect("Unable to get universe target write lock")[channel_index] = value;
        self.rates
            .write()
            .expect("Unable to get universe rates write lock")[channel_index] = (value
            - self
                .current
                .read()
                .expect("unable to get universe current read lock")[channel_index])
            / *self
                .global_dim_rate
                .read()
                .expect("Unable to get universe global dim rate");
        let _ =
            self.max_channels
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current_channel| {
                    if channel >= current_channel {
                        return Some(channel + 1);
                    }
                    None
                });
    }

    /// Starts a thread that writes the universe data to the transmitter.
    pub fn start_thread(&self) -> JoinHandle<()> {
        let rates = self.rates.clone();
        let current = self.current.clone();
        let target = self.target.clone();
        let device = self.config.device.clone();
        let ftdi = self.config.ftdi;
        let max_channels = self.max_channels.clone();
        let cancel_handle = self.cancel_handle.clone();

        thread::spawn(move || {
            let mut dmx_transmitter: Box<dyn DmxTransmitter> = if ftdi {
                Box::new(FTDIDMXTransmitter {
                    dmx_transmitter: Box::new(
                        dmx::open_serial(device.as_str()).expect("Failed to open DMX transmitter"),
                    ),
                })
            } else {
                Box::new(dmx::open_serial(device.as_str()).expect("Failed to open DMX transmitter"))
            };
            let mut last_time = Instant::now();
            let tick_duration = Duration::from_secs(1).div_f64(TARGET_HZ);
            loop {
                if cancel_handle.is_cancelled() {
                    return;
                }
                let current_snapshot = Universe::approach_target(
                    rates.clone(),
                    current.clone(),
                    target.clone(),
                    max_channels.load(Ordering::Relaxed),
                );

                if let Err(e) = dmx_transmitter.send_dmx_packet(&current_snapshot) {
                    error!(
                        err = e.to_string(),
                        "Error sending DMX packet to {}", device
                    );
                }

                last_time += tick_duration;
                spin_sleep::sleep(last_time - Instant::now());
            }
        })
    }

    fn approach_target(
        rates: Arc<RwLock<Vec<f64>>>,
        current: Arc<RwLock<Vec<f64>>>,
        target: Arc<RwLock<Vec<f64>>>,
        max_channels: u16,
    ) -> Vec<u8> {
        let mut current = current
            .write()
            .expect("Unable to get current universe information write lock");
        let rates = rates
            .read()
            .expect("Unable to get rates universe information lock");
        let target = target
            .read()
            .expect("Unable to get target universe information lock");

        for i in 0..usize::from(max_channels) {
            // We want current == target, but due to floating points we'll test if they're close to each other.
            if (current[i] - target[i]).abs() > f64::EPSILON {
                if rates[i] > 0.0 {
                    current[i] = (current[i] + rates[i]).min(target[i])
                } else {
                    current[i] = (current[i] + rates[i]).max(target[i])
                }
            }
        }

        current.as_slice()[0..usize::from(max_channels)]
            .iter()
            .map(|value| {
                info!(value = value, "Value");
                value.min(u8::MAX.into()).max(u8::MIN.into()).round() as u8
            })
            .collect()
    }
}

/// FTDIDMXTransmitter is a DMX transmitter interface that adheres to ENTEC's
/// FTDI protocol.
/// https://cdn.enttec.com/pdf/assets/70304/70304_DMX_USB_PRO_API.pdf
struct FTDIDMXTransmitter {
    dmx_transmitter: Box<dyn DmxTransmitter>,
}

impl DmxTransmitter for FTDIDMXTransmitter {
    fn send_break(&mut self) -> dmx_serial::Result<()> {
        // We actually don't want to send a break here for FTDI devices.
        Ok(())
    }

    fn send_raw_data(&mut self, data: &[u8]) -> dmx_serial::Result<()> {
        // Defer to the regular DMX transmitter.
        self.dmx_transmitter.send_raw_data(data)
    }

    fn send_raw_dmx_packet(&mut self, data: &[u8]) -> dmx_serial::Result<()> {
        // We need to add 5 extra fields to the data that we're going to send.
        let data_len = data.len();
        let mut data_to_send = vec![0u8; data_len + 5];

        if data_len > 512 {
            return Err(dmx_serial::Error::new(
                ErrorKind::InvalidInput,
                format!("universe size too large ({})", data_len),
            ));
        }

        data_to_send[0] = 0x7E; // Start of message delimiter.
        data_to_send[1] = 0x06; // Output Only Send DMX Packet Request
        data_to_send[2] = data_len as u8; // The least significant bits.
        data_to_send[3] = (data_len >> 8) as u8; // The most significant bits.
        data_to_send[4..4 + data_len].copy_from_slice(data); // Copy the DMX universe information.
        data_to_send[5 + data_len] = 0xE7; // End of message delimiter.

        // Send the packet to the serial interface.
        self.send_raw_data(data)
    }
}

#[cfg(test)]
mod test {
    use std::{sync::atomic::Ordering, time::Duration};

    use crate::{
        dmx::universe::{UniverseConfig, TARGET_HZ},
        playsync::CancelHandle,
    };

    use super::Universe;

    fn new_universe() -> Universe {
        Universe::new(
            UniverseConfig {
                device: "mock".into(),
                ftdi: false,
            },
            CancelHandle::new(),
        )
    }

    #[test]
    fn test_no_dimming() {
        let mut universe = new_universe();

        // Let's just worry about the first channels.
        universe.update_channel_data(0, 0);
        universe.update_channel_data(1, 50);
        universe.update_channel_data(2, 100);
        universe.update_channel_data(3, 150);
        universe.update_channel_data(4, 200);

        let result = Universe::approach_target(
            universe.rates.clone(),
            universe.current.clone(),
            universe.target.clone(),
            universe.max_channels.load(Ordering::Relaxed),
        );

        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], result.as_slice());
    }

    #[test]
    fn test_dimming_over_two_seconds() {
        let mut universe = new_universe();

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        // Let's just worry about the first channels.
        universe.update_channel_data(0, 0);
        universe.update_channel_data(1, 50);
        universe.update_channel_data(2, 100);
        universe.update_channel_data(3, 150);
        universe.update_channel_data(4, 200);

        // There are TARGET_HZ updates per second.
        let mut result: Vec<u8> = Vec::new();
        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After one second, we should be halfway there.
        assert_eq!([0u8, 25u8, 50u8, 75u8, 100u8], result.as_slice());

        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After two seconds, we should be all the way there.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], result.as_slice());

        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After another second, nothing should have changed.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], result.as_slice());
    }

    #[test]
    fn test_separate_dimming() {
        let mut universe = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(0, 100);

        // Progress one tick.
        let _ = Universe::approach_target(
            universe.rates.clone(),
            universe.current.clone(),
            universe.target.clone(),
            universe.max_channels.load(Ordering::Relaxed),
        );

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        // The two channels should dim at different rates.
        universe.update_channel_data(1, 100);

        // There are TARGET_HZ updates per second.
        let mut result: Vec<u8> = Vec::new();
        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After one second (+ 1 tick), channel 0 should be done and channel 2 should be halfway there.
        assert_eq!([100u8, 50u8], result.as_slice());

        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After two seconds (+ 1 tick), we should be all the way there.
        assert_eq!([100u8, 100u8], result.as_slice());
    }

    #[test]
    fn test_dimming_override() {
        let mut universe = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(0, 100);

        // There are TARGET_HZ updates per second.
        let mut result: Vec<u8> = Vec::new();
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After half of a second, we should be halfway there.
        assert_eq!([50u8], result.as_slice());

        // Dim over 2 seconds and update the channel data again.
        universe.update_dim_speed(Duration::from_secs(2));
        universe.update_channel_data(0, 100);

        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After 1.5 seconds, we should be halfway with the new dimming speed.
        assert_eq!([75u8], result.as_slice());

        for _ in 0..(TARGET_HZ as usize) {
            result = Universe::approach_target(
                universe.rates.clone(),
                universe.current.clone(),
                universe.target.clone(),
                universe.max_channels.load(Ordering::Relaxed),
            )
        }

        // After 2.5 seconds, we should be all the way there.
        assert_eq!([100u8], result.as_slice());
    }
}
