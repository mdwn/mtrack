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

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use super::parser::{parse_fixture_types, parse_venues};
use super::types::{FixtureType, Group, Venue};
use crate::config::Lighting;

/// The lighting system configuration.
#[allow(dead_code)]
pub struct LightingSystem {
    /// Global fixture types.
    fixture_types: HashMap<String, FixtureType>,

    /// Venues.
    venues: HashMap<String, Venue>,

    /// Current venue.
    current_venue: Option<String>,

    /// Inline fixtures.
    inline_fixtures: HashMap<String, String>,

    /// Inline groups.
    inline_groups: HashMap<String, Vec<String>>,
}

#[allow(dead_code)]
impl LightingSystem {
    /// Creates a new lighting system.
    pub fn new() -> LightingSystem {
        LightingSystem {
            fixture_types: HashMap::new(),
            venues: HashMap::new(),
            current_venue: None,
            inline_fixtures: HashMap::new(),
            inline_groups: HashMap::new(),
        }
    }

    /// Loads the lighting configuration.
    pub fn load(&mut self, config: &Lighting, base_path: &Path) -> Result<(), Box<dyn Error>> {
        // Set current venue
        if let Some(venue) = config.current_venue() {
            self.current_venue = Some(venue.to_string());
        }

        // Load inline fixtures and groups
        self.inline_fixtures = config.fixtures();
        self.inline_groups = config.groups();

        // Load directories if configured
        if let Some(dirs) = config.directories() {
            if let Some(fixture_types_dir) = dirs.fixture_types() {
                let path = base_path.join(fixture_types_dir);
                self.load_fixture_types_directory(&path)?;
            }

            if let Some(venues_dir) = dirs.venues() {
                let path = base_path.join(venues_dir);
                self.load_venues_directory(&path)?;
            }
        }

        Ok(())
    }

    /// Loads fixture types from a directory.
    fn load_fixture_types_directory(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        if !dir.exists() {
            return Ok(()); // Directory doesn't exist, skip
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively load subdirectories
                self.load_fixture_types_directory(&path)?;
            } else if path.extension().is_some_and(|ext| ext == "light") {
                // Load .light files
                self.load_fixture_types_file(&path)?;
            }
        }
        Ok(())
    }

    /// Loads venues from a directory.
    fn load_venues_directory(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        if !dir.exists() {
            return Ok(()); // Directory doesn't exist, skip
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively load subdirectories
                self.load_venues_directory(&path)?;
            } else if path.extension().is_some_and(|ext| ext == "light") {
                // Load .light files
                self.load_venue_file(&path)?;
            }
        }
        Ok(())
    }

    /// Loads fixture types from a file.
    fn load_fixture_types_file(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let content = std::fs::read_to_string(path)?;

        match parse_fixture_types(&content) {
            Ok(types) => {
                for (name, fixture_type) in types {
                    self.fixture_types.insert(name, fixture_type);
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse fixture types from {}: {}",
                    path.display(),
                    e
                );
                // Continue loading other files
            }
        }

        Ok(())
    }

    /// Loads venues from a file.
    fn load_venue_file(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let content = std::fs::read_to_string(path)?;

        match parse_venues(&content) {
            Ok(venues) => {
                for (name, venue) in venues {
                    self.venues.insert(name, venue);
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse venues from {}: {}",
                    path.display(),
                    e
                );
                // Continue loading other files
            }
        }

        Ok(())
    }

    /// Parses fixture types from DSL content.
    pub fn parse_fixture_types(
        &self,
        content: &str,
    ) -> Result<HashMap<String, FixtureType>, Box<dyn Error>> {
        parse_fixture_types(content)
    }

    /// Parses venues from DSL content.
    pub fn parse_venues(&self, content: &str) -> Result<HashMap<String, Venue>, Box<dyn Error>> {
        parse_venues(content)
    }

    /// Gets the current venue.
    pub fn current_venue(&self) -> Option<&str> {
        self.current_venue.as_deref()
    }

    /// Gets a venue by name.
    pub fn get_venue(&self, name: &str) -> Option<&Venue> {
        self.venues.get(name)
    }

    /// Gets a group by name from the current venue.
    pub fn get_group(&self, group_name: &str) -> Result<&Group, Box<dyn Error>> {
        if let Some(venue_name) = self.current_venue() {
            if let Some(venue) = self.venues.get(venue_name) {
                if let Some(group) = venue.groups().get(group_name) {
                    return Ok(group);
                }
            }
        }
        Err(format!("Group '{}' not found in current venue", group_name).into())
    }
}
