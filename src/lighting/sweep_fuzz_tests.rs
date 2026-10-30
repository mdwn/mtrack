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

//! A randomized sweep of shows -- fades, cycles, moves, holds, clears and
//! sequences, cued off frame boundaries -- played through against a seek,
//! `evaluate_show` and the show's end. Ignored by default: run with
//! `--ignored` in release; `N`, `SEED` and the generator switches (`NEAR`,
//! `CLEARS`, `MOVES`, `HOLDS`, `SEQS`) narrow it. Known gap: a move cut
//! less than a frame after its cue, or a seek landing on the cut, can differ
//! by whether the move rendered a frame before it.

use super::*;
use crate::lighting::parser::{parse_light_shows, LayerCommandType};
use crate::lighting::types::{ChannelDef, PhysicalRange, PhysicalUnit};

fn shows(source: &str) -> Vec<LightShow> {
    parse_light_shows(source)
        .expect("parse")
        .into_values()
        .collect()
}

fn rgb(name: &str, address: u16) -> FixtureInfo {
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

const MOVER_ADDR: u16 = 41;

fn mover(name: &str) -> FixtureInfo {
    let channels: HashMap<String, u16> = [("pan", 1), ("tilt", 3), ("dimmer", 5)]
        .into_iter()
        .map(|(n, o)| (n.to_string(), o))
        .collect();
    let mut info = FixtureInfo::new(
        name.to_string(),
        1,
        MOVER_ADDR,
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

fn fixtures() -> Vec<FixtureInfo> {
    vec![rgb("a", 1), rgb("b", 11), rgb("c", 21), mover("m")]
}

fn fresh() -> EffectEngine {
    let mut engine = EffectEngine::new();
    for f in fixtures() {
        engine.register_fixture(f);
    }
    engine
}

const LABELS: [&str; 11] = [
    "a.r", "a.g", "a.b", "b.r", "b.g", "b.b", "c.r", "c.g", "c.b", "m.dim", "m.pan",
];

/// a/b/c red,green,blue, m dimmer (0-255) and m pan coarse.
fn snap(engine: &EffectEngine) -> Vec<f64> {
    let st = engine.get_fixture_states();
    let ch = |f: &str, c: &str| {
        st.get(f)
            .and_then(|s| s.channels.get(c).map(|c| c.value * 255.0))
            .unwrap_or(0.0)
    };
    let mut v = Vec::new();
    for f in ["a", "b", "c"] {
        for c in ["red", "green", "blue"] {
            v.push(ch(f, c));
        }
    }
    v.push(ch("m", "dimmer"));
    let pan = engine
        .last_commands()
        .iter()
        .find(|c| c.channel == MOVER_ADDR)
        .map(|c| c.value as f64)
        .unwrap_or(-1.0);
    v.push(pan);
    v
}

fn eval_snap(e: &Evaluation) -> Vec<f64> {
    let ch = |f: &str, c: &str| {
        e.fixtures
            .iter()
            .find(|x| x.name == f)
            .and_then(|x| x.channels.get(c).copied())
            .unwrap_or(0) as f64
    };
    let mut v = Vec::new();
    for f in ["a", "b", "c"] {
        for c in ["red", "green", "blue"] {
            v.push(ch(f, c));
        }
    }
    v.push(ch("m", "dimmer"));
    v.push(ch("m", "pan"));
    v
}

fn close(a: &[f64], b: &[f64], tol: f64, with_pan: bool) -> bool {
    let n = if with_pan { a.len() } else { a.len() - 1 };
    (0..n).all(|i| (a[i] - b[i]).abs() <= tol)
}

fn diff(a: &[f64], b: &[f64]) -> String {
    (0..a.len())
        .filter(|&i| (a[i] - b[i]).abs() > 2.0)
        .map(|i| format!("{}: {:.1} vs {:.1}", LABELS[i], a[i], b[i]))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Played through from 0, frames at phase, phase+10, ...; snapshot per
/// frame and how far the show is lit to in it, in ms (0 if not): to the
/// next frame while an effect is active, else to the last clear or stop of
/// a frame whose cues started an effect, which it may have cut before it
/// rendered -- lit then, if for less than a frame.
fn played_run(src: &str, phase: u64, until_ms: u64) -> Vec<(u64, Vec<f64>, usize)> {
    let mut tl = LightingTimeline::new(shows(src));
    let mut engine = fresh();
    engine.set_song_time(Duration::ZERO);
    tl.start();
    let step = Duration::from_millis(10);
    let mut out = Vec::new();
    let mut ms = phase;
    while ms <= until_ms {
        let t = Duration::from_millis(ms);
        let u = tl.update(t);
        let clears = u
            .layer_commands
            .iter()
            .zip(&u.layer_command_times)
            .filter(|(c, _)| c.command_type == LayerCommandType::Clear)
            .map(|(_, at)| *at);
        let cut = clears
            .chain(u.stop_sequence_times.iter().copied())
            .max()
            .filter(|_| !u.effects.is_empty())
            .map_or(0, |at| at.as_millis() as usize);
        apply_timeline_update(&mut engine, u, &mut |e| e);
        let _ = engine.update(step, Some(t));
        let lit = if engine.active_effects_count() > 0 {
            ms as usize + 10
        } else {
            cut
        };
        out.push((ms, snap(&engine), lit));
        ms += 10;
    }
    out
}

/// Seek to seek_ms as the DMX engine does, then frames seek, seek+10, ...;
/// snapshots at the given times (each seek_ms + 10k).
fn seek_run(src: &str, seek_ms: u64, at: &[u64]) -> Vec<(u64, Vec<f64>)> {
    let mut tl = LightingTimeline::new(shows(src));
    let mut engine = fresh();
    let start = Duration::from_millis(seek_ms);
    let h = tl.start_at(start);
    engine.set_song_time(start);
    let _ = engine.apply_timeline_update(h);
    let step = Duration::from_millis(10);
    let last = *at.iter().max().unwrap();
    let mut out = Vec::new();
    let mut ms = seek_ms;
    while ms <= last {
        let t = Duration::from_millis(ms);
        let u = tl.update(t);
        apply_timeline_update(&mut engine, u, &mut |e| e);
        let _ = engine.update(step, Some(t));
        if at.contains(&ms) {
            out.push((ms, snap(&engine)));
        }
        ms += 10;
    }
    out
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

fn flag(name: &str, default: bool) -> bool {
    std::env::var(name).map(|v| v != "0").unwrap_or(default)
}

fn fmt_t(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Returns (source, cue times).
fn gen(rng: &mut Rng) -> (String, Vec<u64>) {
    let near = flag("NEAR", true);
    let clears = flag("CLEARS", true);
    let moves = flag("MOVES", true);
    let holds = flag("HOLDS", true);
    let seqs = flag("SEQS", true);
    let mut s = String::from(
        "sequence \"q\" {\n    @0.000\n    c: static blue: 100%, duration: 1001ms, layer: foreground\n    \
         @0.707\n    b: cycle color: \"red\", color: \"green\", speed: 1.5, duration: 1400ms, layer: midground\n}\n\n",
    );
    s.push_str("show \"s\" {\n");
    let n = 2 + rng.next() % 6;
    let mut times: Vec<u64> = (0..n).map(|_| (rng.next() % 1100) * 7).collect();
    if near {
        for _ in 0..(rng.next() % 3) {
            let base = *rng.pick(&times);
            times.push(base + 1 + rng.next() % 9);
        }
    }
    times.sort();
    times.dedup();
    for &t in &times {
        s.push_str(&format!("    @{}\n", fmt_t(t)));
        let items = 1 + rng.next() % 2;
        for _ in 0..items {
            let mut kinds: Vec<u32> = vec![0, 0, 0, 1, 1];
            if moves {
                kinds.extend([2, 2]);
            }
            if clears {
                kinds.extend([3, 4]);
            }
            if seqs {
                kinds.push(5);
                if clears {
                    kinds.push(6);
                }
            }
            if holds {
                kinds.extend([7, 8, 9, 9]);
            }
            let layer = rng.pick(&LAYERS);
            let dur_ms = (1 + rng.next() % 400) * 7;
            match rng.pick(&kinds) {
                0 => {
                    let fx = rng.pick(&["a", "b", "c"]);
                    let col = rng.pick(&["red", "green", "blue"]);
                    let timing = if rng.next().is_multiple_of(2) {
                        format!("duration: {dur_ms}ms")
                    } else {
                        format!(
                            "up_time: {}ms, hold_time: {}ms, down_time: {}ms",
                            (rng.next() % 100) * 7,
                            (rng.next() % 150) * 7,
                            (rng.next() % 100) * 7
                        )
                    };
                    let bm = if rng.next().is_multiple_of(2) {
                        ", blend_mode: replace"
                    } else {
                        ""
                    };
                    s.push_str(&format!(
                        "    {fx}: static {col}: 100%, {timing}, layer: {layer}{bm}\n"
                    ));
                }
                1 => {
                    let fx = rng.pick(&["a", "b", "c"]);
                    let sp = rng.pick(&["0.5", "1.0", "2.0", "3.3"]);
                    s.push_str(&format!(
                        "    {fx}: cycle color: \"red\", color: \"green\", color: \"blue\", speed: {sp}, duration: {dur_ms}ms, layer: {layer}\n"
                    ));
                }
                2 => {
                    let from = rng.pick(&["-90", "0", "45"]);
                    let to = rng.pick(&["90", "-45", "180"]);
                    s.push_str(&format!(
                        "    m: move from_pan: {from}deg, from_tilt: 0deg, pan: {to}deg, tilt: 30deg, duration: {dur_ms}ms, layer: {layer}\n"
                    ));
                    if rng.next().is_multiple_of(2) {
                        s.push_str(&format!(
                            "    m: static dimmer: 100%, duration: {dur_ms}ms, layer: {layer}\n"
                        ));
                    }
                }
                3 => s.push_str(&format!("    clear(layer: {layer})\n")),
                4 => s.push_str("    clear()\n"),
                5 => s.push_str("    sequence \"q\"\n"),
                6 => s.push_str("    stop sequence \"q\"\n"),
                7 => s.push_str(&format!("    freeze(layer: {layer})\n")),
                8 => s.push_str(&format!("    unfreeze(layer: {layer})\n")),
                _ => {
                    let sp = rng.pick(&["0", "0.25", "0.5", "2", "4", "1"]);
                    s.push_str(&format!("    master(layer: {layer}, speed: {sp})\n"));
                }
            }
        }
    }
    s.push_str("}\n");
    (s, times)
}

#[test]
#[ignore]
fn sweep_random_shows() {
    let n: u64 = std::env::var("N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(300);
    let seed: u64 = std::env::var("SEED")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(4242);
    let only: Option<u64> = std::env::var("CASE").ok().and_then(|n| n.parse().ok());
    let do_seek = flag("SEEK", true);
    let do_eval = flag("EVAL", true);
    let do_end = flag("END", true);
    let mut rng = Rng(seed);
    let horizon = 30_000;
    let mut bad: Vec<String> = Vec::new();
    let mut counts = [0usize; 3];
    let mut failing_cases = [0usize; 3];
    for case in 0..n {
        let (src, times) = gen(&mut rng);
        // Seek points drawn regardless of the case filter, to keep the
        // stream aligned.
        let mut seeks: Vec<u64> = Vec::new();
        for _ in 0..3 {
            seeks.push(rng.next() % 12_000);
        }
        for _ in 0..2 {
            seeks.push(*rng.pick(&times));
        }
        let evals: Vec<u64> = (0..4).map(|_| (rng.next() % 1200) * 10).collect();
        if only.is_some_and(|c| c != case) {
            continue;
        }
        if parse_light_shows(&src).is_err() {
            bad.push(format!("case {case}: does not parse\n{src}"));
            continue;
        }
        let mut runs: HashMap<u64, Vec<(u64, Vec<f64>, usize)>> = HashMap::new();
        let mut run = |phase: u64| -> Vec<(u64, Vec<f64>, usize)> {
            runs.entry(phase)
                .or_insert_with(|| played_run(&src, phase, horizon))
                .clone()
        };
        let mut case_bad = [false; 3];
        // 1. Played through vs seek then play.
        if do_seek {
            for &s in &seeks {
                let phase = s % 10;
                let p = run(phase);
                let at: Vec<u64> = [0, 10, 370, 1500].iter().map(|k| s + k).collect();
                for (t, got) in seek_run(&src, s, &at) {
                    counts[0] += 1;
                    let i = ((t - phase) / 10) as usize;
                    let ok = [i.wrapping_sub(1), i, i + 1]
                        .iter()
                        .filter_map(|&j| p.get(j))
                        .any(|(_, v, _)| close(v, &got, 2.0, true));
                    if !ok {
                        case_bad[0] = true;
                        bad.push(format!(
                            "case {case} SEEK {s} at {t}: played vs sought [{}]\n{src}",
                            diff(&p[i].1, &got)
                        ));
                    }
                }
            }
        }
        // 2. Played through vs evaluate_show.
        if do_eval {
            let p = run(0);
            let ds: Vec<Duration> = evals.iter().map(|&t| Duration::from_millis(t)).collect();
            let ev = evaluate_show(shows(&src), &fixtures(), &HashMap::new(), None, &ds, |e| e);
            for (k, e) in ev.iter().enumerate() {
                counts[1] += 1;
                let t = evals[k];
                let i = (t / 10) as usize;
                let got = eval_snap(e);
                // Pan is checked only if evaluate reports it.
                let with_pan = e
                    .fixtures
                    .iter()
                    .any(|x| x.name == "m" && x.channels.contains_key("pan"));
                let ok = [i.wrapping_sub(1), i, i + 1]
                    .iter()
                    .filter_map(|&j| p.get(j))
                    .any(|(_, v, _)| close(v, &got, 2.0, with_pan));
                if !ok {
                    case_bad[1] = true;
                    bad.push(format!(
                        "case {case} EVAL at {t}: played vs evaluate [{}]\n{src}",
                        diff(&p[i].1, &got)
                    ));
                }
            }
        }
        // 3. show_end vs going idle.
        if do_end {
            counts[2] += 1;
            let p = run(0);
            let lit_until = p.iter().map(|(_, _, lit)| *lit as u64).max().unwrap_or(0);
            let last_active = (lit_until > 0).then_some(lit_until.saturating_sub(10));
            let head = LightingTimeline::new(shows(&src)).show_end();
            let msg = match last_active {
                Some(t) if t >= horizon => {
                    let cap = Duration::from_secs(1000);
                    let mut tl = LightingTimeline::new(shows(&src));
                    tl.set_end_cap(Some(cap));
                    let capped = tl.show_end();
                    (capped != cap).then(|| {
                        format!("still active at {horizon} ms, capped show_end {capped:?}")
                    })
                }
                _ => {
                    let live = Duration::from_millis(lit_until);
                    (live.abs_diff(head) > Duration::from_millis(10))
                        .then(|| format!("idle from {live:?}, show_end {head:?}"))
                }
            };
            if let Some(m) = msg {
                case_bad[2] = true;
                bad.push(format!("case {case} END: {m}\n{src}"));
            }
        }
        for k in 0..3 {
            failing_cases[k] += case_bad[k] as usize;
        }
    }
    eprintln!(
        "checks: seek {} eval {} end {}; failing cases: seek {} eval {} end {}",
        counts[0], counts[1], counts[2], failing_cases[0], failing_cases[1], failing_cases[2]
    );
    let shown: usize = std::env::var("SHOW")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(10);
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad[..bad.len().min(shown)].join("\n")
    );
}
