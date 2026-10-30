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

//! The show's end against the lint's reckoning, for shows without holds,
//! and against a played-through engine going idle, for shows with them.

use super::*;
use crate::lighting::parser::parse_light_shows;

fn shows(source: &str) -> Vec<LightShow> {
    parse_light_shows(source)
        .expect("parse")
        .into_values()
        .collect()
}

fn rgb(name: &str, address: u16) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    FixtureInfo::new(
        name.to_string(),
        1,
        address,
        "RGB".to_string(),
        channels,
        None,
    )
}

/// Played through from 0 in 10 ms frames; the frame time from which no
/// effect is active through `until_ms` (None if one still is then).
fn live_end(source: &str, until_ms: u64) -> Option<Duration> {
    let mut tl = LightingTimeline::new(shows(source));
    let mut engine = EffectEngine::new();
    for (i, n) in ["a", "b", "c"].iter().enumerate() {
        engine.register_fixture(rgb(n, 1 + 10 * i as u16));
    }
    engine.set_song_time(Duration::ZERO);
    tl.start();
    let step = Duration::from_millis(10);
    let mut last_active: Option<u64> = None;
    let mut ms = 0;
    while ms <= until_ms {
        let t = Duration::from_millis(ms);
        let u = tl.update(t);
        apply_timeline_update(&mut engine, u, &mut |e| e);
        let _ = engine.update(step, Some(t));
        if engine.active_effects_count() > 0 {
            last_active = Some(ms);
        }
        ms += 10;
    }
    match last_active {
        Some(ms) if ms == until_ms => None,
        Some(ms) => Some(Duration::from_millis(ms + 10)),
        None => Some(Duration::ZERO),
    }
}

/// The show's end before holds counted: the lint's reckoning.
fn lint_end(source: &str) -> Duration {
    let cues: Vec<_> = shows(source).into_iter().flat_map(|s| s.cues).collect();
    let clears: Vec<_> = cues
        .iter()
        .flat_map(|cue| {
            cue.layer_commands
                .iter()
                .filter(|c| c.command_type == crate::lighting::parser::LayerCommandType::Clear)
                .map(move |c| (c.layer, cue.time))
        })
        .collect();
    let stops: Vec<_> = cues
        .iter()
        .flat_map(|cue| {
            cue.stop_sequences
                .iter()
                .map(move |n| (n.clone(), cue.time))
        })
        .collect();
    // Every clear and stop scanned for each effect, as the lint did before
    // it looked them up (#595): what the lookup must still come to.
    cues.iter()
        .flat_map(|cue| {
            let (clears, stops) = (&clears, &stops);
            cue.effects.iter().map(move |e| {
                let layer = e
                    .layer
                    .unwrap_or(crate::lighting::effects::EffectLayer::Background);
                let authored_end = cue.time + e.total_duration();
                let cleared_at = clears
                    .iter()
                    .filter(|(cleared, at)| *at > cue.time && cleared.is_none_or(|l| l == layer))
                    .map(|(_, at)| *at);
                let stopped_at = stops
                    .iter()
                    .filter(|(name, at)| {
                        Some(name.as_str()) == e.sequence_name.as_deref() && *at > cue.time
                    })
                    .map(|(_, at)| *at);
                cleared_at
                    .chain(stopped_at)
                    .filter(|at| *at < authored_end)
                    .min()
                    .unwrap_or(authored_end)
            })
        })
        .max()
        .unwrap_or(Duration::ZERO)
}

fn end(source: &str) -> Duration {
    LightingTimeline::new(shows(source)).show_end()
}

#[test]
fn show_end_full_clear_after_effect_over_with_unrelated_freeze() {
    // A 2 s look on background, over at 2 s. A freeze/unfreeze on
    // foreground (holding nothing), and a full clear at 10 s that cuts
    // nothing live.
    let show = "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    \
        @3.000\n    freeze(layer: foreground)\n    @4.000\n    unfreeze(layer: foreground)\n    \
        @10.000\n    clear()\n}\n";
    let live = live_end(show, 12000);
    assert_eq!(live, Some(Duration::from_secs(2)));
    assert_eq!(end(show), Duration::from_secs(2));
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
}

