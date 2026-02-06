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

use ola::DmxBuffer;
use spin_sleep;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::{sync::RwLock, time::Duration};
use tracing::error;

use crate::config;
use crate::playsync::CancelHandle;

use super::engine::DmxMessage;

/// A DMX universe is 512 channels.
const UNIVERSE_SIZE: usize = 512;

/// The target number of updates per second.
const TARGET_HZ: f64 = 44.0;

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
    config: config::Universe,
    /// The cancel handle for the thread attached to this universe.
    cancel_handle: CancelHandle,
    /// Used to send data to the OLA client thread.
    ola_sender: Sender<DmxMessage>,
}

impl Universe {
    /// Creates a new universe.
    pub(super) fn new(
        config: config::Universe,
        cancel_handle: CancelHandle,
        ola_sender: Sender<DmxMessage>,
    ) -> Universe {
        Universe {
            rates: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            current: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            target: Arc::new(RwLock::new(vec![0.0; UNIVERSE_SIZE])),
            global_dim_rate: RwLock::new(1.0),
            max_channels: Arc::new(AtomicU16::new(0)),
            config,
            cancel_handle,
            ola_sender,
        }
    }

    #[cfg(test)]
    pub fn get_dim_speed(&self) -> f64 {
        *self
            .global_dim_rate
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(test)]
    pub fn get_target_value(&self, channel_index: usize) -> f64 {
        self.target.read().unwrap_or_else(|e| e.into_inner())[channel_index]
    }

    /// Updates the dim speed.
    pub fn update_dim_speed(&self, dim_rate: Duration) {
        let mut global_dim_rate = self
            .global_dim_rate
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if dim_rate.is_zero() {
            *global_dim_rate = 1.0
        } else {
            *global_dim_rate = dim_rate.as_secs_f64() * TARGET_HZ
        }
    }

