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
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        mpsc::{channel, Sender},
        Arc, Barrier,
    },
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Stream,
};
use hound::SampleFormat;
use tracing::{debug, error, info, span, Level};

use crate::{
    playsync::CancelHandle,
    songs::{self, Song},
};

/// A small wrapper around a cpal::Device. Used for storing some extra
/// data that makes multitrack playing more convenient.
pub struct Device {
    /// The name of the device.
    name: String,
    /// The maximum number of channels the device supports.
    max_channels: u16,
    /// The host ID of the device.
    host_id: cpal::HostId,
    /// The underlying cpal device.
    device: cpal::Device,
    /// Supports i32.
    supports_i32: bool,
    /// Supports f32.
    supports_f32: bool,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (Channels={}) ({})",
            self.name,
            self.max_channels,
            self.host_id.name()
        )
    }
}

impl Device {
    /// Lists cpal devices and produces the Device trait.
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
        // Suppress noisy output here.
        let _shh_stdout = shh::stdout()?;
        let _shh_stderr = shh::stderr()?;

        let mut devices: Vec<Device> = Vec::new();
        for host_id in cpal::available_hosts() {
            let host_devices = match cpal::host_from_id(host_id)?.devices() {
                Ok(host_devices) => host_devices,
                Err(e) => {
                    error!(
                        err = e.to_string(),
                        host = host_id.name(),
                        "Unable to list devices for host"
                    );
                    continue;
                }
            };

            for device in host_devices {
                let mut max_channels = 0;

                let output_configs = device.supported_output_configs();
                if let Err(e) = output_configs {
                    debug!(
                        err = e.to_string(),
                        host = host_id.name(),
                        device = device.name().unwrap_or_default(),
                        "Error getting output configs"
                    );
                    continue;
                }

                let mut supports_f32 = false;
                let mut supports_i32 = false;

                for output_config in device.supported_output_configs()? {
                    if output_config.sample_format().is_float() {
                        supports_f32 = true;
                    }
                    if output_config.sample_format().is_int() {
                        supports_i32 = true;
                    }
                    if max_channels < output_config.channels() {
                        max_channels = output_config.channels();
                    }
                }

                if max_channels > 0 {
                    devices.push(Device {
                        name: device.name()?,
                        max_channels,
                        host_id,
                        device,
                        supports_f32,
                        supports_i32,
                    })
                }
            }
        }

        devices.sort_by_key(|device| device.name.to_string());
        Ok(devices)
    }

    /// Gets the given cpal device.
    pub fn get(name: &String) -> Result<Device, Box<dyn Error>> {
        match Device::list_cpal_devices()?
            .into_iter()
            .find(|device| device.name == *name)
        {
            Some(device) => Ok(device),
            None => Err(format!("no device found with name {}", name).into()),
        }
    }
}

impl super::Device for Device {
    /// Play the given song through the audio device.
    fn play(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (cpal)");
        let _enter = span.enter();

        info!(
            format = if song.sample_format == SampleFormat::Float {
                "float"
            } else {
                "int"
            },
            device = self.name,
            song = song.name,
            duration = song.duration_string(),
            "Playing song."
        );

        let num_channels = *mappings
            .iter()
            .flat_map(|entry| entry.1)
            .max()
            .ok_or("no max channel found")?;

        if self.max_channels < num_channels {
            return Err(format!(
                "{} channels requested for song {}, audio device {} only has {}",
                num_channels, song.name, self.name, self.max_channels
            )
            .into());
        }

        let (tx, rx) = channel();

        play_barrier.wait();
        let output_stream = if self.supports_i32 && song.sample_format == hound::SampleFormat::Int {
            debug!("Playing i32->i32");
            self.build_stream::<i32, i32>(song, mappings, num_channels, tx, cancel_handle)?
        } else if self.supports_f32 && song.sample_format == hound::SampleFormat::Float {
            debug!("Playing f32->f32");
            self.build_stream::<f32, f32>(song, mappings, num_channels, tx, cancel_handle)?
        } else if self.supports_i32 && song.sample_format == hound::SampleFormat::Float {
            debug!("Playing f32->i32");
            self.build_stream::<f32, i32>(song, mappings, num_channels, tx, cancel_handle)?
        } else if self.supports_f32 && song.sample_format == hound::SampleFormat::Int {
            debug!("Playing i32->f32");
            self.build_stream::<i32, f32>(song, mappings, num_channels, tx, cancel_handle)?
        } else {
            return Err("Device does not support correct sample format for song".into());
        };
        output_stream.play()?;

        // Wait for the read finish.
        rx.recv()?;

        Ok(())
    }
}

