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
    error::Error,
    fs::File,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime},
};

use hound::{SampleFormat, WavSpec, WavWriter};

use crate::songs::Sample;

/// Wait for the given predicate to return true or fail.
#[inline]
#[cfg(test)]
pub fn eventually<F>(predicate: F, error_msg: &str)
where
    F: Fn() -> bool,
{
    let start = SystemTime::now();
    let tick = Duration::from_millis(10);
    let timeout = Duration::from_secs(3);

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate() {
            return;
        }
        thread::sleep(tick);
    }
}

#[cfg(test)]
pub fn write_wav<S: Sample>(path: PathBuf, samples: Vec<Vec<S>>) -> Result<(), Box<dyn Error>> {
    let tempwav = File::create(path)?;
    let sample_format = if S::FORMAT.is_int() || S::FORMAT.is_uint() {
        SampleFormat::Int
    } else if S::FORMAT.is_float() {
        SampleFormat::Float
    } else {
        return Err("Unsupported sample format".into());
    };

    let num_channels = samples.len();
    assert!(num_channels <= u16::MAX.into(), "Too many channels!");
    let mut writer = WavWriter::new(
        tempwav,
        WavSpec {
            channels: num_channels as u16,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format,
        },
    )?;

    // Write a simple set of samples to the wav file.
    for channel in 0..samples.len() {
        for sample in &samples[channel] {
            writer.write_sample(sample.to_owned())?;
        }
    }

    Ok(())
}
