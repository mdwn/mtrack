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

use std::collections::HashMap;
use std::fmt;

use serde::Serialize;

/// A fixture type definition.
#[derive(Clone, Serialize)]
pub struct FixtureType {
    /// The name of the fixture type.
    name: String,

    /// Channel mappings.
    channels: HashMap<String, u16>,

    /// Maximum strobe frequency in Hz (if supported).
    pub max_strobe_frequency: Option<f64>,

    /// Minimum strobe frequency in Hz (bottom of variable strobe range).
    pub min_strobe_frequency: Option<f64>,

    /// First DMX value where variable strobe begins.
    pub strobe_dmx_offset: Option<u8>,
}

impl FixtureType {
    /// Creates a new fixture type.
    pub fn new(name: String, channels: HashMap<String, u16>) -> FixtureType {
        FixtureType {
            name,
            channels,
            max_strobe_frequency: None,
            min_strobe_frequency: None,
            strobe_dmx_offset: None,
        }
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the channels.
    pub fn channels(&self) -> &HashMap<String, u16> {
        &self.channels
    }

    /// Gets the maximum strobe frequency.
    pub fn max_strobe_frequency(&self) -> Option<f64> {
        self.max_strobe_frequency
    }

    /// Gets the minimum strobe frequency.
    pub fn min_strobe_frequency(&self) -> Option<f64> {
        self.min_strobe_frequency
    }

    /// Gets the strobe DMX offset.
    pub fn strobe_dmx_offset(&self) -> Option<u8> {
        self.strobe_dmx_offset
    }
}

impl fmt::Display for FixtureType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "fixture_type \"{}\" {{", self.name)?;
        writeln!(f, "  channels: {}", self.channels.len())?;
        writeln!(f, "  channel_map: {{")?;
        let mut entries: Vec<_> = self.channels.iter().collect();
        entries.sort_by_key(|(_, v)| *v);
        for (i, (name, offset)) in entries.iter().enumerate() {
            let comma = if i + 1 < entries.len() { "," } else { "" };
            writeln!(f, "    \"{}\": {}{}", name, offset, comma)?;
        }
        writeln!(f, "  }}")?;
        if let Some(v) = self.max_strobe_frequency {
            writeln!(f, "  max_strobe_frequency: {v}")?;
        }
        if let Some(v) = self.min_strobe_frequency {
            writeln!(f, "  min_strobe_frequency: {v}")?;
        }
        if let Some(v) = self.strobe_dmx_offset {
            writeln!(f, "  strobe_dmx_offset: {v}")?;
        }
        write!(f, "}}")
    }
}

/// A fixture definition.
#[derive(Clone, Serialize)]
pub struct Fixture {
    /// The name of the fixture.
    name: String,

    /// The fixture type.
    fixture_type: String,

    /// The universe.
    universe: u16,

    /// The start channel.
    start_channel: u16,

    /// Tags/roles/capabilities associated with this fixture.
    tags: Vec<String>,
}

impl Fixture {
    /// Creates a new fixture.
    pub fn new(
        name: String,
        fixture_type: String,
        universe: u16,
        start_channel: u16,
        tags: Vec<String>,
    ) -> Fixture {
        Fixture {
            name,
            fixture_type,
            universe,
            start_channel,
            tags,
        }
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixture type.
    pub fn fixture_type(&self) -> &str {
        &self.fixture_type
    }

    /// Gets the universe.
    pub fn universe(&self) -> u16 {
        self.universe
    }

    /// Gets the start channel.
    pub fn start_channel(&self) -> u16 {
        self.start_channel
    }

    /// Gets the tags on this fixture.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
}

/// A group definition.
#[derive(Clone, Serialize)]
pub struct Group {
    /// The name of the group.
    name: String,

    /// The fixtures in the group.
    fixtures: Vec<String>,
}

impl Group {
    /// Creates a new group.
    pub fn new(name: String, fixtures: Vec<String>) -> Group {
        Group { name, fixtures }
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixtures.
    pub fn fixtures(&self) -> &[String] {
        &self.fixtures
    }
}

/// A venue definition.
#[derive(Clone, Serialize)]
pub struct Venue {
    /// The name of the venue.
    name: String,

    /// The fixtures in the venue.
    fixtures: HashMap<String, Fixture>,

