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

/// A fixture type definition.
#[derive(Clone)]
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
    pub fn new(
        name: String,
        channels: HashMap<String, u16>,
        _special_cases: Vec<String>,
    ) -> FixtureType {
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

/// A fixture definition.
#[derive(Clone)]
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
#[derive(Clone)]
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
#[derive(Clone)]
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
        let ft = FixtureType::new("RGB Par".to_string(), channels, vec![]);
        assert_eq!(ft.name(), "RGB Par");
        assert_eq!(ft.channels().len(), 3);
        assert_eq!(ft.max_strobe_frequency(), None);
        assert_eq!(ft.min_strobe_frequency(), None);
        assert_eq!(ft.strobe_dmx_offset(), None);
    }

    #[test]
    fn fixture_type_strobe_fields() {
        let mut ft = FixtureType::new("Strobe".to_string(), HashMap::new(), vec![]);
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

    #[test]
    fn venue_empty() {
        let v = Venue::new("Empty".to_string(), HashMap::new(), HashMap::new());
        assert_eq!(v.name(), "Empty");
        assert!(v.fixtures().is_empty());
        assert!(v.groups().is_empty());
    }
}
