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
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        mpsc::{channel, Sender},
        Arc, Barrier,
    },
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Stream,
};
use hound::SampleFormat;
use tracing::{debug, error, info, span, Level};

use crate::{
    audio::TargetFormat,
    config,
    playsync::CancelHandle,
    songs::{self, Song},
};

/// A small wrapper around a cpal::Device. Used for storing some extra
/// data that makes multitrack playing more convenient.
pub struct Device {
    /// The name of the device.
    name: String,
    /// Controls how long to wait before playback of an audio file starts.
    playback_delay: Duration,
    /// The maximum number of channels the device supports.
    max_channels: u16,
    /// The host ID of the device.
    host_id: cpal::HostId,
    /// The underlying cpal device.
    device: cpal::Device,
    /// The target format for this device.
    target_format: TargetFormat,
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

                for output_config in device.supported_output_configs()? {
                    if max_channels < output_config.channels() {
                        max_channels = output_config.channels();
                    }
                }

                if max_channels > 0 {
                    // Create device with default format - will be overridden in get() method
                    let default_format = TargetFormat::new(44100, SampleFormat::Int, 32)?;

                    devices.push(Device {
                        name: device.name()?,
                        playback_delay: Duration::ZERO,
                        max_channels,
                        host_id,
                        device,
                        target_format: default_format,
                    })
                }
            }
        }

        devices.sort_by_key(|device| device.name.to_string());
        Ok(devices)
    }

    /// Gets the given cpal device.
    pub fn get(config: config::Audio) -> Result<Device, Box<dyn Error>> {
        let name = config.device();
        match Device::list_cpal_devices()?
            .into_iter()
            .find(|device| device.name.trim() == name)
        {
            Some(mut device) => {
                device.playback_delay = config.playback_delay()?;

                device.target_format = TargetFormat::new(
                    config.sample_rate(),
                    config.sample_format()?,
                    config.bits_per_sample(),
                )?;
                Ok(device)
            }
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

        let is_transcoded = song.needs_transcoding(&self.target_format);
        info!(
            format = if song.sample_format() == SampleFormat::Float {
                "float"
            } else {
                "int"
            },
            device = self.name,
            song = song.name(),
            duration = song.duration_string(),
            transcoded = is_transcoded,
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
                num_channels,
                song.name(),
                self.name,
                self.max_channels
            )
            .into());
        }

        let (tx, rx) = channel();

        play_barrier.wait();
        spin_sleep::sleep(self.playback_delay);
        // Use the device's target format - transcoder handles all conversion
        let output_stream = if self.target_format.sample_format == SampleFormat::Float {
            debug!("Playing with f32 target format");
            self.build_stream::<f32, f32>(song, mappings, num_channels, tx, cancel_handle)?
        } else {
            debug!("Playing with i32 target format");
            self.build_stream::<f32, i32>(song, mappings, num_channels, tx, cancel_handle)?
        };
        output_stream.play()?;

        // Wait for the read finish.
        rx.recv()?;

        Ok(())
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<super::mock::Device>, Box<dyn Error>> {
        Err("not a mock".into())
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
        // Use the device's configured target format
        let target_format = &self.target_format;

        // Use the target format's sample rate for the CPAL device
        let stream_config = cpal::StreamConfig {
            channels: num_channels,
            sample_rate: cpal::SampleRate(target_format.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let error_callback = |err: cpal::StreamError| {
            error!(err = err.to_string(), "Error during stream.");
        };

        let source = song.source::<S>(mappings, target_format.clone())?;
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
        audio::TargetFormat, config, playsync::CancelHandle, songs::Song, testutil::write_wav,
    };

    #[test]
    fn output_callback() -> Result<(), Box<dyn Error>> {
        let tempdir = tempfile::tempdir()?.into_path();
        let tempwav1 = "tempwav1.wav";
        let tempwav2 = "tempwav2.wav";

        write_wav(
            tempdir.join(tempwav1),
            vec![vec![1_i32, 2_i32, 3_i32]],
            44100,
        )?;
        write_wav(tempdir.join(tempwav2), vec![vec![2_i32, 3_i32]], 44100)?;

        let track1 = config::Track::new("test 1".into(), tempwav1, Some(1));
        let track2 = config::Track::new("test 2".into(), tempwav2, Some(1));

        let song = Song::new(
            &tempdir,
            &config::Song::new("song name", None, None, None, None, vec![track1, track2]),
        )?;
        let mut mappings: HashMap<String, Vec<u16>> = HashMap::new();
        mappings.insert("test 1".into(), vec![1]);
        mappings.insert("test 2".into(), vec![4]);

        let source = song.source::<i32>(&mappings, TargetFormat::default())?;
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
        let tempwav1 = "tempwav1.wav";
        let tempwav2 = "tempwav2.wav";

        write_wav(
            tempdir.join(tempwav1),
            vec![vec![1_i32, 2_i32, 3_i32]],
            44100,
        )?;
        write_wav(tempdir.join(tempwav2), vec![vec![2_i32, 3_i32]], 44100)?;

        let track1 = config::Track::new("test 1".into(), tempwav1, Some(1));
        let track2 = config::Track::new("test 2".into(), tempwav2, Some(1));

        let song = Song::new(
            &tempdir,
            &config::Song::new("song name", None, None, None, None, vec![track1, track2]),
        )?;
        let mut mappings: HashMap<String, Vec<u16>> = HashMap::new();
        mappings.insert("test 1".into(), vec![1]);
        mappings.insert("test 2".into(), vec![4]);

        let source = song.source::<i32>(&mappings, TargetFormat::default())?;
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
        let tempwav1 = "tempwav1.wav";
        let tempwav2 = "tempwav2.wav";

        write_wav(
            tempdir.join(tempwav1),
            vec![vec![1_i32, 2_i32, 3_i32]],
            44100,
        )?;
        write_wav(tempdir.join(tempwav2), vec![vec![2_i32, 3_i32]], 44100)?;

        let track1 = config::Track::new("test 1".into(), tempwav1, Some(1));
        let track2 = config::Track::new("test 2".into(), tempwav2, Some(1));

        let song = Song::new(
            &tempdir,
            &config::Song::new("song name", None, None, None, None, vec![track1, track2]),
        )?;
        let mut mappings: HashMap<String, Vec<u16>> = HashMap::new();
        mappings.insert("test 1".into(), vec![1]);
        mappings.insert("test 2".into(), vec![4]);

        let source = song.source::<i32>(&mappings, TargetFormat::default())?;
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
