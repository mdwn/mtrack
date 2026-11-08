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

//! The reported fixture state is the bytes the universe holds: a channel
//! an ended effect left set reads as the wire holds it.

use std::collections::HashMap;
use std::error::Error;
use std::net::Ipv4Addr;
use std::time::Duration;

use super::{config, Engine};
use crate::dmx::ola_client::OlaClientFactory;
use crate::lighting::timeline::LightingTimeline;

/// An engine on universe 5 with a CMY wash (dimmer, cyan, magenta,
/// yellow, gobo, zoom at 1-6) and an RGB par (red, green, blue, dimmer,
/// focus, strobe at 7-12).
fn rig() -> Result<Engine, Box<dyn Error>> {
    let engine = Engine::new(
        &config::Dmx::new(
            None,
            None,
            Some(crate::testutil::net::free_tcp_port_on(
                Ipv4Addr::UNSPECIFIED,
            )),
            vec![config::Universe::new(5, "universe1".to_string())],
            None,
        ),
        None,
        None,
        OlaClientFactory::create_mock_client(),
    )?;
    let fixture =
        |name, address, channels| crate::testutil::lighting::generic(name, 5, address, channels);
    let mut effects = engine.effect_engine.lock();
    effects.register_fixture(fixture(
        "wash",
        1,
        &["dimmer", "cyan", "magenta", "yellow", "gobo", "zoom"],
    ));
    effects.register_fixture(fixture(
        "par",
        7,
        &["red", "green", "blue", "dimmer", "focus", "strobe"],
    ));
    drop(effects);
    Ok(engine)
}

/// Plays `source` from `at`: from the top for a song start, part way for a
/// seek.
fn play_from(engine: &Engine, source: &str, at: f64) {
    let shows = crate::lighting::parser::parse_light_shows(source).unwrap();
    engine.stop_lighting_timeline();
    *engine.current_song_timeline.lock() =
        Some(LightingTimeline::new(shows.into_values().collect()));
    engine.begin_playback_generation();
    engine.update_song_time(Duration::from_secs_f64(at));
    engine.start_lighting_timeline_at(Duration::from_secs_f64(at));
}

/// Seeks the playing song to `at`.
fn seek(engine: &Engine, at: f64) {
    engine.update_song_time(Duration::from_secs_f64(at));
    engine.start_lighting_timeline_at(Duration::from_secs_f64(at));
}

/// A frame at song time `secs`.
fn frame(engine: &Engine, secs: f64) {
    engine.update_song_time(Duration::from_secs_f64(secs));
    engine.effects_loop_tick();
}

/// The byte channel `channel` (1-based) of universe 5 holds.
fn wire(engine: &Engine, channel: u16) -> u8 {
    engine
        .get_universe(5)
        .unwrap()
        .get_target_value(usize::from(channel - 1)) as u8
}

/// Each fixture's channels as the live state reports them.
fn state(engine: &Engine) -> HashMap<String, HashMap<String, u8>> {
    let effects = engine.effect_engine.lock();
    crate::state::engine_snapshot(&effects, None, &HashMap::new())
        .fixtures
        .into_iter()
        .map(|f| (f.name, f.channels))
        .collect()
}

/// Every channel of every fixture reads, in the reported state, the byte
/// the universe holds -- a fixture no frame has written reads absent, and
/// its universe 0.
fn assert_state_is_wire(engine: &Engine, when: &str) {
    let state = state(engine);
    let registry = engine.effect_engine.lock().get_fixture_registry().clone();
    for (name, info) in &registry {
        for (channel, def) in &info.channel_defs {
            let on_wire = wire(engine, info.dmx_channel(def.offset).unwrap());
            let reported = state.get(name).and_then(|c| c.get(channel)).copied();
            assert_eq!(
                reported.unwrap_or(0),
                on_wire,
                "{when}: {name}.{channel} reads {reported:?}, the wire holds {on_wire}"
            );
        }
    }
}

const SHOW: &str = r#"show "song" {
    @0.000
    wash: static, dimmer: 100%, red: 100%, green: 0%, blue: 0%, gobo: 50%, zoom: 40%, duration: 5s
    par: static, dimmer: 100%, red: 100%, green: 50%, blue: 0%, focus: 30%, duration: 5s
    @5.000
    wash: static, dimmer: 100%, red: 100%, duration: 10s
    par: static, dimmer: 100%, red: 100%, duration: 10s
}
"#;