const LAYERS: [&str; 3] = ["background", "midground", "foreground"];

fn gen_show(rng: &mut Rng, holds: bool) -> String {
    let mut s = String::from(
        "sequence \"q\" {\n    @0.000\n    c: static blue: 100%, duration: 2s, layer: foreground\n    \
         @1.000\n    b: static green: 100%, duration: 1500ms, layer: midground\n}\n\n",
    );
    s.push_str("show \"s\" {\n");
    let n = 1 + rng.next() % 6;
    let mut times: Vec<u64> = (0..n).map(|_| (rng.next() % 17) * 500).collect();
    times.sort();
    for t in times {
        s.push_str(&format!("    @{}.{:03}\n", t / 1000, t % 1000));
        let items = 1 + rng.next() % 2;
        for _ in 0..items {
            let kinds: &[u32] = if holds {
                &[0, 0, 0, 1, 2, 3, 4, 5, 5, 6, 7]
            } else {
                &[0, 0, 0, 1, 2, 3, 4]
            };
            let layer = rng.pick(&LAYERS);
            match rng.pick(kinds) {
                0 => {
                    let fx = rng.pick(&["a", "b", "c"]);
                    let dur = rng.pick(&[
                        "duration: 1500ms",
                        "duration: 500ms",
                        "duration: 1s",
                        "duration: 2s",
                        "duration: 3s",
                        "up_time: 500ms, hold_time: 1s, down_time: 500ms",
                    ]);
                    s.push_str(&format!(
                        "    {fx}: static red: 100%, {dur}, layer: {layer}\n"
                    ));
                }
                1 => s.push_str(&format!("    clear(layer: {layer})\n")),
                2 => s.push_str("    clear()\n"),
                3 => s.push_str("    sequence \"q\"\n"),
                4 => s.push_str("    stop sequence \"q\"\n"),
                5 => s.push_str(&format!("    freeze(layer: {layer})\n")),
                6 => s.push_str(&format!("    unfreeze(layer: {layer})\n")),
                _ => {
                    let sp = rng.pick(&["0.5", "2.0", "1.0", "0.25"]);
                    s.push_str(&format!("    master(layer: {layer}, speed: {sp})\n"));
                }
            }
        }
    }
    s.push_str("}\n");
    s
}

/// A show without holds keeps the lint's end exactly, as before them; one
/// with holds ends within a frame of a played-through engine going idle.
/// (Without holds the lint's end can run past the live one: it counts a
/// stopped sequence's later cues, as it always has -- #594.)
fn fails(src: &str, until_ms: u64, hold_free: bool) -> Option<String> {
    let parsed = parse_light_shows(src);
    if parsed.is_err() {
        return None;
    }
    let head = end(src);
    if hold_free {
        let lint = lint_end(src);
        return (lint != head).then(|| format!("lint {lint:?} head {head:?}\n{src}"));
    }
    let Some(live) = live_end(src, until_ms) else {
        // Still lit when the run stopped: held for good, so with audio the
        // show must run to its end.
        let cap = Duration::from_secs(1000);
        let mut timeline = LightingTimeline::new(shows(src));
        timeline.set_end_cap(Some(cap));
        let capped = timeline.show_end();
        return (capped != cap).then(|| format!("still lit, capped show_end {capped:?}\n{src}"));
    };
    let diff = live.abs_diff(head);
    if diff > Duration::from_millis(10) {
        return Some(format!("live {live:?} head {head:?}\n{src}"));
    }
    None
}

