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

/// A fixture in `universe` at `address` with `channels`, each on the next
/// channel from its first.
pub fn generic(name: &str, universe: u16, address: u16, channels: &[&str]) -> FixtureInfo {
    let channels: HashMap<String, u16> = channels
        .iter()
        .zip(1..)
        .map(|(c, offset)| (c.to_string(), offset))
        .collect();
    FixtureInfo::new(
        name.to_string(),
        universe,
        address,
        "Generic".to_string(),
        channels,
        None,
    )
}

/// A made-up show, the same for the same `seed`: eight cues over about
/// four seconds, each firing one to three statics -- on a random fixture
/// of `fixtures` (its name and the channels a static may set), random
/// channels of it at random levels, a random layer, a random length, now
/// and then fading or blending by `add` or `multiply` -- and now and then
/// a clear of a layer first. Besides, each kind of cue that can stop a
/// byte being written: a sequence holding a setting (a gobo, a zoom, a
/// focus) far longer than the show (the DSL requires a duration), stopped
/// by a later cue; a layer's speed changed, moving when its effects end;
/// and `mover` sent by a `move`, a second `move` taking it over part way,
/// and then, after it arrives, the whole rig cleared, so the head holds a
/// pose nothing remembers. Channels that change over time too: a `cycle`
/// of colours, and a setting fading up under an `add` blend. Last, a
/// setting set for 100 ms between two of the 250 ms instants a test
/// samples at, with no other cue in its life, so only the frame just
/// before its end sends its byte. A static that draws no channel is left
/// out. A setting is any channel that is not a level or colour
/// (`fades_to_dark`).
///
/// A setting fades up only, never down: one fading down to its effect's
/// end is still moving at the end, which a frame lands on only by chance
/// live, so what it holds after depends on the frame rate.
pub fn made_up_show(seed: u64, fixtures: &[(&str, &[&str])], mover: &str) -> String {
    let mut state = seed;
    let mut next = |n: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % n
    };
    let layers = ["background", "midground", "foreground"];
    // The settings (a gobo, a zoom, a focus) of `fixtures`: the first the
    // sequence holds, and nothing else writes, so the byte its stop leaves
    // shows; the rest fade.
    let settings: Vec<(&str, &str)> = fixtures
        .iter()
        .flat_map(|(fixture, names)| {
            names
                .iter()
                .filter(|n| !crate::lighting::channels::fades_to_dark(n))
                .map(move |n| (*fixture, *n))
        })
        .collect();
    let held = settings.first().copied();
    let setting = settings.get(1).copied();
    let mut out = String::new();
    if let Some((fixture, channel)) = held {
        // Still fading up when it is stopped, so the byte its stop leaves
        // is the one the frame at the stop sent, not an earlier frame's.
        out.push_str(&format!(
            "sequence \"held\" {{\n    @00:00.000\n    \
             {fixture}: static, {channel}: 100%, up_time: 4s, duration: 60s\n}}\n\n"
        ));
    }
    out.push_str("show \"made up\" {\n");
    let mut at = 0u64;
    let cycle_at = next(8);
    let move_at = next(4);
    let held_at = next(5);
    let held_stop = held_at + 1 + next(2);
    let speed_at = next(8);
    let fade_at = next(8);
    // When the second move arrives; the clear comes at the first cue after.
    let mut clear_after: Option<u64> = None;
    let mut cleared = false;
    for cue in 0..8 {
        out.push_str(&format!("    @{}.{:03}\n", at / 1000, at % 1000));
        if clear_after.is_some_and(|arrives| at > arrives) && !cleared {
            out.push_str("    clear()\n");
            cleared = true;
        }
        if next(6) == 0 {
            let layer = layers[next(3) as usize];
            out.push_str(&format!("    clear(layer: {layer})\n"));
        }
        if cue == speed_at {
            // The DSL has no `release`: a layer's speed moves when its
            // effects end instead.
            let layer = layers[next(3) as usize];
            let speed = [0.5, 2.0][next(2) as usize];
            out.push_str(&format!("    master(layer: {layer}, speed: {speed})\n"));
        }
        if held.is_some() {
            if cue == held_at {
                out.push_str("    sequence \"held\"\n");
            }
            if cue == held_stop {
                out.push_str("    stop sequence \"held\"\n");
            }
        }
        for _ in 0..=next(2) {
            let (fixture, names) = fixtures[next(fixtures.len() as u64) as usize];
            let mut params: Vec<String> = Vec::new();
            for name in names {
                if held == Some((fixture, *name)) {
                    continue;
                }
                if next(2) == 0 {
                    params.push(format!("{name}: {}%", next(101)));
                }
            }
            if params.is_empty() {
                continue;
            }
            let layer = layers[next(3) as usize];
            let mut length = 200 + next(1500);
            if next(3) == 0 {
                let settings = params.iter().any(|p| {
                    let name = p.split(':').next().unwrap_or_default();
                    !crate::lighting::channels::fades_to_dark(name)
                });
                let (up, down) = (next(400), next(400));
                if settings {
                    params.push(format!("up_time: {up}ms"));
                } else {
                    params.push(format!("up_time: {up}ms, down_time: {down}ms"));
                }
                length = length.max(up + down + 50);
            }
            match next(8) {
                0 => params.push("blend_mode: add".to_string()),
                1 => params.push("blend_mode: multiply".to_string()),
                _ => {}
            }
            out.push_str(&format!(
                "    {fixture}: static, {}, layer: {layer}, duration: {length}ms\n",
                params.join(", "),
            ));
        }
        if let Some((fixture, channel)) = setting.filter(|_| cue == fade_at) {
            let up = 100 + next(700);
            let length = up + 100 + next(800);
            let layer = layers[next(3) as usize];
            out.push_str(&format!(
                "    {fixture}: static, {channel}: {}%, up_time: {up}ms, \
                 blend_mode: add, layer: {layer}, duration: {length}ms\n",
                next(101),
            ));
        }
        if cue == cycle_at {
            let (fixture, _) = fixtures[next(fixtures.len() as u64) as usize];
            let layer = layers[next(3) as usize];
            let speed = 1 + next(4);
            let length = 400 + next(2000);
            out.push_str(&format!(
                "    {fixture}: cycle color: \"red\", color: \"green\", color: \"blue\", \
                 speed: {speed}.0, layer: {layer}, duration: {length}ms\n",
            ));
        }
        if cue == move_at {
            // Longer than the gap to the next cue, whose move takes over.
            let pan = next(361) as i64 - 180;
            let tilt = next(181) as i64 - 90;
            let length = 1100 + next(600);
            out.push_str(&format!(
                "    {mover}: move pan: {pan}deg, tilt: {tilt}deg, duration: {length}ms\n",
            ));
        }
        if cue == move_at + 1 {
            let pan = next(361) as i64 - 180;
            let tilt = next(181) as i64 - 90;
            let length = 200 + next(1000);
            out.push_str(&format!(
                "    {mover}: move pan: {pan}deg, tilt: {tilt}deg, duration: {length}ms\n",
            ));
            clear_after = Some(at + length);
        }
        at += 100 + next(900);
    }
    // A setting set briefly, alone on the rig (a clear first), between two
    // of the 250 ms instants a test samples at and with no other cue in its
    // life: no frame but the one just before its end sends its byte, which
    // the rig holds after.
    let mut after = at;
    if let Some((fixture, channel)) = setting {
        let brief = at.div_ceil(250) * 250 + 20;
        out.push_str(&format!(
            "    @{}.{:03}\n    clear()\n    {fixture}: static, {channel}: {}%, duration: 100ms\n",
            brief / 1000,
            brief % 1000,
            1 + next(100),
        ));
        after = brief + 150;
    }
    if !cleared {
        // After the last cue, and after the move arrives.
        let at = clear_after.map_or(after, |arrives| after.max(arrives + 100));
        out.push_str(&format!(
            "    @{}.{:03}\n    clear()\n",
            at / 1000,
            at % 1000
        ));
    }
    out.push_str("}\n");
    out
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
