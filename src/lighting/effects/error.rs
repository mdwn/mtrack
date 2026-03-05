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

/// Error types for the effects system
#[derive(Debug)]
pub enum EffectError {
    Fixture(String),
    Parameter(String),
    Timing(String),
}

impl std::fmt::Display for EffectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EffectError::Fixture(msg) => write!(f, "Invalid fixture: {}", msg),
            EffectError::Parameter(msg) => write!(f, "Invalid parameter: {}", msg),
            EffectError::Timing(msg) => write!(f, "Invalid timing: {}", msg),
        }
    }
}

impl std::error::Error for EffectError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_fixture() {
        let e = EffectError::Fixture("par1 not found".to_string());
        assert_eq!(format!("{}", e), "Invalid fixture: par1 not found");
    }

    #[test]
    fn display_parameter() {
        let e = EffectError::Parameter("out of range".to_string());
        assert_eq!(format!("{}", e), "Invalid parameter: out of range");
    }

    #[test]
    fn display_timing() {
        let e = EffectError::Timing("negative duration".to_string());
        assert_eq!(format!("{}", e), "Invalid timing: negative duration");
    }

    #[test]
    fn is_std_error() {
        let e: Box<dyn std::error::Error> = Box::new(EffectError::Fixture("test".to_string()));
        assert!(e.to_string().contains("Invalid fixture"));
    }

    #[test]
    fn debug_format() {
        let e = EffectError::Fixture("test".to_string());
        let debug = format!("{:?}", e);
        assert!(debug.contains("Fixture"));
    }
}