    /// Updates the universe with the DMX channel/value.
    pub fn update_channel_data(&self, channel: u16, value: u8, dim: bool) {
        let channel_index = if channel > 0 {
            usize::from(channel - 1) // Convert from 1-based DMX to 0-based OLA
        } else {
            0 // Handle channel 0 case - map to index 0
        };
        let value = f64::from(value);
        self.target.write().unwrap_or_else(|e| e.into_inner())[channel_index] = value;
        self.rates.write().unwrap_or_else(|e| e.into_inner())[channel_index] = if dim {
            (value - self.current.read().unwrap_or_else(|e| e.into_inner())[channel_index])
                / *self
                    .global_dim_rate
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
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

    /// Updates the universe with effect commands (bypasses dimming for immediate effect)
    pub fn update_effect_commands(&self, commands: Vec<(u16, u8)>) {
        for (channel, value) in commands {
            self.update_channel_data(channel, value, false); // No dimming for effects
        }
    }

    /// Starts a thread that writes the universe data to the transmitter.
    pub fn start_thread(&self) -> JoinHandle<()> {
        let rates = self.rates.clone();
        let current = self.current.clone();
        let target = self.target.clone();
        let max_channels = self.max_channels.clone();
        let cancel_handle = self.cancel_handle.clone();
        let universe = u32::from(self.config.universe());
        let ola_sender = self.ola_sender.clone();

        thread::spawn(move || {
            let mut last_time = Instant::now();
            let tick_duration = Duration::from_secs(1).div_f64(TARGET_HZ);

            let mut buffer = DmxBuffer::new();

            loop {
                if cancel_handle.is_cancelled() {
                    return;
                }

                if Universe::approach_target(&rates, &current, &target, &max_channels, &mut buffer)
                {
                    if let Err(e) = ola_sender.send(DmxMessage {
                        universe,
                        buffer: buffer.clone(),
                    }) {
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
        rates: &Arc<RwLock<Vec<f64>>>,
        current: &Arc<RwLock<Vec<f64>>>,
        target: &Arc<RwLock<Vec<f64>>>,
        max_channels: &Arc<AtomicU16>,
        buffer: &mut DmxBuffer,
    ) -> bool {
        let mut current = current.write().unwrap_or_else(|e| e.into_inner());
        let rates = rates.read().unwrap_or_else(|e| e.into_inner());
        let target = target.read().unwrap_or_else(|e| e.into_inner());

        let mut changed = false;
        for i in 0..usize::from(max_channels.load(Ordering::Relaxed)) {
            // We want current == target, but due to floating points we'll test if they're close to each other.
            if (current[i] - target[i]).abs() > f64::EPSILON {
                changed = true;
                if rates[i] > 0.0 {
                    current[i] = (current[i] + rates[i]).min(target[i])
                } else if rates[i] == 0.0 {
                    current[i] = target[i]
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
    use std::{
        error::Error,
        sync::mpsc::{self, Receiver},
        thread,
        time::Duration,
    };

    use ola::DmxBuffer;

    use crate::{
        config,
        dmx::{engine::DmxMessage, universe::TARGET_HZ},
        playsync::CancelHandle,
    };

    use super::Universe;

    fn new_universe() -> (Universe, Receiver<DmxMessage>) {
        let (sender, receiver) = mpsc::channel();
        (
            Universe::new(
                config::Universe::new(1, "universe".to_string()),
                CancelHandle::new(),
                sender,
            ),
            receiver,
        )
    }

    #[test]
    fn test_thread() -> Result<(), Box<dyn Error>> {
        // We just want to make sure that the thread vaguely does what we think it should.
        let (universe, receiver) = new_universe();

        let receiver_handle = thread::spawn(move || receiver.recv());

        let handle = universe.start_thread();
        universe.update_channel_data(0, 0, false);
        universe.update_channel_data(1, 50, false);

        let result = receiver_handle
            .join()
            .map_err(|_| "Error waiting for join".to_string())??;

        assert_eq!([50u8, 0u8], result.buffer.as_slice()[0..2]);

        universe.cancel_handle.cancel();
        handle
            .join()
            .map_err(|_| "Error waiting for join".to_string())?;

        Ok(())
    }

    #[test]
    fn test_no_dimming() {
        let (universe, _) = new_universe();

        universe.update_channel_data(1, 0, true);
        universe.update_channel_data(1, 50, true);
        universe.update_channel_data(2, 100, true);
        universe.update_channel_data(3, 150, true);
        universe.update_channel_data(4, 200, true);

        let mut buffer = DmxBuffer::new();

        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        assert_eq!([50u8, 100u8, 150u8, 200u8, 0u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_ignore_dimming() {
        let (universe, _) = new_universe();

        // Dim over 2 seconds. This will be ignored.
        universe.update_dim_speed(Duration::from_secs(2));

        universe.update_channel_data(1, 0, false);
        universe.update_channel_data(2, 50, false);
        universe.update_channel_data(3, 100, false);
        universe.update_channel_data(4, 150, false);
        universe.update_channel_data(5, 200, false);

        let mut buffer = DmxBuffer::new();

        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);

        // We found a bug with dimming back down, so let's test that here.
        universe.update_channel_data(2, 50u8, false);
        universe.update_channel_data(3, 200u8, false);
        universe.update_channel_data(4, 0, false);

        Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        assert_eq!([0u8, 50u8, 200u8, 0u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_dimming_over_two_seconds() {
        let (universe, _) = new_universe();

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        universe.update_channel_data(1, 0, true);
        universe.update_channel_data(2, 50, true);
        universe.update_channel_data(3, 100, true);
        universe.update_channel_data(4, 150, true);
        universe.update_channel_data(5, 200, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second, we should be halfway there.
        // Channel 1 (index 0): 0 -> 0, Channel 2 (index 1): 0 -> 25, Channel 3 (index 2): 0 -> 50, etc.
        assert_eq!([0u8, 25u8, 50u8, 75u8, 100u8], buffer.as_slice()[0..5]);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After two seconds, we should be all the way there.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(!Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After another second, nothing should have changed.
        assert_eq!([0u8, 50u8, 100u8, 150u8, 200u8], buffer.as_slice()[0..5]);
    }

    #[test]
    fn test_separate_dimming() {
        let (universe, _) = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(1, 100, true);

        let mut buffer = DmxBuffer::new();

        // Progress one tick.
        let _ = Universe::approach_target(
            &universe.rates,
            &universe.current,
            &universe.target,
            &universe.max_channels,
            &mut buffer,
        );

        // Dim over 2 seconds.
        universe.update_dim_speed(Duration::from_secs(2));

        // The two channels should dim at different rates.
        universe.update_channel_data(1, 100, true);

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second (+ 1 tick), channel 1 (index 0) should be at ~51
        // (started at 0, +1 tick at 1s rate = ~2, then +44 ticks at 2s rate = ~51)
        assert!(buffer.as_slice()[0] >= 50 && buffer.as_slice()[0] <= 52);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After two seconds (+ 1 tick), we should be all the way there.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }

    #[test]
    fn test_dimming_override() {
        let (universe, _) = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));
        universe.update_channel_data(1, 100, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After half of a second, we should be halfway there.
        assert_eq!([50u8], buffer.as_slice()[0..1]);

        // Dim over 2 seconds and update the channel data again.
        universe.update_dim_speed(Duration::from_secs(2));
        universe.update_channel_data(1, 100, true);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After 1.5 seconds, we should be halfway with the new dimming speed.
        assert_eq!([75u8], buffer.as_slice()[0..1]);

        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After 2.5 seconds, we should be all the way there.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }

    #[test]
    fn test_dimming_down_from_higher_to_lower() {
        let (universe, _) = new_universe();

        // Dim over 1 second.
        universe.update_dim_speed(Duration::from_secs(1));

        // First, dim up to 200
        universe.update_channel_data(1, 200, true);

        let mut buffer = DmxBuffer::new();

        // There are TARGET_HZ updates per second.
        for _ in 0..(TARGET_HZ as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second, we should be at 200.
        assert_eq!([200u8], buffer.as_slice()[0..1]);

        // Now dim down from 200 to 100
        universe.update_channel_data(1, 100, true);

        // After half a second, we should be halfway (at 150)
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // Should be around 150 (halfway between 200 and 100)
        let value = buffer.as_slice()[0];
        assert!(
            value >= 148 && value <= 152,
            "Expected ~150 when dimming down from 200 to 100, got {}",
            value
        );

        // Continue for another half second to reach target
        for _ in 0..((TARGET_HZ / 2.0) as usize) {
            assert!(Universe::approach_target(
                &universe.rates,
                &universe.current,
                &universe.target,
                &universe.max_channels,
                &mut buffer,
            ))
        }

        // After one second total, we should be at 100.
        assert_eq!([100u8], buffer.as_slice()[0..1]);
    }
}
