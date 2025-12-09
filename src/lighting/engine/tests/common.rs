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
use crate::lighting::effects::FixtureInfo;
use std::collections::HashMap;

pub(crate) fn create_test_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);
    channels.insert("white".to_string(), 5);
    channels.insert("strobe".to_string(), 6);

    FixtureInfo {
        name: name.to_string(),
        universe,
        address,
        fixture_type: "RGBW_Strobe".to_string(),
        channels,
        max_strobe_frequency: Some(20.0), // Default test fixture max strobe
    }
}