    /// The groups in the venue.
    groups: HashMap<String, Group>,
}

impl Venue {
    /// Creates a new venue.
    pub fn new(
        name: String,
        fixtures: HashMap<String, Fixture>,
        groups: HashMap<String, Group>,
    ) -> Venue {
        Venue {
            name,
            fixtures,
            groups,
        }
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixtures.
    pub fn fixtures(&self) -> &HashMap<String, Fixture> {
        &self.fixtures
    }

    /// Gets the groups.
    pub fn groups(&self) -> &HashMap<String, Group> {
        &self.groups
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "venue \"{}\" {{", self.name)?;
        let mut fixtures: Vec<_> = self.fixtures.values().collect();
        fixtures.sort_by_key(|fix| (fix.universe, fix.start_channel));
        for fix in &fixtures {
            write!(
                f,
                "  fixture \"{}\" {} @ {}:{}",
                fix.name, fix.fixture_type, fix.universe, fix.start_channel
            )?;
            if !fix.tags.is_empty() {
                let tags: Vec<String> = fix.tags.iter().map(|t| format!("\"{t}\"")).collect();
                write!(f, " tags [{}]", tags.join(", "))?;
            }
            writeln!(f)?;
        }
        let mut groups: Vec<_> = self.groups.values().collect();
        groups.sort_by_key(|g| g.name());
        for group in &groups {
            writeln!(
                f,
                "  group \"{}\" = {}",
                group.name,
                group.fixtures.join(", ")
            )?;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FixtureType ────────────────────────────────────────────────

    #[test]
    fn fixture_type_new() {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        let ft = FixtureType::new("RGB Par".to_string(), channels);
        assert_eq!(ft.name(), "RGB Par");
        assert_eq!(ft.channels().len(), 3);
        assert_eq!(ft.max_strobe_frequency(), None);
        assert_eq!(ft.min_strobe_frequency(), None);
        assert_eq!(ft.strobe_dmx_offset(), None);
    }

    #[test]
    fn fixture_type_strobe_fields() {
        let mut ft = FixtureType::new("Strobe".to_string(), HashMap::new());
        ft.max_strobe_frequency = Some(25.0);
        ft.min_strobe_frequency = Some(1.0);
        ft.strobe_dmx_offset = Some(128);
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.min_strobe_frequency(), Some(1.0));
        assert_eq!(ft.strobe_dmx_offset(), Some(128));
    }

    // ── Fixture ────────────────────────────────────────────────────

    #[test]
    fn fixture_new() {
        let f = Fixture::new(
            "par1".to_string(),
            "RGB Par".to_string(),
            1,
            10,
            vec!["front".to_string(), "wash".to_string()],
        );
        assert_eq!(f.name(), "par1");
        assert_eq!(f.fixture_type(), "RGB Par");
        assert_eq!(f.universe(), 1);
        assert_eq!(f.start_channel(), 10);
        assert_eq!(f.tags(), &["front", "wash"]);
    }

    #[test]
    fn fixture_no_tags() {
        let f = Fixture::new("spot1".to_string(), "Spot".to_string(), 2, 1, vec![]);
        assert!(f.tags().is_empty());
    }

    // ── Group ──────────────────────────────────────────────────────

    #[test]
    fn group_new() {
        let g = Group::new(
            "front_wash".to_string(),
            vec!["par1".to_string(), "par2".to_string()],
        );
        assert_eq!(g.name(), "front_wash");
        assert_eq!(g.fixtures().len(), 2);
        assert_eq!(g.fixtures()[0], "par1");
    }

    #[test]
    fn group_empty_fixtures() {
        let g = Group::new("empty".to_string(), vec![]);
        assert!(g.fixtures().is_empty());
    }

    // ── Venue ──────────────────────────────────────────────────────

    #[test]
    fn venue_new() {
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "par1".to_string(),
            Fixture::new("par1".to_string(), "RGB".to_string(), 1, 1, vec![]),
        );

        let mut groups = HashMap::new();
        groups.insert(
            "all".to_string(),
            Group::new("all".to_string(), vec!["par1".to_string()]),
        );

        let v = Venue::new("Club".to_string(), fixtures, groups);
        assert_eq!(v.name(), "Club");
        assert_eq!(v.fixtures().len(), 1);
        assert!(v.fixtures().contains_key("par1"));
        assert_eq!(v.groups().len(), 1);
        assert!(v.groups().contains_key("all"));
    }
}