#[test]
fn show_end_property_hold_free_show_end() {
    let mut rng = Rng(12345);
    let mut bad = Vec::new();
    for _ in 0..std::env::var("N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300)
    {
        let src = gen_show(&mut rng, false);
        if let Some(f) = fails(&src, 15000, true) {
            bad.push(f);
        }
    }
    assert!(
        bad.is_empty(),
        "{} failures: {:#?}",
        bad.len(),
        &bad[..bad.len().min(6)]
    );
}

#[test]
fn show_end_property_held_show_end() {
    let mut rng = Rng(777);
    let mut bad = Vec::new();
    for _ in 0..std::env::var("N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300)
    {
        let src = gen_show(&mut rng, true);
        if let Some(f) = fails(&src, 40000, false) {
            bad.push(f);
        }
    }
    assert!(
        bad.is_empty(),
        "{} failures: {:#?}",
        bad.len(),
        &bad[..bad.len().min(8)]
    );
}

fn fixtures3(engine: &mut EffectEngine) {
    for (i, n) in ["a", "b", "c"].iter().enumerate() {
        engine.register_fixture(rgb(n, 1 + 10 * i as u16));
    }
}

fn run(tl: &mut LightingTimeline, engine: &mut EffectEngine, from_ms: u64, until_ms: u64) {
    let step = Duration::from_millis(10);
    let mut ms = from_ms;
    while ms <= until_ms {
        let t = Duration::from_millis(ms);
        let u = tl.update(t);
        apply_timeline_update(engine, u, &mut |e| e);
        let _ = engine.update(step, Some(t));
        ms += 10;
    }
}

fn played3(source: &str, until_ms: u64) -> EffectEngine {
    let mut tl = LightingTimeline::new(shows(source));
    let mut engine = EffectEngine::new();
    fixtures3(&mut engine);
    engine.set_song_time(Duration::ZERO);
    tl.start();
    run(&mut tl, &mut engine, 0, until_ms);
    engine
}

/// As the DMX engine's start_lighting_timeline_at.
fn seek3(source: &str, seek_ms: u64, until_ms: u64) -> EffectEngine {
    let mut tl = LightingTimeline::new(shows(source));
    let mut engine = EffectEngine::new();
    fixtures3(&mut engine);
    let start = Duration::from_millis(seek_ms);
    let h = tl.start_at(start);
    engine.set_song_time(start);
    let _ = engine.apply_timeline_update(h);
    run(&mut tl, &mut engine, seek_ms, until_ms);
    engine
}

fn red(engine: &EffectEngine, f: &str) -> f64 {
    engine
        .get_fixture_states()
        .get(f)
        .and_then(|s| s.channels.get("red").map(|c| c.value))
        .unwrap_or(0.0)
}

const FREEZE_AT_END: &str =
    "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s, layer: midground\n    \
    @2.000\n    freeze(layer: midground)\n}\n";

#[test]
fn show_end_seek_onto_freeze_at_looks_last_instant() {
    let live = played3(FREEZE_AT_END, 3000);
    let seek = seek3(FREEZE_AT_END, 2000, 3000);
    assert_eq!(
        (red(&seek, "a"), seek.active_effects_count()),
        (red(&live, "a"), live.active_effects_count())
    );
}

#[test]
fn show_end_evaluate_onto_freeze_at_looks_last_instant() {
    let live = played3(FREEZE_AT_END, 2000);
    let ev = evaluate_show(
        shows(FREEZE_AT_END),
        &[rgb("a", 1)],
        &HashMap::new(),
        None,
        &[Duration::from_secs(2)],
        |e| e,
    )
    .remove(0);
    assert_eq!(ev.active_effects.len(), live.active_effects_count());
}

const CYCLE_SLOW_AT_END: &str = "show \"s\" {\n    @0.000\n    a: cycle color: \"red\", color: \"green\", speed: 1.0, duration: 2s, layer: midground\n    @2.000\n    master(layer: midground, speed: 0.5)\n}\n";

fn rg(engine: &EffectEngine) -> (f64, f64) {
    let s = engine.get_fixture_states();
    let st = s.get("a");
    let ch = |n: &str| {
        st.and_then(|s| s.channels.get(n).map(|c| c.value))
            .unwrap_or(0.0)
    };
    (ch("red"), ch("green"))
}

#[test]
fn show_end_seek_onto_slow_down_at_cycles_last_instant() {
    for at in [2000u64, 2010, 2100] {
        let live = played3(CYCLE_SLOW_AT_END, at);
        let seek = seek3(CYCLE_SLOW_AT_END, 2000, at);
        assert_eq!(rg(&seek), rg(&live), "at {at} ms");
    }
}

#[test]
fn show_end_edge_ends() {
    // Each show with the end it should have, checked against a played-through
    // engine too where that lights anything.
    let cases = [
        // No effects at all.
        ("show \"s\" {\n    @0.000\n    freeze(layer: foreground)\n    @3.000\n    unfreeze(layer: foreground)\n}\n", 0.0),
        // A clear in the effect's own cue (or a same-time cue) runs first.
        ("show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    clear()\n    @1.000\n    freeze(layer: foreground)\n}\n", 2.0),
        ("show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    @0.000\n    clear()\n    @1.000\n    freeze(layer: foreground)\n}\n", 2.0),
        // A look with no length is over at its cue, frozen layer or not.
        ("show \"s\" {\n    @0.000\n    a: static red: 100%, hold_time: 0s\n    @1.000\n    freeze(layer: foreground)\n    @2.000\n    unfreeze(layer: foreground)\n}\n", 0.0),
        ("show \"s\" {\n    @2.000\n    a: static red: 100%, hold_time: 0s, layer: midground\n    freeze(layer: midground)\n    @3.000\n    unfreeze(layer: midground)\n}\n", 2.0),
        // A full clear cuts a running look, and the freeze after holds nothing.
        ("show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    @1.000\n    clear()\n    freeze(layer: background)\n    @3.000\n    unfreeze(layer: background)\n}\n", 1.0),
    ];
    for (c, expected) in cases {
        let head = end(c);
        assert_eq!(head, Duration::from_secs_f64(expected), "{c}");
        if let Some(live) = live_end(c, 6000).filter(|live| !live.is_zero()) {
            assert!(
                live.abs_diff(head) <= Duration::from_millis(10),
                "live {live:?} show_end {head:?}\n{c}"
            );
        }
    }
}

#[test]
fn show_end_full_clear_variants() {
    let base = "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    @3.000\n    freeze(layer: foreground)\n    @4.000\n    unfreeze(layer: foreground)\n";
    let none = format!("{base}}}\n");
    let layer = format!("{base}    @10.000\n    clear(layer: background)\n}}\n");
    let full = format!("{base}    @10.000\n    clear()\n}}\n");
    let other = format!("{base}    @10.000\n    master(layer: foreground, speed: 1.0)\n}}\n");
    for (name, show) in [
        ("layer clear", &layer),
        ("full clear", &full),
        ("master", &other),
    ] {
        assert_eq!(end(show), end(&none), "{name}");
    }
}

#[test]
fn show_end_zero_length_cued_on_frozen_layer() {
    let show = "show \"s\" {\n    @2.000\n    a: static red: 100%, hold_time: 0s, layer: midground\n    freeze(layer: midground)\n    @3.000\n    unfreeze(layer: midground)\n}\n";
    let live = played3(show, 2000);
    assert_eq!(live.active_effects_count(), 0);
    assert_eq!(end(show), Duration::from_secs(2));
}

/// A look cued at 1.002 s and slowed to 0.1 at 1.008 s fire in the same
/// frame, and apply in turn: the look runs its 6 ms at full speed before
/// the change, then 3.994 s at a tenth, over at 40.948 s -- played through,
/// in the show's end and after a seek.
#[test]
fn show_end_slow_down_in_the_same_frame_as_the_cue() {
    let show = "show \"s\" {\n    @1.002\n    a: static red: 100%, duration: 4s\n    \
                @1.008\n    master(layer: background, speed: 0.1)\n}\n";
    let live = live_end(show, 45000).expect("ends");
    let head = end(show);
    assert!(
        head.abs_diff(Duration::from_millis(40948)) <= Duration::from_millis(1),
        "show_end {head:?}"
    );
    assert!(
        live.abs_diff(head) <= Duration::from_millis(10),
        "live {live:?} show_end {head:?}"
    );
}

/// The same with a fade sped up: played through it is where a seek has it.
#[test]
fn show_end_speed_up_in_the_same_frame_as_the_cue_plays_as_it_seeks() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 2s, hold_time: 1s, \
                layer: midground\n    @2.009\n    master(layer: midground, speed: 4)\n}\n";
    for at in [2100u64, 2300, 2500] {
        let live = rg(&played3(show, at));
        let seek = rg(&seek3(show, at, at));
        assert!(
            (live.0 - seek.0).abs() < 0.002,
            "at {at} ms: live {live:?} seek {seek:?}"
        );
    }
}

