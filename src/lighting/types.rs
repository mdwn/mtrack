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

/// A fixture type definition.
#[derive(Clone)]
#[allow(dead_code)]
pub struct FixtureType {
    /// The name of the fixture type.
    name: String,

    /// Channel mappings.
    channels: HashMap<String, u16>,

    /// Special case handling.
    special_cases: Vec<String>,
}

#[allow(dead_code)]
impl FixtureType {
    /// Creates a new fixture type.
    pub fn new(
        name: String,
        channels: HashMap<String, u16>,
        special_cases: Vec<String>,
    ) -> FixtureType {
        FixtureType {
            name,
            channels,
            special_cases,
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

    /// Gets the special cases.
    pub fn special_cases(&self) -> &Vec<String> {
        &self.special_cases
    }
}

/// A fixture definition.
#[derive(Clone)]
#[allow(dead_code)]
pub struct Fixture {
    /// The name of the fixture.
    name: String,

    /// The fixture type.
    fixture_type: String,

    /// The universe.
    universe: u32,

    /// The start channel.
    start_channel: u16,

    /// Tags/roles/capabilities associated with this fixture.
    tags: Vec<String>,
}

#[allow(dead_code)]
impl Fixture {
    /// Creates a new fixture.
    pub fn new(
        name: String,
        fixture_type: String,
        universe: u32,
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
    pub fn universe(&self) -> u32 {
        self.universe
    }

    /// Gets the start channel.
    pub fn start_channel(&self) -> u16 {
        self.start_channel
    }

    /// Gets the tags on this fixture.
    pub fn tags(&self) -> &Vec<String> {
        &self.tags
    }
}

/// A group definition.
#[derive(Clone)]
#[allow(dead_code)]
pub struct Group {
    /// The name of the group.
    name: String,

    /// The fixtures in the group.
    fixtures: Vec<String>,
}

#[allow(dead_code)]
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
    pub fn fixtures(&self) -> &Vec<String> {
        &self.fixtures
    }
}

/// A venue definition.
#[derive(Clone)]
#[allow(dead_code)]
pub struct Venue {
    /// The name of the venue.
    name: String,

    /// The fixtures in the venue.
    fixtures: HashMap<String, Fixture>,

    /// The groups in the venue.
    groups: HashMap<String, Group>,
}

#[allow(dead_code)]
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
