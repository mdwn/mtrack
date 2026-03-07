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
    any::Any,
    error::Error,
    fmt,
    sync::{Arc, Barrier},
};

use midly::live::LiveEvent;
use tokio::sync::mpsc::Sender;

use crate::{config, dmx::engine::Engine, playsync::CancelHandle, songs::Song};

pub(crate) mod midir;
pub(crate) mod mock;
pub(crate) mod playback;
mod transform;

/// A MIDI device that can play MIDI files and listen for inputs.
pub trait Device: Any + fmt::Display + std::marker::Send + std::marker::Sync {
    /// Watches MIDI input for events and sends them to the given sender. If a DMX engine
    /// is loaded, the events may be passed through to the engine.
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>>;

    /// Stops watching events.
    fn stop_watch_events(&self);

    /// Plays the given song through the MIDI interface, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
        start_time: std::time::Duration,
    ) -> Result<(), Box<dyn Error>>;

    /// Emits an event.
    fn emit(&self, midi_event: Option<LiveEvent<'static>>) -> Result<(), Box<dyn Error>>;

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<mock::Device>, Box<dyn Error>>;
}

/// Lists devices known to midir.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    midir::list()
}

/// Gets a device with the given name.
pub fn get_device(
    config: Option<config::Midi>,
    dmx_engine: Option<Arc<Engine>>,
) -> Result<Option<Arc<dyn Device>>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    let device = config.device();
    if device.starts_with("mock") {
        return Ok(Some(Arc::new(mock::Device::get(device))));
    };

    Ok(Some(Arc::new(midir::get(&config, dmx_engine)?)))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn get_device_none_config_returns_none() {
        let result = get_device(None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_device_mock_returns_some() {
        let config = config::Midi::new("mock-midi", None);
        let result = get_device(Some(config), None).unwrap();
        assert!(result.is_some());
        assert!(format!("{}", result.unwrap()).contains("mock-midi"));
    }

    #[test]
    fn mock_device_display() {
        let device = mock::Device::get("mock-test");
        assert!(format!("{}", device).contains("mock-test"));
        assert!(format!("{}", device).contains("Mock"));
    }

    #[test]
    fn mock_device_emit_none_is_ok() {
        let device = mock::Device::get("mock-test");
        let midi_device: &dyn Device = &device;
        assert!(midi_device.emit(None).is_ok());
    }

    #[test]
    fn mock_device_emit_some_stores_event() {
        let device = mock::Device::get("mock-test");
        let event = LiveEvent::Midi {
            channel: 0.into(),
            message: midly::MidiMessage::NoteOn {
                key: midly::num::u7::new(60),
                vel: midly::num::u7::new(100),
            },
        };
        let midi_device: &dyn Device = &device;
        assert!(midi_device.emit(Some(event)).is_ok());
        let emitted = device.get_emitted_event();
        assert!(emitted.is_some());
        let bytes = emitted.unwrap();
        assert_eq!(bytes[0], 0x90); // NoteOn channel 0
        assert_eq!(bytes[1], 60);
        assert_eq!(bytes[2], 100);
    }

    #[test]
    fn mock_device_reset_emitted_event() {
        let device = mock::Device::get("mock-test");
        let event = LiveEvent::Midi {
            channel: 0.into(),
            message: midly::MidiMessage::NoteOn {
                key: midly::num::u7::new(60),
                vel: midly::num::u7::new(100),
            },
        };
        let midi_device: &dyn Device = &device;
        midi_device.emit(Some(event)).unwrap();
        assert!(device.get_emitted_event().is_some());
        device.reset_emitted_event();
        assert!(device.get_emitted_event().is_none());
    }

    #[test]
    fn mock_device_to_mock() {
        let device = mock::Device::get("mock-test");
        let midi_device: &dyn Device = &device;
        let mock = midi_device.to_mock();
        assert!(mock.is_ok());
    }

    #[test]
    fn mock_device_to_string_contains_mock() {
        let device = mock::Device::get("my-device");
        let s = format!("{}", device);
        assert!(s.contains("my-device"));
        assert!(s.contains("Mock"));
    }

    #[tokio::test]
    async fn mock_device_watch_and_stop() {
        let device = mock::Device::get("mock-test");
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let midi_device: &dyn Device = &device;
        midi_device.watch_events(tx).unwrap();

        // Send a mock event
        device.mock_event(&[0x90, 60, 100]);

        // Should receive it
        let received = rx.recv().await.unwrap();
        assert_eq!(received, vec![0x90, 60, 100]);

        // Stop watching
        midi_device.stop_watch_events();

        // Second watch_events should fail since the closed flag is set
        // (the old connection's thread exited)
    }

    #[tokio::test]
    async fn mock_device_watch_events_already_watching() {
        let device = mock::Device::get("mock-test");
        let (tx1, _rx1) = tokio::sync::mpsc::channel(10);
        let (tx2, _rx2) = tokio::sync::mpsc::channel(10);
        let midi_device: &dyn Device = &device;
        midi_device.watch_events(tx1).unwrap();

        // Second call should fail
        let result = midi_device.watch_events(tx2);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Already watching"));

        // Clean up
        midi_device.stop_watch_events();
    }
}
