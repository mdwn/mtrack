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

use std::{error::Error, time::Duration};

use duration_string::DurationString;
use serde::{Deserialize, Serialize};

use super::lighting::Lighting;

/// The default DMX dimming speed.
#[cfg(not(test))]
pub const DEFAULT_OLA_PORT: u16 = 9010;
pub const DEFAULT_DMX_DIMMING_SPEED_MODIFIER: f64 = 1.0;
pub const DEFAULT_DMX_PLAYBACK_DELAY: Duration = Duration::ZERO;

/// A YAML representation of the DMX configuration.
#[derive(Deserialize, Serialize, Clone)]
pub struct Dmx {
    /// Controls the dim speed modifier. A modifier of 1.0 means a dim speed of 1 == 1.0 second.
    dim_speed_modifier: Option<f64>,

    /// Controls how long to wait before playback of a DMX lighting file starts.
    playback_delay: Option<String>,

    /// The OLA port. Defaults to the default OLA port.
    ola_port: Option<u16>,

    /// The configuration of devices to universes.
    universes: Vec<Universe>,

    /// The lighting configuration.
    lighting: Option<Lighting>,

    /// When true, use a no-op OLA client if the OLA daemon is unavailable.
    /// Allows the effects engine to run (e.g. for TUI display) without OLA.
    #[serde(default)]
    null_client: bool,
}

impl Dmx {
    /// Creates a new DMX configuration.
    #[cfg(test)]
    pub fn new(
        dim_speed_modifier: Option<f64>,
        playback_delay: Option<String>,
        ola_port: Option<u16>,
        universes: Vec<Universe>,
        lighting: Option<Lighting>,
    ) -> Dmx {
        Dmx {
            dim_speed_modifier,
            playback_delay,
            ola_port,
            universes,
            lighting,
            null_client: false,
        }
    }

    /// Gets the OLA port field (for test builds to ensure field is not marked as dead code).
    #[cfg(test)]
    pub fn get_ola_port(&self) -> Option<u16> {
        self.ola_port
    }

    /// Gets the dimming speed modifier.
    pub fn dimming_speed_modifier(&self) -> f64 {
        self.dim_speed_modifier
            .unwrap_or(DEFAULT_DMX_DIMMING_SPEED_MODIFIER)
    }

    /// Gets the playback delay.
    pub fn playback_delay(&self) -> Result<Duration, Box<dyn Error>> {
        super::parse_playback_delay(&self.playback_delay, DEFAULT_DMX_PLAYBACK_DELAY)
    }

    /// Gets the OLA port to use.
    #[cfg(not(test))]
    pub fn ola_port(&self) -> u16 {
        self.ola_port.unwrap_or(DEFAULT_OLA_PORT)
    }

    /// Converts the configuration into universe configs.
    pub fn universes(&self) -> &[Universe] {
        &self.universes
    }

    /// Gets the lighting configuration.
    pub fn lighting(&self) -> Option<&Lighting> {
        self.lighting.as_ref()
    }

    /// Whether to fall back to a no-op OLA client when OLA is unavailable.
    pub fn null_client(&self) -> bool {
        self.null_client
    }

    /// Returns a mutable reference to the lighting configuration.
    pub fn lighting_mut(&mut self) -> Option<&mut Lighting> {
        self.lighting.as_mut()
    }

