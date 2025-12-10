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
use std::path::Path;

use super::error::TranscodingError;
use super::traits::SampleSource;
use super::wav::WavSampleSource;

/// Create a SampleSource from a file, automatically detecting the file type
pub fn create_sample_source_from_file<P: AsRef<Path>>(
    path: P,
) -> Result<Box<dyn SampleSource>, TranscodingError> {
    create_sample_source_from_file_with_seek(path, None)
}

pub fn create_sample_source_from_file_with_seek<P: AsRef<Path>>(
    path: P,
    start_time: Option<std::time::Duration>,
) -> Result<Box<dyn SampleSource>, TranscodingError> {
    let path = path.as_ref();

    // Get file extension to determine type
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "wav" => {
            let wav_source = WavSampleSource::from_file_with_seek(path, start_time)?;
            Ok(Box::new(wav_source))
        }
        _ => Err(TranscodingError::SampleConversionFailed(format!(
            "Unsupported file format: {}",
            extension
        ))),
    }
}
