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
use super::audio::AudioSampleSource;
use super::error::SampleSourceError;
use super::traits::SampleSource;

/// Create a SampleSource from a file, automatically detecting the file type
pub fn create_sample_source_from_file<P: AsRef<std::path::Path>>(
    path: P,
    start_time: Option<std::time::Duration>,
    buffer_size: usize,
) -> Result<Box<dyn SampleSource>, SampleSourceError> {
    let path = path.as_ref();
    let audio_source = AudioSampleSource::from_file(path, start_time, buffer_size)?;
    Ok(Box::new(audio_source))
}
