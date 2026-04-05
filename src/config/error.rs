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

use std::path::PathBuf;

/// Typed error for config load/parse failures so callers can distinguish
/// e.g. file-not-found from parse errors without string matching.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Config load/parse error: {0}")]
    Load(#[from] config::ConfigError),

    #[error("IO error reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to parse profile from {path}: {source}")]
    ProfileParse {
        path: PathBuf,
        source: config::ConfigError,
    },

    #[error(
        "Checksum mismatch: config has changed since last read; re-fetch to get current state"
    )]
    StaleChecksum { expected: String, actual: String },

    #[error("Config serialization error: {0}")]
    StoreSerialization(String),

    #[error("Config I/O error: {0}")]
    StoreIo(String),

    #[error("Invalid profile index {index} (have {len} profiles)")]
    InvalidProfileIndex { index: usize, len: usize },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Missing field: {0}")]
    MissingField(String),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_io_error() {
        let e = ConfigError::Io {
            path: PathBuf::from("/tmp/config.yaml"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        };
        let msg = format!("{}", e);
        assert!(msg.contains("/tmp/config.yaml"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn is_std_error() {
        let e = ConfigError::Io {
            path: PathBuf::from("test"),
            source: std::io::Error::other("test"),
        };
        let boxed: Box<dyn std::error::Error> = Box::new(e);
        assert!(boxed.to_string().contains("test"));
    }
}
