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
use super::alsa;
use std::{
    any::type_name,
    error::Error,
    fmt,
    sync::{mpsc::channel, Arc},
};

use cpal::traits::{DeviceTrait, StreamTrait};
use tracing::{error, info, span, Level};

use crate::{
    playsync::CancelHandle,
    songs::{self, Song},
};

/// A small wrapper around a rodio::Device. Used for storing some extra
/// data that makes multitrack playing more convenient.
pub struct Device {
    /// The name of the device.
    pub name: String,
    /// The long name of the device. May be empty.
    pub long_name: String,
    /// IDs that will match this device.
    matches: Vec<String>,
    /// The underlying rodio::Device that will be doing our low level operations.
    device: cpal::Device,
    /// The ID of the host that this device belongs to.
    host_id: cpal::HostId,
    /// The maximum number of channels this device can play back through.
    max_channels: u16,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (Channels={}) ({:?})",
            self.name, self.max_channels, self.host_id
        )
    }
}

impl Device {
    /// Lists rodio devices and produces the Device trait.
    pub fn list() -> Result<Vec<Box<dyn super::Device>>, Box<dyn Error>> {
        Ok(Device::list_cpal_devices()?
            .into_iter()
            .map(|device| {
                let device: Box<dyn super::Device> = Box::new(device);
                device
            })
            .collect())
    }

    /// Lists cpal devices.
    fn list_cpal_devices() -> Result<Vec<Device>, Box<dyn Error>> {
        let mut devices: Vec<Device> = Vec::new();
        for host_id in cpal::available_hosts() {
            match host_id {
                cpal::HostId::Alsa => {
                    for device in alsa::list_devices()? {
                        devices.push(Device {
                            name: device.name,
                            long_name: device.long_name,
                            matches: device.matches,
                            device: device.device,
                            host_id: cpal::HostId::Alsa,
                            max_channels: u16::try_from(device.channels)?,
                        });
                    }
                }
            }
        }

        devices.sort_by_key(|device| device.name.to_string());
        Ok(devices)
    }

    /// Gets the given rodio device.
    pub fn get(name: &String) -> Result<Device, Box<dyn Error>> {
        match Device::list_cpal_devices()?.into_iter().find(|device| {
            device
                .matches
                .iter()
                .any(|device_name| *device_name == *name)
        }) {
            Some(device) => Ok(device),
            None => Err(format!("no device found with name {}", name).into()),
        }
    }
}

impl super::Device for Device {
    /// Returns the name of the device.
    fn name(&self) -> String {
        self.name.clone()
    }

    /// Play the given song through the audio device.
    fn play(&self, song: Arc<Song>, cancel_handle: CancelHandle) -> Result<(), Box<dyn Error>> {
        match song.sample_format {
            hound::SampleFormat::Int => self.play_format::<i32>(song, cancel_handle),
            hound::SampleFormat::Float => self.play_format::<f32>(song, cancel_handle),
        }
    }
}

impl Device {
    /// Plays the given song using the specified format.
    fn play_format<S>(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
    ) -> Result<(), Box<dyn Error>>
    where
        S: songs::Sample,
    {
        let span = span!(Level::INFO, "play song (cpal)");
        let _enter = span.enter();
        let format_string = type_name::<S>();

        info!(
            format = format_string,
            device = self.name,
            song = song.name,
            duration = song.duration_string(),
            "Playing song."
        );
        if self.max_channels < song.num_channels {
            return Err(format!(
                "Song {} requires {} channels, audio device {} only has {}",
                song.name, song.num_channels, self.name, self.max_channels
            )
            .into());
        }
        let mut source = song.source::<S>(self.max_channels)?;

        let (tx, rx) = channel();

        let err_fn = |err: cpal::StreamError| {
            error!(err = err.to_string(), "Error during stream.");
        };

        let output_data_fn = move |data: &mut [S], _: &cpal::OutputCallbackInfo| {
            // The data length and data position keep track of where we are in the output buffer.
            let data_len = data.len();
            let mut data_pos = 0;

            loop {
                // The playback has been cancelled, so cease playback.
                if cancel_handle.is_cancelled() {
                    tx.send(()).expect("error sending message");
                    return;
                }

                // Copy the data from the song reader buffer to the output buffer
                // sample by sample until we hit either the end of the output buffer or the
                // reader buffer.
                for data in data.iter_mut().take(data_len).skip(data_pos) {
                    match source.next() {
                        Some(sample) => {
                            *data = sample;
                            data_pos += 1;
                        }
                        None => {
                            tx.send(()).expect("error sending message");
                            return;
                        }
                    }
                }

                if data_pos == data_len {
                    return;
                }
            }
        };

        let output_stream = self.device.build_output_stream(
            &cpal::StreamConfig {
                channels: self.max_channels,
                sample_rate: cpal::SampleRate(song.sample_rate),
                buffer_size: cpal::BufferSize::Default,
            },
            output_data_fn,
            err_fn,
            None,
        )?;
        output_stream.play()?;

        // Wait for the read finish.
        rx.recv()?;

        Ok(())
    }
}
