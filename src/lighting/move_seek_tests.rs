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

//! Moving heads and looks on the instants layer commands land, played
//! through against a seek, `evaluate_show` and the show's end.

use super::*;
use crate::lighting::parser::parse_light_shows;
use crate::lighting::types::{ChannelDef, PhysicalRange, PhysicalUnit};

fn mover(name: &str) -> FixtureInfo {
    let channels: HashMap<String, u16> = [("pan", 1), ("tilt", 3), ("dimmer", 5)]
        .into_iter()
        .map(|(n, o)| (n.to_string(), o))
        .collect();
    let mut info = FixtureInfo::new(name.to_string(), 1, 1, "Mover".to_string(), channels, None);
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

fn rgb(name: &str) -> FixtureInfo {
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    FixtureInfo::new(name.to_string(), 1, 10, "RGB".to_string(), channels, None)
}

fn shows(source: &str) -> Vec<LightShow> {
    parse_light_shows(source)
        .expect("parse")
        .into_values()
        .collect()
}

fn run_frames(tl: &mut LightingTimeline, engine: &mut EffectEngine, from_ms: u64, until_ms: u64) {
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

fn fresh(fixtures: &[FixtureInfo]) -> EffectEngine {
    let mut engine = EffectEngine::new();
    for f in fixtures {
        engine.register_fixture(f.clone());
    }
    engine
}

/// Played through from 0 in exact 10 ms frames, as the effects loop does.
fn played(source: &str, fixtures: &[FixtureInfo], until_ms: u64) -> EffectEngine {
    let mut tl = LightingTimeline::new(shows(source));
    let mut engine = fresh(fixtures);
    engine.set_song_time(Duration::ZERO);
    tl.start();
    run_frames(&mut tl, &mut engine, 0, until_ms);
    engine
}

/// A seek to `seek_ms`, as the DMX engine's start_lighting_timeline_at does
/// it, then played on to `until_ms`.
fn seek_play(source: &str, fixtures: &[FixtureInfo], seek_ms: u64, until_ms: u64) -> EffectEngine {
    let mut tl = LightingTimeline::new(shows(source));
    let mut engine = fresh(fixtures);
    let start = Duration::from_millis(seek_ms);
    let h = tl.start_at(start);
    engine.set_song_time(start);
    let _ = engine.apply_timeline_update(h);
    run_frames(&mut tl, &mut engine, seek_ms, until_ms);
    engine
}

fn pan_of(engine: &EffectEngine) -> Option<f64> {
    let cmds = engine.last_commands();
    let coarse = cmds.iter().find(|c| c.channel == 1)?.value;
    let fine = cmds.iter().find(|c| c.channel == 2)?.value;
    let v = u16::from_be_bytes([coarse, fine]) as f64;
    Some(((-270.0 + v / 65535.0 * 540.0) * 10.0).round() / 10.0)
}

fn blue_of(engine: &EffectEngine) -> f64 {
    engine
        .get_fixture_states()
        .get("w")
        .and_then(|s| s.channels.get("blue").map(|c| c.value))
        .unwrap_or(0.0)
}

fn eval_one(source: &str, fixtures: &[FixtureInfo], secs: f64) -> Evaluation {
    evaluate_show(
        shows(source),
        fixtures,
        &HashMap::new(),
        None,
        &[Duration::from_secs_f64(secs)],
        |e| e,
    )
    .remove(0)
}

const MOVE_THEN_SLOW: &str = "show \"s\" {\n    @00:02.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s\n    \
    @00:04.000\n    master(layer: background, speed: 0.5)\n}\n";

#[test]
fn move_seek_move_ending_on_a_slow_down_seek_onto_it() {
    let f = [mover("m")];
    let live = pan_of(&played(MOVE_THEN_SLOW, &f, 4000));
    let seek = pan_of(&seek_play(MOVE_THEN_SLOW, &f, 4000, 4000));
    let later_live = pan_of(&played(MOVE_THEN_SLOW, &f, 4500));
    let later_seek = pan_of(&seek_play(MOVE_THEN_SLOW, &f, 4000, 4500));
    assert_eq!(seek, live, "seek onto 4 s");
    assert_eq!(later_seek, later_live, "seek onto 4 s, played to 4.5 s");
}

#[test]
fn move_seek_move_past_slow_down() {
    let f = [mover("m")];
    let live = pan_of(&played(MOVE_THEN_SLOW, &f, 4500));
    let seek = pan_of(&seek_play(MOVE_THEN_SLOW, &f, 4500, 4500));
    assert_eq!(seek, live);
}

fn mv(at: &str, extra: &str) -> String {
    format!("    @{at}\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s{extra}\n")
}

/// Compares (pan, active count) played through against a seek then play on.
fn compare_pan(name: &str, src: &str, pairs: &[(u64, u64)]) -> Vec<String> {
    let f = [mover("m")];
    let mut bad = Vec::new();
    for &(seek, at) in pairs {
        let l = played(src, &f, at);
        let s = seek_play(src, &f, seek, at);
        let lv = (pan_of(&l), l.get_active_effects().len());
        let sv = (pan_of(&s), s.get_active_effects().len());
        if lv.0 != sv.0 {
            bad.push(format!(
                "{name} seek {seek} at {at}: live {lv:?} seek {sv:?}"
            ));
        }
    }
    bad
}

fn compare_blue(name: &str, src: &str, pairs: &[(u64, u64)]) -> Vec<String> {
    let f = [rgb("w")];
    let mut bad = Vec::new();
    for &(seek, at) in pairs {
        let l = played(src, &f, at);
        let s = seek_play(src, &f, seek, at);
        let lv = (blue_of(&l), l.get_active_effects().len());
        let sv = (blue_of(&s), s.get_active_effects().len());
        if lv != sv {
            bad.push(format!(
                "{name} seek {seek} at {at}: live {lv:?} seek {sv:?}"
            ));
        }
        if seek == at {
            let ev = eval_one(src, &f, at as f64 / 1000.0);
            let ev_blue = ev
                .fixtures
                .iter()
                .find(|x| x.name == "w")
                .and_then(|x| x.channels.get("blue").copied())
                .unwrap_or(0);
            let evv = (ev_blue as f64 / 255.0, ev.active_effects.len());
            if evv != lv {
                bad.push(format!("{name} eval at {at}: live {lv:?} eval {evv:?}"));
            }
        }
    }
    bad
}

fn show(body: String) -> String {
    format!("show \"s\" {{\n{body}}}\n")
}

fn st(at: &str, d: &str) -> String {
    format!("    @{at}\n    w: static, color: \"blue\", duration: {d}\n")
}

#[test]
fn move_seek_battery() {
    let mut bad = Vec::new();
    // Move: master then clear in one cue, at its last instant.
    let s = show(mv("00:02.000", "") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n    clear(layer: background)\n");
    bad.extend(compare_pan(
        "move master+clear@end",
        &s,
        &[(4500, 4500), (4500, 6000)],
    ));
    // Move: clear alone at its last instant.
    let s = show(mv("00:02.000", "") + "    @00:04.000\n    clear(layer: background)\n");
    bad.extend(compare_pan("move clear@end", &s, &[(4500, 4500)]));
    // Move: freeze at its last instant, then slow down later.
    let s = show(mv("00:02.000", "") + "    @00:04.000\n    freeze(layer: background)\n    @00:05.000\n    master(layer: background, speed: 0.5)\n");
    bad.extend(compare_pan(
        "move freeze@end then slow",
        &s,
        &[(4500, 4500), (5000, 5000), (5500, 5500)],
    ));
    // Move: freeze then master in one cue at its last instant.
    let s = show(mv("00:02.000", "") + "    @00:04.000\n    freeze(layer: background)\n    master(layer: background, speed: 0.5)\n    @00:05.000\n    unfreeze(layer: background)\n");
    bad.extend(compare_pan(
        "move freeze+master@end",
        &s,
        &[
            (4000, 4000),
            (4500, 4500),
            (5000, 5000),
            (5500, 5500),
            (6500, 6500),
            (7500, 7500),
        ],
    ));
    // Move: sped up layer, ends at 3, master 1.0 at 3.
    let s = show(
        "    @00:01.500\n    master(layer: background, speed: 2.0)\n".to_string()
            + &mv("00:02.000", "")
            + "    @00:03.000\n    master(layer: background, speed: 1.0)\n",
    );
    bad.extend(compare_pan(
        "move fast ends on master 1",
        &s,
        &[(3000, 3000), (3500, 3500)],
    ));
    // Move: master 0 (freeze by speed) at its end, then master 0.5.
    let s = show(mv("00:02.000", "") + "    @00:04.000\n    master(layer: background, speed: 0.0)\n    @00:05.000\n    master(layer: background, speed: 0.5)\n");
    bad.extend(compare_pan(
        "move speed0@end then 0.5",
        &s,
        &[(4500, 4500), (5500, 5500)],
    ));
    // Move: master then unfreeze in one cue.
    let s = show(mv("00:02.000", "") + "    @00:03.000\n    freeze(layer: background)\n    @00:05.000\n    master(layer: background, speed: 0.5)\n    unfreeze(layer: background)\n");
    bad.extend(compare_pan(
        "move master+unfreeze",
        &s,
        &[(5000, 5000), (5500, 5500), (6500, 6500), (7500, 7500)],
    ));

    // Statics.
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    freeze(layer: background)\n    master(layer: background, speed: 0.5)\n    @00:05.000\n    unfreeze(layer: background)\n");
    bad.extend(compare_blue(
        "static freeze+master@end",
        &s,
        &[
            (4000, 4000),
            (4500, 4500),
            (5000, 5000),
            (5500, 5500),
            (6900, 6900),
            (7100, 7100),
        ],
    ));
    let s = show(st("00:02.000", "2s") + "    @00:03.000\n    freeze(layer: background)\n    @00:05.000\n    master(layer: background, speed: 0.5)\n    unfreeze(layer: background)\n");
    bad.extend(compare_blue(
        "static master+unfreeze",
        &s,
        &[(5000, 5000), (5500, 5500), (6900, 6900), (7100, 7100)],
    ));
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n    master(layer: background, speed: 1.0)\n");
    bad.extend(compare_blue(
        "static 0.5 then 1 @end",
        &s,
        &[(4000, 4000), (4500, 4500)],
    ));
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n    clear(layer: background)\n    @00:04.500\n    master(layer: background, speed: 0.25)\n");
    bad.extend(compare_blue(
        "static master+clear",
        &s,
        &[(4000, 4000), (4500, 4500), (5000, 5000)],
    ));
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    freeze(layer: background)\n    @00:06.000\n    unfreeze(layer: background)\n    master(layer: background, speed: 0.5)\n");
    bad.extend(compare_blue(
        "static freeze@end, later unfreeze+master",
        &s,
        &[(6000, 6000), (6500, 6500)],
    ));
    let s =
        show(st("00:02.000", "2s") + "    @00:04.000\n    master(layer: foreground, speed: 0.5)\n");
    bad.extend(compare_blue("other layer master", &s, &[(4000, 4000)]));
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    freeze(layer: background)\n    w: static, color: \"blue\", duration: 1s\n    @00:06.000\n    unfreeze(layer: background)\n");
    bad.extend(compare_blue(
        "cued with freeze",
        &s,
        &[(4000, 4000), (5000, 5000), (6500, 6500), (7100, 7100)],
    ));
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn move_seek_battery2() {
    let mut bad = Vec::new();
    let s = show(mv("00:02.000", "") + "    @00:04.000\n    freeze(layer: background)\n    @00:05.000\n    master(layer: background, speed: 0.5)\n");
    bad.extend(compare_pan(
        "move freeze@end seek onto",
        &s,
        &[(4000, 4000), (4000, 5500)],
    ));
    // A sequence move, slowed then stopped on its last instant.
    let s = "sequence \"q\" {\n    @00:00.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s\n}\n\n\
        show \"s\" {\n    @00:02.000\n    sequence \"q\"\n    @00:04.000\n    master(layer: background, speed: 0.5)\n    stop sequence \"q\"\n}\n";
    bad.extend(compare_pan("seq move master+stop@end", s, &[(4500, 4500)]));
    // Statics: seek exactly onto the unfreeze, and onto a freeze+master.
    let s = show(st("00:02.000", "2s") + "    @00:03.000\n    freeze(layer: background)\n    @00:05.000\n    unfreeze(layer: background)\n");
    bad.extend(compare_blue(
        "static seek onto unfreeze",
        &s,
        &[(5000, 5000), (5000, 5990), (5000, 6010)],
    ));
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    freeze(layer: background)\n    master(layer: background, speed: 0.5)\n");
    bad.extend(compare_blue(
        "static seek onto freeze+master",
        &s,
        &[(4000, 4000), (4000, 9000)],
    ));
    let s = show(st("00:02.000", "2s") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n    freeze(layer: background)\n    @00:05.000\n    unfreeze(layer: background)\n");
    bad.extend(compare_blue(
        "static master then freeze then unfreeze",
        &s,
        &[
            (4000, 4000),
            (4500, 4500),
            (5500, 5500),
            (6900, 6900),
            (7100, 7100),
        ],
    ));
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn move_seek_static_slow_down_seek_onto_it() {
    let src = "show \"s\" {\n    @00:02.000\n    w: static, color: \"blue\", duration: 2s\n    \
        @00:04.000\n    master(layer: background, speed: 0.5)\n}\n";
    let f = [rgb("w")];
    for (seek, at) in [(4000, 4000), (4000, 5000), (4500, 4500)] {
        let live = blue_of(&played(src, &f, at));
        let s = blue_of(&seek_play(src, &f, seek, at));
        assert_eq!(s, live, "seek {seek} at {at}");
    }
}

/// Live: the last frame (10 ms grid) with an effect still active, + 10 ms.
fn live_end(src: &str, fixtures: &[FixtureInfo], horizon_ms: u64) -> Duration {
    let mut tl = LightingTimeline::new(shows(src));
    let mut engine = fresh(fixtures);
    engine.set_song_time(Duration::ZERO);
    tl.start();
    let step = Duration::from_millis(10);
    let mut last_active = 0;
    let mut ms = 0;
    while ms <= horizon_ms {
        let t = Duration::from_millis(ms);
        let u = tl.update(t);
        apply_timeline_update(&mut engine, u, &mut |e| e);
        let _ = engine.update(step, Some(t));
        if !engine.get_active_effects().is_empty() {
            last_active = ms;
        }
        ms += 10;
    }
    Duration::from_millis(last_active + 10)
}

#[test]
fn move_seek_show_end() {
    let mut bad = Vec::new();
    let cases: Vec<(&str, String, bool)> = vec![
        ("static slowed at end", show(st("00:02.000", "2s") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n"), false),
        ("static freeze+master at end, unfreeze", show(st("00:02.000", "2s") + "    @00:04.000\n    freeze(layer: background)\n    master(layer: background, speed: 0.5)\n    @00:05.000\n    unfreeze(layer: background)\n"), false),
        ("move slowed at end", show(mv("00:02.000", "") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n"), true),
        ("move fast layer ends on master 1", show("    @00:01.500\n    master(layer: background, speed: 2.0)\n".to_string() + &mv("00:02.000", "") + "    @00:03.000\n    master(layer: background, speed: 1.0)\n"), true),
        ("static 0.5 then clear", show(st("00:02.000", "2s") + "    @00:04.000\n    master(layer: background, speed: 0.5)\n    clear(layer: background)\n"), false),
        ("static 2x then 0.5 same cue", show(st("00:02.000", "2s") + "    @00:03.000\n    master(layer: background, speed: 2.0)\n    master(layer: background, speed: 0.5)\n"), false),
        ("static freeze, master, unfreeze at end", show(st("00:02.000", "2s") + "    @00:03.000\n    freeze(layer: background)\n    @00:04.000\n    master(layer: background, speed: 0.5)\n    unfreeze(layer: background)\n"), false),
    ];
    for (name, src, is_move) in cases {
        let f = if is_move {
            vec![mover("m")]
        } else {
            vec![rgb("w")]
        };
        let live = live_end(&src, &f, 20000);
        let end = LightingTimeline::new(shows(&src)).show_end();
        let diff = live.abs_diff(end);
        if diff > Duration::from_millis(15) {
            bad.push(format!("{name}: live {live:?} show_end {end:?}"));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

fn mover_at(name: &str, address: u16) -> FixtureInfo {
    let mut info = mover(name);
    info.address = address;
    info
}

/// A head's pan as the engine remembers it: where it was last shown.
fn pan_in(engine: &EffectEngine, name: &str) -> Option<f64> {
    engine.poses().get(name).map(|pose| pose.pan)
}

fn eval_pan(source: &str, fixtures: &[FixtureInfo], ms: u64, name: &str) -> Option<f64> {
    eval_one(source, fixtures, ms as f64 / 1000.0)
        .poses
        .iter()
        .find(|pose| pose.name == name)
        .map(|pose| pose.pan)
}

/// Each head's pan played through, after a seek and from `evaluate_show`,
/// at each `(seek, at)`: the seek plays on to `at`, and evaluate is asked
/// for `at`. Within a frame's travel: live, a cut head holds the pose of
/// the frame before the cut, a seek the pose at the cut itself.
fn compare_heads(
    name: &str,
    src: &str,
    fixtures: &[FixtureInfo],
    pairs: &[(u64, u64)],
) -> Vec<String> {
    let close = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= 1.0,
        (a, b) => a == b,
    };
    let mut bad = Vec::new();
    for &(seek, at) in pairs {
        let live = played(src, fixtures, at);
        let sought = seek_play(src, fixtures, seek, at);
        for head in fixtures.iter().map(|f| f.name.as_str()) {
            let (l, s) = (pan_in(&live, head), pan_in(&sought, head));
            if !close(l, s) {
                bad.push(format!(
                    "{name} {head} seek {seek} at {at}: live {l:?} seek {s:?}"
                ));
            }
            // Evaluate reports a head with no pose at rest, at 0.
            let e = eval_pan(src, fixtures, at, head);
            if !close(l.or(Some(0.0)), e) {
                bad.push(format!("{name} {head} eval at {at}: live {l:?} eval {e:?}"));
            }
        }
    }
    bad
}

/// A layer clear part way through a move leaves the head where the clear
/// caught it, after a seek past the clear as played through (#596).
#[test]
fn a_layer_clear_part_way_through_a_move_holds_the_head_after_a_seek() {
    let src = show(mv("00:02.000", "") + "    @00:03.000\n    clear(layer: background)\n");
    let f = [mover("m")];
    let held = pan_in(&played(&src, &f, 3500), "m").expect("a pose");
    assert!(held > 30.0 && held < 60.0, "held at {held}");
    let bad = compare_heads(
        "layer clear",
        &src,
        &f,
        &[(3500, 3500), (4500, 4500), (3010, 5000), (4500, 6000)],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The same for `stop sequence` part way through a sequence's move (#596).
#[test]
fn a_stop_part_way_through_a_sequence_move_holds_the_head_after_a_seek() {
    let src = "sequence \"q\" {\n    @00:00.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s\n}\n\n\
        show \"s\" {\n    @00:02.000\n    sequence \"q\"\n    @00:03.000\n    stop sequence \"q\"\n}\n";
    let f = [mover("m")];
    let held = pan_in(&played(src, &f, 3500), "m").expect("a pose");
    assert!(held > 30.0 && held < 60.0, "held at {held}");
    let bad = compare_heads(
        "stop sequence",
        src,
        &f,
        &[(3500, 3500), (4500, 4500), (4500, 6000)],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A full `clear` part way through a move still lets the head go: the next
/// move without `from` snaps to its target rather than sweeping from where
/// the clear caught the head, after a seek as played through.
#[test]
fn a_full_clear_part_way_through_a_move_releases_the_pose_after_a_seek() {
    let src = show(
        mv("00:02.000", "")
            + "    @00:03.000\n    clear()\n    @00:04.000\n    m: move pan: 180deg, tilt: 0deg, duration: 2s\n",
    );
    let f = [mover("m")];
    assert_eq!(pan_in(&played(&src, &f, 3500), "m"), None);
    assert_eq!(pan_in(&played(&src, &f, 4500), "m"), Some(180.0));
    let bad = compare_heads(
        "full clear",
        &src,
        &f,
        &[(3500, 3500), (4500, 4500), (3500, 4500), (5000, 5000)],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

const LAYERED_MOVES: &str = "show \"s\" {\n    @6.587\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 180deg, tilt: 30deg, duration: 2772ms, layer: foreground\n    \
    @6.993\n    m: move from_pan: 45deg, from_tilt: 0deg, pan: -45deg, tilt: 30deg, duration: 1505ms, layer: midground\n}\n";

/// Two moves on different layers over one head: the later takes it, and
/// the head rests where that one ends, however it is reached (#599).
#[test]
fn the_latest_move_takes_a_head_whatever_its_layer() {
    let f = [mover("m")];
    assert_eq!(pan_in(&played(LAYERED_MOVES, &f, 9900), "m"), Some(-45.0));
    let live = played(LAYERED_MOVES, &f, 7500);
    assert_eq!(
        live.get_active_effects().len(),
        1,
        "the earlier move is over"
    );
    let bad = compare_heads(
        "layered moves",
        LAYERED_MOVES,
        &f,
        &[
            (6800, 6800),
            (7500, 7500),
            (7000, 8000),
            (9000, 9000),
            (9900, 9900),
            (6800, 10500),
        ],
    );
    assert!(bad.is_empty(), "{bad:#?}");
    // Over when the later move ends, not when the earlier would have.
    let end = LightingTimeline::new(shows(LAYERED_MOVES)).show_end();
    assert_eq!(end, Duration::from_millis(6993 + 1505));
}

/// A move over one of a two-head move's fixtures takes just that one: the
/// other runs on to its end, and the head taken carries on from where it
/// was, as the later move has no `from` (#599).
#[test]
fn a_move_takes_one_head_of_a_two_head_move() {
    let src = "show \"s\" {\n    @00:02.000\n    m1, m2: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:03.000\n    m2: move pan: -90deg, tilt: 0deg, duration: 2s, layer: background\n}\n";
    let f = [mover_at("m1", 1), mover_at("m2", 11)];
    let live = played(src, &f, 4500);
    assert_eq!(pan_in(&live, "m1"), Some(90.0));
    let m2 = pan_in(&live, "m2").expect("a pose");
    assert!(m2 > -90.0 && m2 < 45.0, "m2 sweeping from the cut, at {m2}");
    assert_eq!(pan_in(&played(src, &f, 5500), "m2"), Some(-90.0));
    let bad = compare_heads(
        "one head of two",
        src,
        &f,
        &[
            (3500, 3500),
            (4500, 4500),
            (3010, 4500),
            (5500, 5500),
            (2500, 6000),
        ],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

// ---- adversarial review ----

/// A show's own move and a sequence's move on one head at one time: live,
/// `apply_timeline_update` starts sequence effects first, so the show's own
/// move starts last and takes the head. The seek replay supersedes in cue
/// order, so the sequence's move, listed after it, takes the head instead.
const SAME_TIME_SEQ_AND_SHOW_MOVE: &str = "sequence \"q\" {\n    @00:00.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: -90deg, tilt: 0deg, duration: 4s\n}\n\n\
    show \"s\" {\n    @00:02.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s\n    sequence \"q\"\n}\n";

#[test]
fn a_show_move_beats_a_sequence_move_at_one_time_after_a_seek() {
    let f = [mover("m")];
    let live = pan_in(&played(SAME_TIME_SEQ_AND_SHOW_MOVE, &f, 7000), "m");
    let seek = pan_in(&seek_play(SAME_TIME_SEQ_AND_SHOW_MOVE, &f, 7000, 7000), "m");
    let mid_live = pan_in(&played(SAME_TIME_SEQ_AND_SHOW_MOVE, &f, 3000), "m");
    let mid_seek = pan_in(&seek_play(SAME_TIME_SEQ_AND_SHOW_MOVE, &f, 3000, 3000), "m");
    let eval = eval_pan(SAME_TIME_SEQ_AND_SHOW_MOVE, &f, 7000, "m");
    assert_eq!(
        (mid_seek, seek, eval),
        (mid_live, live, live),
        "seek/eval vs live (mid, rest, eval)"
    );
}

#[test]
fn a_show_move_beats_a_sequence_move_at_one_time_in_the_show_end() {
    let f = [mover("m")];
    // Live, the show's own 2 s move takes the head; nothing runs after 4 s.
    let live = played(SAME_TIME_SEQ_AND_SHOW_MOVE, &f, 4500);
    assert!(live.get_active_effects().is_empty(), "live idle at 4.5 s");
    let end = LightingTimeline::new(shows(SAME_TIME_SEQ_AND_SHOW_MOVE)).show_end();
    assert_eq!(end, Duration::from_secs(4));
}

#[test]
fn a_move_takes_a_head_from_one_frozen_or_slowed() {
    let f = [mover_at("m1", 1), mover_at("m2", 11)];
    let frozen = "show \"s\" {\n    @00:02.000\n    m1, m2: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:02.500\n    freeze(layer: foreground)\n    @00:03.000\n    m2: move pan: -90deg, tilt: 0deg, duration: 2s, layer: background\n    \
        @00:04.000\n    unfreeze(layer: foreground)\n}\n";
    let slowed = "show \"s\" {\n    @00:02.000\n    m1, m2: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:02.500\n    master(layer: foreground, speed: 0.5)\n    @00:03.000\n    m2: move pan: -90deg, tilt: 0deg, duration: 2s, layer: background\n}\n";
    let pairs = [
        (2800, 2800),
        (3500, 3500),
        (4500, 4500),
        (3010, 6000),
        (6000, 6000),
        (8000, 8000),
    ];
    let mut bad = compare_heads("frozen", frozen, &f, &pairs);
    bad.extend(compare_heads("slowed", slowed, &f, &pairs));
    assert!(bad.is_empty(), "{bad:#?}");
}

fn two_heads() -> [FixtureInfo; 2] {
    [mover_at("m1", 1), mover_at("m2", 11)]
}

/// Checks the show's end against the last frame live has an effect on.
fn end_check(name: &str, src: &str, f: &[FixtureInfo], bad: &mut Vec<String>) {
    let live = live_end(src, f, 20000);
    let end = LightingTimeline::new(shows(src)).show_end();
    if live.abs_diff(end) > Duration::from_millis(15) {
        bad.push(format!("{name} end: live {live:?} show_end {end:?}"));
    }
}

/// A seek exactly onto a layer clear part way through a move holds the
/// head where the move had it then, as played through: the move restored
/// part way has not rendered when the clear lands in the first frame.
#[test]
fn a_seek_onto_a_layer_clear_holds_the_head_where_the_move_had_it() {
    let src = show(mv("00:02.000", "") + "    @00:03.000\n    clear(layer: background)\n");
    let bad = compare_heads(
        "onto clear",
        &src,
        &[mover("m")],
        &[(3000, 3000), (3000, 3500), (3000, 5000)],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The same for a seek onto a `stop sequence` part way through its move.
#[test]
fn a_seek_onto_a_stop_holds_the_head_where_the_sequence_move_had_it() {
    let src = "sequence \"q\" {\n    @00:00.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s\n}\n\n\
        show \"s\" {\n    @00:02.000\n    sequence \"q\"\n    @00:03.000\n    stop sequence \"q\"\n}\n";
    let bad = compare_heads(
        "onto stop",
        src,
        &[mover("m")],
        &[(3000, 3000), (3000, 3500), (3000, 5000)],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

/// And the DMX sent after a seek onto that clear, from the first frame:
/// the head held where the clear caught it, then swept on by a later move
/// from there, not dropped.
#[test]
fn a_seek_onto_a_clear_sends_the_held_head_from_its_first_frame() {
    let src = show(mv("00:02.000", "") + "    @00:03.000\n    clear(layer: background)\n    @00:04.000\n    m: move pan: -90deg, tilt: 0deg, duration: 2s\n");
    let f = [mover("m")];
    let mut bad = Vec::new();
    for (seek, at) in [
        (3000, 3000),
        (3000, 3500),
        (3000, 4500),
        (3010, 3500),
        (3010, 4500),
    ] {
        let live = pan_of(&played(&src, &f, at));
        let sought = pan_of(&seek_play(&src, &f, seek, at));
        let close = matches!((live, sought), (Some(l), Some(s)) if (l - s).abs() <= 1.0);
        if !close {
            bad.push(format!(
                "seek {seek} at {at}: live {live:?} seek {sought:?}"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A seek onto a later move's cue that takes one head of a two-head move:
/// the head sweeps on from where the earlier move had it, rather than
/// snapping to the later move's target for want of a pose.
#[test]
fn a_seek_onto_a_later_move_sweeps_the_head_from_where_it_is() {
    let src = "show \"s\" {\n    @00:02.000\n    m1, m2: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:03.000\n    m2: move pan: -90deg, tilt: 0deg, duration: 2s, layer: background\n}\n";
    let bad = compare_heads(
        "onto supersede",
        src,
        &two_heads(),
        &[(3000, 3000), (3000, 3500), (3000, 4500), (3000, 6000)],
    );
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A layer clear of a move held by a freeze or slowed by the speed master,
/// sought onto and past: the head holds where the move had it, and the
/// show ends at the clear.
#[test]
fn a_clear_of_a_frozen_or_slowed_move_holds_the_head_after_a_seek() {
    let f = [mover("m")];
    let frozen = show(mv("00:02.000", "") + "    @00:02.500\n    freeze(layer: background)\n    @00:03.000\n    clear(layer: background)\n");
    let slowed = show(mv("00:02.000", "") + "    @00:02.500\n    master(layer: background, speed: 0.5)\n    @00:03.500\n    clear(layer: background)\n");
    let pairs = [
        (2800, 2800),
        (3000, 3000),
        (3010, 3500),
        (3500, 3500),
        (4000, 4000),
        (2700, 5000),
    ];
    let mut bad = compare_heads("frozen clear", &frozen, &f, &pairs);
    bad.extend(compare_heads("slowed clear", &slowed, &f, &pairs));
    end_check("frozen", &frozen, &f, &mut bad);
    end_check("slowed", &slowed, &f, &mut bad);
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A move on another layer taking the head from a frozen move, sought
/// onto: it sweeps from where the freeze holds the head, and the show ends
/// with it, the frozen move over.
#[test]
fn a_seek_onto_a_move_taking_the_head_from_a_frozen_move() {
    let f = [mover("m")];
    let src = "show \"s\" {\n    @00:02.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:02.500\n    freeze(layer: foreground)\n    @00:03.000\n    m: move pan: -90deg, tilt: 0deg, duration: 2s, layer: background\n}\n";
    let active = played(src, &f, 6000).get_active_effects().len();
    let mut bad = compare_heads(
        "frozen sup",
        src,
        &f,
        &[(3000, 3000), (3500, 3500), (6000, 6000)],
    );
    end_check("frozen sup", src, &f, &mut bad);
    assert!(bad.is_empty(), "{bad:#?} active at 6s {active}");
}

/// A clear in one show and a move in another at one time, in either file
/// order: the move sweeps from where the clear held the head.
#[test]
fn a_clear_and_a_move_at_one_time_in_two_shows_after_a_seek() {
    let f = [mover("m")];
    let a = format!(
        "show \"a\" {{\n{}    @00:03.000\n    clear(layer: background)\n}}\n",
        mv("00:02.000", "")
    );
    let b = "show \"b\" {\n    @00:03.000\n    m: move pan: -90deg, tilt: 0deg, duration: 2s\n}\n"
        .to_string();
    let pairs = [
        (3000, 3000),
        (3000, 4000),
        (3500, 3500),
        (4000, 4000),
        (2500, 6000),
        (6000, 6000),
    ];
    let mut bad = Vec::new();
    for (n, s) in [("ab", format!("{a}\n{b}")), ("ba", format!("{b}\n{a}"))] {
        bad.extend(compare_heads(n, &s, &f, &pairs));
        end_check(n, &s, &f, &mut bad);
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A sequence stopped and started again on one cue part way through its
/// move, then a show's move and another stop: each sought onto and past.
#[test]
fn a_sequence_stopped_and_restarted_at_one_time_after_a_seek() {
    let f = [mover("m")];
    let src = "sequence \"q\" {\n    @00:00.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s\n}\n\n\
        show \"s\" {\n    @00:02.000\n    sequence \"q\"\n    @00:03.000\n    stop sequence \"q\"\n    sequence \"q\"\n    @00:03.500\n    m: move pan: -90deg, tilt: 0deg, duration: 1s\n    stop sequence \"q\"\n}\n";
    let pairs = [
        (3000, 3000),
        (3200, 3200),
        (3500, 3500),
        (3500, 4000),
        (3700, 3700),
        (5000, 5000),
        (2500, 6000),
    ];
    let mut bad = compare_heads("stop restart", src, &f, &pairs);
    end_check("stop restart", src, &f, &mut bad);
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A move taking one head of a frozen two-head move, then an unfreeze and
/// a slow down: the other head runs on slowed from where it froze.
#[test]
fn an_unfreeze_after_a_head_is_taken_runs_the_other_on_after_a_seek() {
    let f = two_heads();
    let src = "show \"s\" {\n    @00:02.000\n    m1, m2: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:02.500\n    freeze(layer: foreground)\n    @00:03.000\n    m2: move pan: -90deg, tilt: 0deg, duration: 2s, layer: foreground\n    \
        @00:03.500\n    unfreeze(layer: foreground)\n    master(layer: foreground, speed: 0.5)\n}\n";
    let pairs = [
        (3000, 3000),
        (3200, 3200),
        (3500, 3500),
        (3500, 5000),
        (4000, 4000),
        (6000, 6000),
        (9000, 9000),
    ];
    let mut bad = compare_heads("unfreeze after cut", src, &f, &pairs);
    end_check("unfreeze after cut", src, &f, &mut bad);
    assert!(bad.is_empty(), "{bad:#?}");
}

/// A long show of overlapping moves over eight heads on three layers, with
/// slow downs and clears now and then: `n` cues 50 ms apart.
fn many_overlapping_moves(n: u64) -> String {
    let mut body = String::new();
    let layers = ["background", "midground", "foreground"];
    for i in 0..n {
        let ms = i * 50;
        let head = i % 8;
        body.push_str(&format!(
            "    @{}.{:03}\n    h{head}, h{}: move from_pan: 0deg, from_tilt: 0deg, pan: {}deg, tilt: 0deg, duration: 3s, layer: {}\n",
            ms / 1000,
            ms % 1000,
            (head + 1) % 8,
            (i % 180) as i64 - 90,
            layers[(i % 3) as usize]
        ));
        if i % 97 == 0 {
            body.push_str("    master(layer: midground, speed: 0.7)\n");
        }
        if i % 211 == 0 {
            body.push_str("    clear(layer: foreground)\n");
        }
    }
    show(body)
}

/// Builds the timeline for `n` cues of [`many_overlapping_moves`] and seeks
/// to near its end: how long each takes, and how many effects the seek
/// hands the engine.
fn build_and_seek(n: u64) -> (Duration, Duration, usize) {
    let s = shows(&many_overlapping_moves(n));
    let t = std::time::Instant::now();
    let mut tl = LightingTimeline::new(s);
    let built = t.elapsed();
    let t = std::time::Instant::now();
    let h = tl.start_at(Duration::from_millis(n * 50 - 100));
    (built, t.elapsed(), h.effects_with_elapsed.len())
}

/// A move that sets a head's pose outright, over another's, leaves nothing
/// of the earlier one's pose to keep, so the replay forgets that one:
/// building the timeline and seeking stay linear in the cues, and the seek
/// hands the engine only what still counts, not every move since the top
/// (#596, #599). 8000 cues took about 4 s in release while they piled up.
#[test]
fn a_long_show_of_moves_builds_and_seeks_quickly() {
    let (built, sought, resumed) = build_and_seek(8000);
    assert!(resumed <= 16, "{resumed} effects handed over");
    assert!(built < Duration::from_secs(5), "built in {built:?}");
    assert!(sought < Duration::from_secs(5), "sought in {sought:?}");
}

/// The same, `R2N` cues (4000 by default), timed.
#[test]
#[ignore]
fn time_a_long_show_of_moves() {
    let n: u64 = std::env::var("R2N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);
    let (built, sought, resumed) = build_and_seek(n);
    eprintln!("{n} cues: new {built:?}, start_at {sought:?}, {resumed} effects handed over");
}