/// a freeze then a speed change in the frame a fade cued just
/// before them starts in: frozen 2 ms in, as a seek has it.
#[test]
fn show_end_same_frame_freeze_then_speed_change_in_the_cues_frame() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 20ms, hold_time: 5s, \
                layer: midground\n    @2.003\n    freeze(layer: midground)\n    \
                @2.007\n    master(layer: midground, speed: 4)\n}\n";
    for at in [2100u64, 2500] {
        let live = rg(&played3(show, at));
        let seek = rg(&seek3(show, at, at));
        assert!(
            (live.0 - seek.0).abs() < 0.01 && (live.0 - 0.1).abs() < 0.01,
            "at {at} ms: live {live:?} seek {seek:?}, want 0.1"
        );
    }
}

/// the freeze alone holds the fade 2 ms in.
#[test]
fn show_end_same_frame_freeze_alone_in_the_cues_frame() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 20ms, hold_time: 5s, \
                layer: midground\n    @2.003\n    freeze(layer: midground)\n}\n";
    for at in [2100u64, 2500] {
        let live = rg(&played3(show, at));
        let seek = rg(&seek3(show, at, at));
        assert!(
            (live.0 - seek.0).abs() < 0.01 && (live.0 - 0.1).abs() < 0.01,
            "at {at} ms: live {live:?} seek {seek:?}, want 0.1"
        );
    }
}