#[test]
fn an_ended_effects_channels_read_as_the_wire_holds_them() -> Result<(), Box<dyn Error>> {
    let engine = rig()?;
    play_from(&engine, SHOW, 0.0);
    for secs in [0.0, 0.1, 2.5, 4.99, 5.0, 5.01, 5.5, 6.0, 8.0] {
        frame(&engine, secs);
        assert_state_is_wire(&engine, &format!("playing, {secs}s"));
    }
    let state = state(&engine);
    let wash = &state["wash"];
    // The CMY head shows red: cyan open, magenta and yellow (levels)
    // released to open with the effect that set them; the gobo and zoom,
    // which nothing writes now, hold where it left them.
    assert_eq!(
        (wash["cyan"], wash["magenta"], wash["yellow"]),
        (0, 0, 0),
        "the flags as the wire holds them"
    );
    assert_eq!((wash["gobo"], wash["zoom"]), (128, 102));
    assert!(!wash.contains_key("red"), "a CMY head is sent no red");
    let par = &state["par"];
    assert_eq!((par["red"], par["green"], par["focus"]), (255, 0, 77));
    Ok(())
}

#[test]
fn after_a_seek_the_state_is_what_the_wire_holds() -> Result<(), Box<dyn Error>> {
    // Played past the hand-over, then sought back into the first look and
    // forward past it again: the gobo the universe holds is the one read.
    let engine = rig()?;
    play_from(&engine, SHOW, 0.0);
    frame(&engine, 1.0);
    frame(&engine, 6.0);
    for at in [2.0, 7.0, 0.5, 9.0] {
        seek(&engine, at);
        for secs in [at, at + 0.1, at + 0.5] {
            frame(&engine, secs);
            assert_state_is_wire(&engine, &format!("seek to {at}, {secs}s"));
        }
    }
    assert_eq!(state(&engine)["wash"]["gobo"], 128);

    // A rig that never played the first look: straight to 7 s, its gobo
    // was never sent.
    let fresh = rig()?;
    play_from(&fresh, SHOW, 7.0);
    frame(&fresh, 7.0);
    assert_state_is_wire(&fresh, "fresh seek to 7 s");
    assert_eq!(state(&fresh)["wash"]["gobo"], 0);
    Ok(())
}

#[test]
fn across_a_song_change_the_state_is_what_the_wire_holds() -> Result<(), Box<dyn Error>> {
    let engine = rig()?;
    play_from(&engine, SHOW, 0.0);
    frame(&engine, 1.0);
    // The next song drives the wash's colour only, after a cue-less start.
    let next = r#"show "next" {
    @1.000
    wash: static, dimmer: 100%, blue: 100%, duration: 10s
}
"#;
    play_from(&engine, next, 0.0);
    for secs in [0.0, 0.5, 1.0, 2.0] {
        frame(&engine, secs);
        assert_state_is_wire(&engine, &format!("next song, {secs}s"));
    }
    let wash = &state(&engine)["wash"];
    assert_eq!(wash["gobo"], 128, "the last song's gobo is still set");
    engine.stop_lighting_timeline();
    frame(&engine, 3.0);
    assert_state_is_wire(&engine, "stopped");
    Ok(())
}

/// What a made-up show may set on each fixture.
const MADE_UP: [(&str, &[&str]); 2] = [
    (
        "wash",
        &["dimmer", "red", "green", "blue", "gobo", "zoom", "cyan"],
    ),
    (
        "par",
        &["dimmer", "red", "green", "blue", "focus", "strobe"],
    ),
];

#[test]
fn across_made_up_shows_the_state_is_the_wire_on_every_channel() -> Result<(), Box<dyn Error>> {
    for seed in 1..=12 {
        let source = crate::testutil::lighting::made_up_show(seed, &MADE_UP);
        let engine = rig()?;
        play_from(&engine, &source, 0.0);
        let mut secs = 0.0;
        while secs < 9.0 {
            frame(&engine, secs);
            assert_state_is_wire(&engine, &format!("seed {seed}, {secs}s\n{source}"));
            secs += 0.023;
        }
        // And sought back part way.
        seek(&engine, 1.7);
        frame(&engine, 1.7);
        assert_state_is_wire(&engine, &format!("seed {seed}, after a seek\n{source}"));
    }
    Ok(())
}
