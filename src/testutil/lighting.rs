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

//! Light shows, fixtures and effect engines for tests.

use std::collections::HashMap;

use crate::lighting::{
    effects::FixtureInfo,
    parser::{parse_light_shows, LightShow},
    types::{ChannelDef, PhysicalRange, PhysicalUnit},
    EffectEngine,
};

/// The shows in `source`, in no particular order.
pub fn shows(source: &str) -> Vec<LightShow> {
    parse_light_shows(source)
        .expect("test show should parse")
        .into_values()
        .collect()
}

/// The shows in `source`, by name: for a test whose result depends on
/// their order.
pub fn sorted_shows(source: &str) -> Vec<LightShow> {
    let mut parsed = shows(source);
    parsed.sort_by(|a, b| a.name.cmp(&b.name));
    parsed
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

/// A mover in universe 1 at `address`, hung 3 m up facing straight down:
/// 16-bit pan over ±270° (channels 1-2), tilt over ±135° (3-4), dimmer (5).
pub fn mover(name: &str, address: u16) -> FixtureInfo {
    let channels: HashMap<String, u16> = [("pan", 1), ("tilt", 3), ("dimmer", 5)]
        .into_iter()
        .map(|(n, o)| (n.to_string(), o))
        .collect();
    let mut info = FixtureInfo::new(
        name.to_string(),
        1,
        address,
        "Mover".to_string(),
        channels,
        None,
    );
    let mut defs = HashMap::new();
    defs.insert(
        "pan".to_string(),
        ChannelDef {
            offset: 1,
            fine: Some(2),
            range: Some(PhysicalRange {
                from: -270.0,
                to: 270.0,
                unit: PhysicalUnit::Degrees,
            }),
            functions: Vec::new(),
            mirrors: Vec::new(),
        },
    );
    defs.insert(
        "tilt".to_string(),
        ChannelDef {
            offset: 3,
            fine: Some(4),
            range: Some(PhysicalRange {
                from: -135.0,
                to: 135.0,
                unit: PhysicalUnit::Degrees,
            }),
            functions: Vec::new(),
            mirrors: Vec::new(),
        },
    );
    defs.insert("dimmer".to_string(), ChannelDef::at(5));
    info = info.with_channel_defs(defs);
    info.position = Some([0.0, 3.0, 0.0]);
    info.rotation = Some([0.0, 0.0, 0.0]);
    info
}

/// A fresh effect engine with `fixtures` registered.
pub fn engine_with(fixtures: &[FixtureInfo]) -> EffectEngine {
    let mut engine = EffectEngine::new();
    for fixture in fixtures {
        engine.register_fixture(fixture.clone());
    }
    engine
}

/// A GDTF file holding the synthetic fixture type's description.
pub fn synthetic_gdtf() -> Vec<u8> {
    synthetic_gdtf_of(crate::lighting::gdtf::SYNTHETIC_DESCRIPTION)
}

/// A GDTF file holding `description` as its `description.xml`.
pub fn synthetic_gdtf_of(description: &str) -> Vec<u8> {
    crate::lighting::gdtf::build_zip(&[("description.xml", description.as_bytes())])
}

/// [`synthetic_gdtf`] written to `synth.gdtf` in `dir`.
pub fn write_synthetic_gdtf(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("synth.gdtf");
    std::fs::write(&path, synthetic_gdtf()).unwrap();
    path
}