    /// Validates the DMX configuration for semantic issues.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if let Some(modifier) = self.dim_speed_modifier {
            if modifier <= 0.0 {
                errors.push("dmx dim_speed_modifier must be greater than 0".to_string());
            }
        }
        if let Some(ref delay) = self.playback_delay {
            if DurationString::from_string(delay.clone()).is_err() {
                errors.push(format!(
                    "dmx playback_delay '{}' is not a valid duration",
                    delay
                ));
            }
        }
        for (i, universe) in self.universes.iter().enumerate() {
            if universe.name.trim().is_empty() {
                errors.push(format!("dmx universe[{}]: name must not be empty", i));
            }
        }
        // Check for duplicate universe names.
        let mut seen_names = std::collections::HashSet::new();
        for universe in &self.universes {
            if !seen_names.insert(&universe.name) {
                errors.push(format!(
                    "dmx universe name '{}' is duplicated",
                    universe.name
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// A YAML representation of a DMX universe configuration.
#[derive(Deserialize, Serialize, Clone)]
pub struct Universe {
    /// The OpenLighting universe.
    universe: u16,

    /// The name of this universe. Will be mapped to a universe by the player.
    name: String,
}

impl Universe {
    /// Creates a new universe configuration.
    #[cfg(test)]
    pub fn new(universe: u16, name: String) -> Universe {
        Universe { universe, name }
    }

    /// Gets the OpenLighting universe.
    pub fn universe(&self) -> u16 {
        self.universe
    }

    /// Gets the name of the universe.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dmx_ola_port_field() {
        // Test that ola_port field can be set and retrieved
        let dmx = Dmx::new(None, None, Some(9090), vec![], None);
        assert_eq!(dmx.get_ola_port(), Some(9090));

        // Test with None
        let dmx = Dmx::new(None, None, None, vec![], None);
        assert_eq!(dmx.get_ola_port(), None);
    }

    #[test]
    fn dimming_speed_modifier_default() {
        let dmx = Dmx::new(None, None, None, vec![], None);
        assert!(
            (dmx.dimming_speed_modifier() - DEFAULT_DMX_DIMMING_SPEED_MODIFIER).abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn dimming_speed_modifier_custom() {
        let dmx = Dmx::new(Some(2.5), None, None, vec![], None);
        assert!((dmx.dimming_speed_modifier() - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn playback_delay_default() {
        let dmx = Dmx::new(None, None, None, vec![], None);
        assert_eq!(dmx.playback_delay().unwrap(), DEFAULT_DMX_PLAYBACK_DELAY);
    }

    #[test]
    fn playback_delay_valid() {
        let dmx = Dmx::new(None, Some("500ms".to_string()), None, vec![], None);
        assert_eq!(dmx.playback_delay().unwrap(), Duration::from_millis(500));
    }

    #[test]
    fn playback_delay_invalid() {
        let dmx = Dmx::new(None, Some("not_a_duration".to_string()), None, vec![], None);
        assert!(dmx.playback_delay().is_err());
    }

    #[test]
    fn null_client_default() {
        let dmx = Dmx::new(None, None, None, vec![], None);
        assert!(!dmx.null_client());
    }

    #[test]
    fn universes_empty() {
        let dmx = Dmx::new(None, None, None, vec![], None);
        assert!(dmx.universes().is_empty());
    }

    #[test]
    fn universes_populated() {
        let dmx = Dmx::new(
            None,
            None,
            None,
            vec![
                Universe::new(1, "front".to_string()),
                Universe::new(2, "back".to_string()),
            ],
            None,
        );
        let unis = dmx.universes();
        assert_eq!(unis.len(), 2);
        assert_eq!(unis[0].universe(), 1);
        assert_eq!(unis[0].name(), "front");
        assert_eq!(unis[1].universe(), 2);
        assert_eq!(unis[1].name(), "back");
    }

    #[test]
    fn lighting_none() {
        let dmx = Dmx::new(None, None, None, vec![], None);
        assert!(dmx.lighting().is_none());
    }

    #[test]
    fn lighting_some() {
        let lighting = Lighting::new(Some("venue1".to_string()), None, None, None);
        let dmx = Dmx::new(None, None, None, vec![], Some(lighting));
        assert!(dmx.lighting().is_some());
        assert_eq!(dmx.lighting().unwrap().current_venue(), Some("venue1"));
    }

    #[test]
    fn serde_round_trip() {
        let yaml = r#"
            dim_speed_modifier: 1.5
            playback_delay: "200ms"
            ola_port: 9020
            universes:
              - universe: 1
                name: main
              - universe: 2
                name: aux
            null_client: true
        "#;
        let dmx: Dmx = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert!((dmx.dimming_speed_modifier() - 1.5).abs() < f64::EPSILON);
        assert_eq!(dmx.playback_delay().unwrap(), Duration::from_millis(200));
        assert_eq!(dmx.get_ola_port(), Some(9020));
        assert_eq!(dmx.universes().len(), 2);
        assert!(dmx.null_client());
    }

    #[test]
    fn serde_minimal() {
        let yaml = r#"
            universes: []
        "#;
        let dmx: Dmx = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert!(
            (dmx.dimming_speed_modifier() - DEFAULT_DMX_DIMMING_SPEED_MODIFIER).abs()
                < f64::EPSILON
        );
        assert_eq!(dmx.playback_delay().unwrap(), DEFAULT_DMX_PLAYBACK_DELAY);
        assert!(!dmx.null_client());
        assert!(dmx.universes().is_empty());
        assert!(dmx.lighting().is_none());
    }
}
