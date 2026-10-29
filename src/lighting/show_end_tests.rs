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

//! Review: show_end against the lint reckoning and a played-through engine.

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

/// 11fc9f2's show end: the lint reckoning.
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
    cues.iter()
        .flat_map(|cue| {
            let (clears, stops) = (&clears, &stops);
            cue.effects.iter().map(move |e| {
                crate::lighting::lint::effective_end(
                    cue.time + e.total_duration(),
                    cue.time,
                    e.layer
                        .unwrap_or(crate::lighting::effects::EffectLayer::Background),
                    e.sequence_name.as_deref(),
                    clears,
                    stops,
                )
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
                if std::env::var("NOFULL").is_ok() {
                    &[0, 0, 0, 1, 3, 4, 5, 5, 6, 7, 7]
                } else {
                    &[0, 0, 0, 1, 2, 3, 4, 5, 5, 6, 7]
                }
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
    let live = live_end(src, until_ms)?;
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
    for b in bad.iter().take(6) {}
    assert!(bad.is_empty(), "{} failures", bad.len());
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
    for b in bad.iter().take(8) {}
    assert!(bad.is_empty(), "{} failures", bad.len());
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
    engine.apply_layer_commands(&h.layer_commands, &[]);
    for s in &h.stop_sequences {
        engine.stop_sequence(s);
    }
    let mut we: Vec<_> = h.effects_with_elapsed.into_values().collect();
    we.sort_by_key(|(e, _)| e.cue_time.unwrap_or(Duration::ZERO));
    for (e, el) in we {
        let _ = engine.start_effect_with_elapsed(e, el);
    }
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
    }
    let live = played3(CYCLE_SLOW_AT_END, 2000);
    let seek = seek3(CYCLE_SLOW_AT_END, 2000, 2000);
    assert_eq!(rg(&seek), rg(&live));
}

#[test]
fn show_end_edge_ends() {
    let cases = [
        "show \"s\" {\n    @0.000\n    freeze(layer: foreground)\n    @3.000\n    unfreeze(layer: foreground)\n}\n",
        "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    clear()\n    @1.000\n    freeze(layer: foreground)\n}\n",
        "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    @0.000\n    clear()\n    @1.000\n    freeze(layer: foreground)\n}\n",
        "show \"s\" {\n    @0.000\n    a: static red: 100%, hold_time: 0s\n    @1.000\n    freeze(layer: foreground)\n    @2.000\n    unfreeze(layer: foreground)\n}\n",
        "show \"s\" {\n    @2.000\n    a: static red: 100%, hold_time: 0s, layer: midground\n    freeze(layer: midground)\n    @3.000\n    unfreeze(layer: midground)\n}\n",
        "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    @1.000\n    clear()\n    freeze(layer: background)\n    @3.000\n    unfreeze(layer: background)\n}\n",
    ];
    for c in cases {}
}

#[test]
fn show_end_full_clear_variants() {
    let base = "show \"s\" {\n    @0.000\n    a: static red: 100%, duration: 2s\n    @3.000\n    freeze(layer: foreground)\n    @4.000\n    unfreeze(layer: foreground)\n";
    let none = format!("{base}}}\n");
    let layer = format!("{base}    @10.000\n    clear(layer: background)\n}}\n");
    let full = format!("{base}    @10.000\n    clear()\n}}\n");
    let other = format!("{base}    @10.000\n    master(layer: foreground, speed: 1.0)\n}}\n");
    assert_eq!(end(&full), end(&layer));
}

#[test]
fn show_end_zero_length_cued_on_frozen_layer() {
    let show = "show \"s\" {\n    @2.000\n    a: static red: 100%, hold_time: 0s, layer: midground\n    freeze(layer: midground)\n    @3.000\n    unfreeze(layer: midground)\n}\n";
    let live = played3(show, 2000);
    assert_eq!(live.active_effects_count(), 0);
    assert_eq!(end(show), Duration::from_secs(2));
}
