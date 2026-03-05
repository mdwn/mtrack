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
/// Error types for sample source operations
#[derive(Debug, thiserror::Error)]
pub enum SampleSourceError {
    #[error("Resampling failed: {0}Hz -> {1}Hz")]
    ResamplingFailed(u32, u32),

    #[error("Sample conversion failed for {0}")]
    SampleConversionFailed(String),

    #[error("Audio file error: {0}")]
    AudioError(#[from] symphonia::core::errors::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_resampling_failed() {
        let e = SampleSourceError::ResamplingFailed(44100, 48000);
        assert_eq!(format!("{}", e), "Resampling failed: 44100Hz -> 48000Hz");
    }

    #[test]
    fn display_sample_conversion_failed() {
        let e = SampleSourceError::SampleConversionFailed("test.wav".to_string());
        assert_eq!(format!("{}", e), "Sample conversion failed for test.wav");
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let e: SampleSourceError = io_err.into();
        assert!(format!("{}", e).contains("file not found"));
    }

    #[test]
    fn is_std_error() {
        let e: Box<dyn std::error::Error> =
            Box::new(SampleSourceError::ResamplingFailed(44100, 48000));
        assert!(e.to_string().contains("Resampling"));
    }
}