/// freeze, speed 0 then speed 1 in one frame (the speed 1 lifts
/// the freeze command's freeze too): frozen from 2.003 to 2.009.
#[test]
fn show_end_same_frame_freeze_then_speed_zero_and_back_in_the_cues_frame() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 100ms, hold_time: 5s, \
                layer: midground\n    @2.003\n    freeze(layer: midground)\n    \
                @2.005\n    master(layer: midground, speed: 0)\n    \
                @2.009\n    master(layer: midground, speed: 1)\n}\n";
    let live = rg(&played3(show, 2050));
    let seek = rg(&seek3(show, 2050, 2050));
    assert!(
        (live.0 - seek.0).abs() < 0.01,
        "live {live:?} seek {seek:?}"
    );
}

/// frozen and unfrozen, no speed change, in the cue's frame.
#[test]
fn show_end_same_frame_freeze_unfreeze_in_the_cues_frame() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 100ms, hold_time: 5s, \
                layer: midground\n    @2.004\n    freeze(layer: midground)\n    \
                @2.008\n    unfreeze(layer: midground)\n}\n";
    let live = rg(&played3(show, 2050));
    let seek = rg(&seek3(show, 2050, 2050));
    assert!(
        (live.0 - seek.0).abs() < 0.005,
        "live {live:?} seek {seek:?}"
    );
}

