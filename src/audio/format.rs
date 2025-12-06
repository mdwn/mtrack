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

use std::{error::Error, fmt, str::FromStr};

/// Sample format enumeration for audio processing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// Integer samples (e.g., 16-bit, 24-bit, 32-bit)
    Int,
    /// Floating point samples (e.g., 32-bit float, 64-bit float)
    Float,
}

impl FromStr for SampleFormat {
    /// Convert from string representation
    fn from_str(s: &str) -> Result<Self, Box<dyn Error>> {
        match s {
            "float" | "Float" => Ok(SampleFormat::Float),
            "int" | "Int" => Ok(SampleFormat::Int),
            _ => Err(format!("Unsupported sample format: {}", s).into()),
        }
    }

    type Err = Box<dyn Error>;
}

impl SampleFormat {
    /// Convert to string representation
    pub fn as_str(self) -> &'static str {
        match self {
            SampleFormat::Float => "float",
            SampleFormat::Int => "int",
        }
    }
}

impl fmt::Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Target audio format for transcoding
#[derive(Debug, Clone, PartialEq)]
pub struct TargetFormat {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Sample format (integer or float)
    pub sample_format: SampleFormat,
    /// Bits per sample
    pub bits_per_sample: u16,
}

impl TargetFormat {
    /// Creates a new TargetFormat
    pub fn new(
        sample_rate: u32,
        sample_format: SampleFormat,
        bits_per_sample: u16,
    ) -> Result<Self, Box<dyn Error>> {
        // Basic sanity check - let the audio interface decide what's actually supported
        if sample_rate == 0 {
            return Err("Sample rate must be greater than 0".into());
        }

        Ok(TargetFormat {
            sample_rate,
            sample_format,
            bits_per_sample,
        })
    }
}

impl Default for TargetFormat {
    /// Creates a default target format (44.1kHz, 16-bit integer)
    fn default() -> Self {
        TargetFormat {
            sample_rate: 44100,
            sample_format: SampleFormat::Int,
            bits_per_sample: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_format_from_str() {
        // Test valid formats
        assert_eq!(
            SampleFormat::from_str("float").unwrap(),
            SampleFormat::Float
        );
        assert_eq!(
            SampleFormat::from_str("Float").unwrap(),
            SampleFormat::Float
        );
        assert_eq!(SampleFormat::from_str("int").unwrap(), SampleFormat::Int);
        assert_eq!(SampleFormat::from_str("Int").unwrap(), SampleFormat::Int);
    }

    #[test]
    fn test_sample_format_from_str_invalid() {
        // Test invalid formats
        assert!(SampleFormat::from_str("invalid").is_err());
        assert!(SampleFormat::from_str("").is_err());
        assert!(SampleFormat::from_str("double").is_err());
    }

    #[test]
    fn test_sample_format_as_str() {
        assert_eq!(SampleFormat::Float.as_str(), "float");
        assert_eq!(SampleFormat::Int.as_str(), "int");
    }

    #[test]
    fn test_sample_format_display() {
        assert_eq!(format!("{}", SampleFormat::Float), "float");
        assert_eq!(format!("{}", SampleFormat::Int), "int");
    }

    #[test]
    fn test_target_format_new() {
        // Test valid creation
        let format = TargetFormat::new(44100, SampleFormat::Float, 32).unwrap();
        assert_eq!(format.sample_rate, 44100);
        assert_eq!(format.sample_format, SampleFormat::Float);
        assert_eq!(format.bits_per_sample, 32);

        let format = TargetFormat::new(48000, SampleFormat::Int, 16).unwrap();
        assert_eq!(format.sample_rate, 48000);
        assert_eq!(format.sample_format, SampleFormat::Int);
        assert_eq!(format.bits_per_sample, 16);
    }

    #[test]
    fn test_target_format_new_invalid() {
        // Test invalid sample rate
        assert!(TargetFormat::new(0, SampleFormat::Float, 32).is_err());
    }

    #[test]
    fn test_target_format_default() {
        let format = TargetFormat::default();
        assert_eq!(format.sample_rate, 44100);
        assert_eq!(format.sample_format, SampleFormat::Int);
        assert_eq!(format.bits_per_sample, 16);
    }

    #[test]
    fn test_target_format_equality() {
        let format1 = TargetFormat::new(44100, SampleFormat::Float, 32).unwrap();
        let format2 = TargetFormat::new(44100, SampleFormat::Float, 32).unwrap();
        let format3 = TargetFormat::new(48000, SampleFormat::Float, 32).unwrap();

        assert_eq!(format1, format2);
        assert_ne!(format1, format3);
    }
}
