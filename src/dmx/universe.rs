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

use ola::DmxBuffer;
use spin_sleep;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::{sync::RwLock, time::Duration};
use tracing::error;

use crate::playsync::CancelHandle;

/// A DMX universe is 512 channels.
const UNIVERSE_SIZE: usize = 512;

/// The target number of updates per second.
const TARGET_HZ: f64 = 44.0;

/// The configuration for a universe.
pub(crate) struct UniverseConfig {
    /// The OpenLighting universe.
    pub universe: u16,
    /// The name of this universe. Will be mapped to a universe by the player.
    pub name: String,
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
    pub fn update_channel_data(&mut self, channel: u16, value: u8, dim: bool) {
        let channel_index = usize::from(channel);
        let value = f64::from(value);
        self.target
            .write()
            .expect("Unable to get universe target write lock")[channel_index] = value;
        self.rates
            .write()
            .expect("Unable to get universe rates write lock")[channel_index] = if dim {
            (value
                - self
                    .current
                    .read()
                    .expect("unable to get universe current read lock")[channel_index])
                / *self
                    .global_dim_rate
                    .read()
                    .expect("Unable to get universe global dim rate")
        } else {
            0.0
        };

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
        let max_channels = self.max_channels.clone();
        let cancel_handle = self.cancel_handle.clone();
        let universe = self.config.universe;

        thread::spawn(move || {
            let mut last_time = Instant::now();
            let tick_duration = Duration::from_secs(1).div_f64(TARGET_HZ);
            let mut client = ola::connect().unwrap();
            let mut buffer = DmxBuffer::new();

            loop {
                if cancel_handle.is_cancelled() {
                    return;
                }
                if Universe::approach_target(
                    rates.clone(),
                    current.clone(),
                    target.clone(),
                    max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ) {
                    if let Err(e) = client.send_dmx(1, &buffer) {
                        error!(
                            err = e.to_string(),
                            "Error sending DMX packet to universe {}", universe
                        );
                    }
                }

                last_time += tick_duration;
                spin_sleep::sleep(last_time - Instant::now());
            }
        })
    }

    /// Takes the given inputs and approaches the current expected DMX values.
    /// Returns true if anything changed.
    fn approach_target(
        rates: Arc<RwLock<Vec<f64>>>,
        current: Arc<RwLock<Vec<f64>>>,
        target: Arc<RwLock<Vec<f64>>>,
        max_channels: u16,
        buffer: &mut DmxBuffer,
    ) -> bool {
        let mut current = current
            .write()
            .expect("Unable to get current universe information write lock");
        let rates = rates
            .read()
            .expect("Unable to get rates universe information lock");
        let target = target
            .read()
            .expect("Unable to get target universe information lock");

        let mut changed = false;
        for i in 0..usize::from(max_channels) {
            // We want current == target, but due to floating points we'll test if they're close to each other.
            if (current[i] - target[i]).abs() > f64::EPSILON {
                changed = true;
                if rates[i] > 0.0 {
                    current[i] = (current[i] + rates[i]).min(target[i])
                } else {
                    current[i] = (current[i] + rates[i]).max(target[i])
                }
                buffer.set_channel(
                    i,
                    current[i].min(u8::MAX.into()).max(u8::MIN.into()).round() as u8,
                );
            }
        }
        changed
    }
}

#[cfg(test)]
mod test {
    use std::{sync::atomic::Ordering, time::Duration};

    use ola::DmxBuffer;

    use crate::{
        dmx::universe::{UniverseConfig, TARGET_HZ},
        playsync::CancelHandle,
    };

    use super::Universe;

    fn new_universe() -> Universe {
        Universe::new(
            UniverseConfig {
                universe: 1,
                name: "universe".into(),
            },
            CancelHandle::new(),
        )
    }

    #[test]
    fn test_no_dimming() {
        let mut universe = new_universe();

        universe.update_channel_data(0, 0, true);
        universe.update_channel_data(1, 50, true);
        universe.update_channel_data(2, 100, true);
        universe.update_channel_data(3, 150, true);
        universe.update_channel_data(4, 200, true);

        let mut buffer = DmxBuffer::new();

        Universe::approach_target(
            universe.rates.clone(),
            universe.current.clone(),
            universe.target.clone(),
            universe.max_channels.load(Ordering::Relaxed),
            &mut buffer,
        );

        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_ignore_dimming() {
        let mut universe = new_universe();

        // Dim over 2 seconds. This will be ignored.
        universe.update_dim_speed(Duration::from_secs(2));

        universe.update_channel_data(0, 0, false);
        universe.update_channel_data(1, 50, false);
        universe.update_channel_data(2, 100, false);
        universe.update_channel_data(3, 150, false);
        universe.update_channel_data(4, 200, false);

        let mut buffer = DmxBuffer::new();

        Universe::approach_target(
            universe.rates.clone(),
            universe.current.clone(),
            universe.target.clone(),
            universe.max_channels.load(Ordering::Relaxed),
            &mut buffer,
        );

        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_dimming_over_two_seconds() {
        let mut universe = new_universe();

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        universe.update_channel_data(0, 0, true);
        universe.update_channel_data(1, 50, true);
        universe.update_channel_data(2, 100, true);
        universe.update_channel_data(3, 150, true);
        universe.update_channel_data(4, 200, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After one second, we should be halfway there.
        assert_eq!([0u8, 25u8, 50u8, 75u8, 100u8], buffer.as_slice()[0..5]);

        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After two seconds, we should be all the way there.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);

        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                false
            )
        }

        // After another second, nothing should have changed.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_separate_dimming() {
        let mut universe = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(0, 100, true);

        let mut buffer = DmxBuffer::new();

        // Progress one tick.
        let _ = Universe::approach_target(
            universe.rates.clone(),
            universe.current.clone(),
            universe.target.clone(),
            universe.max_channels.load(Ordering::Relaxed),
            &mut buffer,
        );

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        // The two channels should dim at different rates.
        universe.update_channel_data(1, 100, true);

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After one second (+ 1 tick), channel 0 should be done and channel 2 should be halfway there.
        assert_eq!([100u8, 50u8], buffer.as_slice()[0..2]);

        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After two seconds (+ 1 tick), we should be all the way there.
        assert_eq!([100u8, 100u8], buffer.as_slice()[0..2]);
    }

    #[test]
    fn test_dimming_override() {
        let mut universe = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(0, 100, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After half of a second, we should be halfway there.
        assert_eq!([50u8], buffer.as_slice()[0..1]);

        // Dim over 2 seconds and update the channel data again.
        universe.update_dim_speed(Duration::from_secs(2));
        universe.update_channel_data(0, 100, true);

        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After 1.5 seconds, we should be halfway with the new dimming speed.
        assert_eq!([75u8], buffer.as_slice()[0..1]);

        for _ in 0..(TARGET_HZ as usize) {
            assert_eq!(
                Universe::approach_target(
                    universe.rates.clone(),
                    universe.current.clone(),
                    universe.target.clone(),
                    universe.max_channels.load(Ordering::Relaxed),
                    &mut buffer,
                ),
                true
            )
        }

        // After 2.5 seconds, we should be all the way there.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }
}