/// speed 0 and back, no freeze command, in the cue's frame.
#[test]
fn show_end_same_frame_speed_zero_and_back_in_the_cues_frame() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 100ms, hold_time: 5s, \
                layer: midground\n    @2.004\n    master(layer: midground, speed: 0)\n    \
                @2.008\n    master(layer: midground, speed: 1)\n}\n";
    let live = rg(&played3(show, 2050));
    let seek = rg(&seek3(show, 2050, 2050));
    assert!(
        (live.0 - seek.0).abs() < 0.005 && (live.0 - 0.45).abs() < 0.005,
        "live {live:?} seek {seek:?}"
    );
}

/// slowed, then frozen and unfrozen, in the cue's frame.
#[test]
fn show_end_same_frame_speed_change_then_freeze_unfreeze_in_the_cues_frame() {
    let show = "show \"s\" {\n    @2.001\n    a: static red: 100%, up_time: 100ms, hold_time: 5s, \
                layer: midground\n    @2.002\n    master(layer: midground, speed: 0.5)\n    \
                @2.004\n    freeze(layer: midground)\n    \
                @2.008\n    unfreeze(layer: midground)\n}\n";
    let live = rg(&played3(show, 2050));
    let seek = rg(&seek3(show, 2050, 2050));
    assert!(
        (live.0 - seek.0).abs() < 0.005,
        "live {live:?} seek {seek:?}"
    );
}

/// A show with a sequence on `loop:` `passes`, as the band's are: a 4-bar
/// verse looping under a long bed and beat hits, stopped at bar `stop` if
/// given, with `extra` cues after.
fn looping_show(passes: &str, stop: Option<u32>, extra: &str) -> String {
    let mut s =
        String::from("tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\n\n");
    s.push_str("sequence \"verse\" {\n");
    for bar in 1..=4 {
        for beat in 1..=4 {
            s.push_str(&format!(
                "    @{bar}/{beat}\n    a: static red: 100%, duration: 1beats, layer: midground\n"
            ));
        }
    }
    s.push_str(&format!(
        "}}\n\nshow \"s\" {{\n    @1/1\n    sequence \"verse\", loop: {passes}\n    \
         b: static blue: 100%, duration: 400measures\n"
    ));
    if let Some(bar) = stop {
        s.push_str(&format!("    @{bar}/1\n    stop sequence \"verse\"\n"));
    }
    for i in 0..60u32 {
        s.push_str(&format!(
            "    @{}/{}\n    c: static green: 100%, duration: 1beats, layer: foreground\n",
            2 + (i * 37) % 300,
            1 + i % 4
        ));
    }
    s.push_str(extra);
    s.push_str("}\n");
    s
}

/// Looked up rather than scanned, a show's clears and stops cut its effects
/// where they did (#595): the show's end is still the scan's.
#[test]
fn show_end_of_looping_shows_is_the_scans() {
    let sources = [
        looping_show("300", Some(100), ""),
        looping_show("300", None, ""),
        looping_show(
            "300",
            Some(40),
            "    @200/1\n    clear(layer: background)\n",
        ),
        looping_show("300", None, "    @250/3\n    clear()\n"),
        looping_show("300", None, "    @120/1\n    clear(layer: midground)\n"),
        looping_show("2", None, "    @3/1\n    stop sequence \"verse\"\n"),
    ];
    for src in &sources {
        assert_eq!(end(src), lint_end(src), "{src}");
    }
}

/// A sequence on `loop: loop` expands to 10,000 passes, each stopping the
/// one before. Scanning every stop for each effect they start took seconds
/// to build the timeline, holding up a crossfaded loop's next pass and the
/// ambient show's start (#595). A second or so in a debug build now, where
/// the scan took minutes.
#[test]
fn a_show_looping_for_good_builds_its_timeline_quickly() {
    let parsed = shows(&looping_show("loop", Some(100), ""));
    assert!(parsed[0].cues.len() > 160_000);
    let started = std::time::Instant::now();
    let timeline = LightingTimeline::new(parsed);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert!(timeline.show_end() > Duration::ZERO);
}
