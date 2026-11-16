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
//! Light shows and fixtures for tests.

use std::collections::HashMap;

use crate::lighting::effects::{DmxCommand, FixtureInfo};
use crate::lighting::parser::{parse_light_shows, LightShow};

/// The shows in `source`, in no particular order.
pub fn shows(source: &str) -> Vec<LightShow> {
    parse_light_shows(source)
        .expect("test show should parse")
        .into_values()
        .collect()
}

/// An RGB fixture in universe 1 at `address`: red, green, blue on its
/// first three channels.
pub fn rgb(name: &str, address: u16) -> FixtureInfo {
    let channels: HashMap<String, u16> = [("red", 1), ("green", 2), ("blue", 3)]
        .into_iter()
        .map(|(n, o)| (n.to_string(), o))
        .collect();
    FixtureInfo::new(
        name.to_string(),
        1,
        address,
        "RGB".to_string(),
        channels,
        None,
    )
}

pub fn create_test_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);
    channels.insert("white".to_string(), 5);
    channels.insert("strobe".to_string(), 6);

    FixtureInfo::new(
        name.to_string(),
        universe,
        address,
        "RGBW_Strobe".to_string(),
        channels,
        Some(20.0),
    )
}

/// The command written to `channel`. A level with no colour beneath it shows
/// white (design 3.4, rule 7), so the dimmer is no longer the only command.
pub fn on_channel(commands: &[DmxCommand], channel: u16) -> &DmxCommand {
    commands
        .iter()
        .find(|c| c.channel == channel)
        .expect("a command on the channel")
}