impl Device {
    /// Builds an output stream.
    fn build_stream<S: songs::Sample, C: cpal::SizedSample + cpal::FromSample<S> + 'static>(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        num_channels: u16,
        tx: Sender<()>,
        cancel_handle: CancelHandle,
    ) -> Result<Stream, Box<dyn Error>> {
        let stream_config = cpal::StreamConfig {
            channels: num_channels,
            sample_rate: cpal::SampleRate(song.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let error_callback = |err: cpal::StreamError| {
            error!(err = err.to_string(), "Error during stream.");
        };

        let source = song.source::<S>(mappings)?;
        let mut output_callback = Device::output_callback::<S, C>(source, tx, cancel_handle);
        let stream = self.device.build_output_stream(
            &stream_config,
            move |data, _| output_callback(data),
            error_callback,
            None,
        );

        match stream {
            Ok(stream) => Ok(stream),
            Err(e) => Err(e.to_string().into()),
        }
    }
    // If the playback should stop, this sends on the provided Sender and returns true. This will
    // only return true and send if we're on a frame boundary.
    fn signal_stop<S: songs::Sample>(
        source: &songs::SongSource<S>,
        tx: &Sender<()>,
        cancel_handle: &CancelHandle,
    ) -> bool {
        // Stop only when we hit a frame boundary. This will prevent weird noises
        // when stopping a song.
        if cancel_handle.is_cancelled() && source.get_frame_position() == 0 {
            if tx.send(()).is_err() {
                error!("Error sending message")
            }
            true
        } else {
            false
        }
    }

    // Creates a callback function that fills the output device buffer.
    fn output_callback<S: songs::Sample, F: cpal::Sample + cpal::FromSample<S>>(
        mut source: songs::SongSource<S>,
        tx: Sender<()>,
        cancel_handle: CancelHandle,
    ) -> impl FnMut(&mut [F]) {
        move |data: &mut [F]| {
            let data_len = data.len();
            let mut data_pos = 0;

            loop {
                // Copy the data from the song reader buffer to the output buffer
                // sample by sample until we hit either the end of the output buffer or the
                // reader buffer.
                for data in data.iter_mut().take(data_len).skip(data_pos) {
                    if Device::signal_stop(&source, &tx, &cancel_handle) {
                        return;
                    }

                    match source.next() {
                        Some(sample) => {
                            *data = sample.to_sample::<F>();
                            data_pos += 1;
                        }
                        None => {
                            if tx.send(()).is_err() {
                                error!("Error sending message")
                            }
                            return;
                        }
                    }
                }

                // We'll also check if things are stopped here to prevent an extra iteration.
                if Device::signal_stop(&source, &tx, &cancel_handle) {
                    return;
                }

                if data_pos == data_len {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, sync::mpsc::channel};

    use crate::{
        playsync::CancelHandle,
        songs::{Song, Track},
        test::write_wav,
    };

    #[test]
    fn output_callback() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?.into_path();
        let tempwav1_path = tempdir.join("tempwav1.wav");
        let tempwav2_path = tempdir.join("tempwav2.wav");

        write_wav(tempwav1_path.clone(), vec![1_i32, 2_i32, 3_i32])?;
        write_wav(tempwav2_path.clone(), vec![2_i32, 3_i32])?;

        let track1 = Track::new("test 1".into(), tempwav1_path, Some(1))?;
        let track2 = Track::new("test 2".into(), tempwav2_path, Some(1))?;

        let song = Song::new(
            "song name".into(),
            None,
            None,
            Vec::new(),
            vec![track1, track2],
        )?;
        let mut mappings: HashMap<String, Vec<u16>> = HashMap::new();
        mappings.insert("test 1".into(), vec![1]);
        mappings.insert("test 2".into(), vec![4]);

        let source = song.source::<i32>(&mappings)?;
        let (tx, rx) = channel();
        let cancel_handle = CancelHandle::new();
        let mut callback = super::Device::output_callback(source, tx, cancel_handle.clone());

        let mut data = [0_i32; 2];

        callback(&mut data);
        assert_eq!([1_i32, 0_i32], data);
        callback(&mut data);
        assert_eq!([0_i32, 2_i32], data);
        callback(&mut data);
        assert_eq!([2_i32, 0_i32], data);
        callback(&mut data);
        assert_eq!([0_i32, 3_i32], data);
        callback(&mut data);
        assert_eq!([3_i32, 0_i32], data);
        callback(&mut data);
        assert_eq!([0_i32, 0_i32], data);
        callback(&mut data);

        rx.recv().expect("Expected receive once callback is done.");
        Ok(())
    }

    #[test]
    fn stop_callback_immediately() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?.into_path();
        let tempwav1_path = tempdir.join("tempwav1.wav");
        let tempwav2_path = tempdir.join("tempwav2.wav");

        write_wav(tempwav1_path.clone(), vec![1_i32, 2_i32, 3_i32])?;
        write_wav(tempwav2_path.clone(), vec![2_i32, 3_i32])?;

        let track1 = Track::new("test 1".into(), tempwav1_path, Some(1))?;
        let track2 = Track::new("test 2".into(), tempwav2_path, Some(1))?;

        let song = Song::new(
            "song name".into(),
            None,
            None,
            Vec::new(),
            vec![track1, track2],
        )?;
        let mut mappings: HashMap<String, Vec<u16>> = HashMap::new();
        mappings.insert("test 1".into(), vec![1]);
        mappings.insert("test 2".into(), vec![4]);

        let source = song.source::<i32>(&mappings)?;
        let (tx, rx) = channel();
        let cancel_handle = CancelHandle::new();
        let mut callback = super::Device::output_callback(source, tx, cancel_handle.clone());

        let mut data = [0_i32; 2];

        // This should immediately stop since we're on a frame boundary.
        cancel_handle.cancel();

        callback(&mut data);
        assert_eq!([0_i32, 0_i32], data);

        rx.recv().expect("Expected receive once callback is done.");

        Ok(())
    }

    #[test]
    fn stop_callback_on_frame_boundary() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?.into_path();
        let tempwav1_path = tempdir.join("tempwav1.wav");
        let tempwav2_path = tempdir.join("tempwav2.wav");

        write_wav(tempwav1_path.clone(), vec![1_i32, 2_i32, 3_i32])?;
        write_wav(tempwav2_path.clone(), vec![2_i32, 3_i32])?;

        let track1 = Track::new("test 1".into(), tempwav1_path, Some(1))?;
        let track2 = Track::new("test 2".into(), tempwav2_path, Some(1))?;

        let song = Song::new(
            "song name".into(),
            None,
            None,
            Vec::new(),
            vec![track1, track2],
        )?;
        let mut mappings: HashMap<String, Vec<u16>> = HashMap::new();
        mappings.insert("test 1".into(), vec![1]);
        mappings.insert("test 2".into(), vec![4]);

        let source = song.source::<i32>(&mappings)?;
        let (tx, rx) = channel();
        let cancel_handle = CancelHandle::new();
        let mut callback = super::Device::output_callback(source, tx, cancel_handle.clone());

        let mut data = [0_i32; 2];

        callback(&mut data);
        assert_eq!([1_i32, 0_i32], data);

        // This should allow one more get, then it should stop once we hit the frame boundary.
        cancel_handle.cancel();
        callback(&mut data);
        assert_eq!([0_i32, 2_i32], data);

        rx.recv().expect("Expected receive once callback is done.");
        Ok(())
    }
}
