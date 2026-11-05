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
//! The exports answer exactly what the player's own functions answer: every
//! measure and (half-)beat of every grid and tempo map here, and a sweep of
//! times through each. The cases are the web UI's mock songs
//! (`e2e/mock-server/test-data.ts`), the grids the player's own tests use,
//! and tempo maps with every feature a `tempo:` block has (lead-in, snap and
//! ramped tempo changes, signature changes on and off the bar line).
//!
//! With `MTRACK_RULES_DUMP=<path>` the inputs and answers are also written
//! as JSON, for a one-off comparison against another implementation.

use std::time::Duration;

use mtrack_rules_wasm::metronome_config::{MetronomeChange, MetronomeConfig, Subdivision};
use mtrack_rules_wasm::tempo_config::{TempoChangeConfig, TempoConfig, TransitionConfig};
use mtrack_rules_wasm::{
    bounds, signature_parse, BeatGrid, GridQuery, NormalizedTempo, Snap, TempoQuery, TempoSource,
};
use serde::Serialize;
use serde_json::json;

/// A grid from measure beat-counts at a steady tempo, starting at `lead_in`
/// seconds, with a trailing downbeat so the last measure is complete.
fn steady_grid(lead_in: f64, bpm: f64, measures: &[usize]) -> BeatGrid {
    let spacing = 60.0 / bpm;
    let mut beats = Vec::new();
    let mut measure_starts = Vec::new();
    let mut t = lead_in;
    for count in measures {
        measure_starts.push(beats.len());
        for _ in 0..*count {
            beats.push(t);
            t += spacing;
        }
    }
    measure_starts.push(beats.len());
    beats.push(t);
    BeatGrid {
        beats,
        measure_starts,
    }
}

fn grids() -> Vec<(&'static str, BeatGrid)> {
    let mut grids = vec![
        // Test Song Beta / Gamma: 16 measures of 4/4 at 120 BPM, no trailing
        // downbeat (the grid ends inside its last measure).
        (
            "mock-beta",
            BeatGrid {
                beats: (0..64).map(|i| i as f64 * 0.5).collect(),
                measure_starts: (0..16).map(|i| i * 4).collect(),
            },
        ),
        ("steady-4-4", steady_grid(0.0, 120.0, &[4, 4, 4, 4])),
        ("lead-in-3-4", steady_grid(1.25, 90.0, &[3, 3, 3])),
        ("short-bar", steady_grid(0.0, 80.0, &[4, 4, 4, 2, 4, 4])),
        ("seven-eight", steady_grid(0.0, 160.0, &[7, 7, 7])),
        (
            "pickup",
            BeatGrid {
                beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0],
                measure_starts: vec![1, 5],
            },
        ),
        (
            "empty",
            BeatGrid {
                beats: vec![],
                measure_starts: vec![],
            },
        ),
    ];
    for (name, tempo) in tempos() {
        if let Ok(map) = tempo.to_tempo_map() {
            grids.push((
                name,
                BeatGrid::from_tempo_map(&map, Duration::from_secs(40)),
            ));
        }
    }
    grids
}

fn change(measure: u32, beat: Option<f64>) -> TempoChangeConfig {
    TempoChangeConfig {
        measure,
        beat,
        bpm: None,
        time_signature: None,
        transition: None,
    }
}

fn tempos() -> Vec<(&'static str, TempoConfig)> {
    vec![
        (
            "plain",
            TempoConfig {
                bpm: 120.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![],
            },
        ),
        (
            "lead-in-six-eight",
            TempoConfig {
                bpm: 96.0,
                time_signature: "6/8".into(),
                start: Some(0.35),
                changes: vec![],
            },
        ),
        (
            "snap-and-ramp",
            TempoConfig {
                bpm: 118.0,
                time_signature: "4/4".into(),
                start: Some(0.35),
                changes: vec![
                    TempoChangeConfig {
                        bpm: Some(126.0),
                        transition: Some(TransitionConfig::Measures(2.0)),
                        ..change(5, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(96.0),
                        time_signature: Some("6/8".into()),
                        ..change(9, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(140.0),
                        transition: Some(TransitionConfig::Beats(4.0)),
                        ..change(12, None)
                    },
                ],
            },
        ),
        (
            "mid-bar-signature",
            TempoConfig {
                bpm: 100.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![
                    TempoChangeConfig {
                        time_signature: Some("7/8".into()),
                        ..change(3, Some(3.0))
                    },
                    TempoChangeConfig {
                        bpm: Some(110.0),
                        ..change(6, Some(2.5))
                    },
                    TempoChangeConfig {
                        time_signature: Some("3/4".into()),
                        ..change(8, None)
                    },
                ],
            },
        ),
        (
            "unsorted",
            TempoConfig {
                bpm: 120.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![
                    TempoChangeConfig {
                        bpm: Some(100.0),
                        ..change(10, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(90.0),
                        ..change(5, None)
                    },
                ],
            },
        ),
        (
            "repeated",
            TempoConfig {
                bpm: 120.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![
                    TempoChangeConfig {
                        bpm: Some(100.0),
                        ..change(5, None)
                    },
                    TempoChangeConfig {
                        bpm: Some(90.0),
                        ..change(5, Some(1.0))
                    },
                ],
            },
        ),
        (
            "bad-signature",
            TempoConfig {
                bpm: 120.0,
                time_signature: "5/3".into(),
                start: None,
                changes: vec![],
            },
        ),
        (
            "zero-bpm",
            TempoConfig {
                bpm: 0.0,
                time_signature: "4/4".into(),
                start: None,
                changes: vec![],
            },
        ),
    ]
}

fn metronomes() -> Vec<(&'static str, MetronomeConfig)> {
    vec![
        ("default", MetronomeConfig::default()),
        (
            "grouping",
            MetronomeConfig {
                accent: vec![3, 2, 2],
                ..MetronomeConfig::default()
            },
        ),
        (
            "pattern",
            MetronomeConfig {
                accents: vec![3, 0, 2],
                subdivision: Subdivision::Even(2),
                ..MetronomeConfig::default()
            },
        ),
        (
            "changes",
            MetronomeConfig {
                accents: vec![3, 1, 2, 1],
                changes: vec![
                    MetronomeChange {
                        measure: 5,
                        accents: Some(vec![3, 2]),
                        subdivision: None,
                    },
                    MetronomeChange {
                        measure: 2,
                        accents: None,
                        subdivision: Some(Subdivision::Even(3)),
                    },
                    MetronomeChange {
                        measure: 5,
                        accents: Some(vec![2, 2, 2, 3, 3, 3, 3, 3]),
                        subdivision: None,
                    },
                ],
                ..MetronomeConfig::default()
            },
        ),
    ]
}

fn half_beats(upto: usize) -> Vec<f64> {
    (0..=upto * 2).map(|i| 1.0 + i as f64 * 0.5).collect()
}

fn secs_sweep(end: f64) -> Vec<f64> {
    let mut secs = vec![-1.0, 0.0];
    let mut t = 0.0;
    while t < end + 2.0 {
        secs.push(t);
        t += 0.137;
    }
    secs
}

#[derive(Serialize)]
struct Dump {
    grids: Vec<DumpGrid>,
    tempos: Vec<DumpTempo>,
    metronomes: Vec<DumpMetronome>,
    signatures: Vec<(String, Option<(u32, u32)>)>,
}

#[derive(Serialize)]
struct DumpGrid {
    name: String,
    grid: BeatGrid,
    positions: Vec<((u32, f64), Option<mtrack_rules_wasm::GridPosition>)>,
    times: Vec<(f64, Option<mtrack_rules_wasm::GridPosition>)>,
    /// The same times snapped to whole beats.
    snapped: Vec<(f64, Option<mtrack_rules_wasm::GridPosition>)>,
}

#[derive(Serialize)]
struct DumpTempo {
    name: String,
    config: TempoConfig,
    verdict: Option<String>,
    normalized: NormalizedTempo,
    positions: Vec<((u32, f64), Option<mtrack_rules_wasm::TempoPoint>)>,
    times: Vec<(f64, Option<mtrack_rules_wasm::TempoPoint>)>,
}

#[derive(Serialize)]
struct DumpMetronome {
    name: String,
    config: MetronomeConfig,
    patterns: Vec<((u32, u32), Vec<u8>)>,
}

#[test]
fn exports_equal_the_players_rules() {
    let mut dump = Dump {
        grids: vec![],
        tempos: vec![],
        metronomes: vec![],
        signatures: vec![],
    };

    for (name, grid) in grids() {
        let measures = grid.measure_starts.len() + 1;
        let mut positions = vec![];
        for measure in 0..=measures as u32 {
            for beat in half_beats(9) {
                let got = GridQuery::Position {
                    measure,
                    beat,
                    snap: None,
                }
                .answer(&grid);
                // `beat_time` and `beats_in_measure` for a bar; the count-in
                // (measure 0) is `time_of`/`beats_in`, which no section names.
                let expected = measure.checked_sub(1).and_then(|m| {
                    let secs = grid.beat_time(m as usize, beat)?;
                    Some((secs, grid.beats_in_measure(m as usize)? as u32))
                });
                if measure == 0 {
                    let count_in = grid
                        .point(0, beat)
                        .map(|p| (grid.time_of(&p).unwrap(), grid.beats_in(0).unwrap() as u32));
                    assert_eq!(
                        got.map(|p| (p.secs, p.beats_in_measure)),
                        count_in,
                        "{name}: count-in position {beat}"
                    );
                    positions.push(((measure, beat), got));
                    continue;
                }
                assert_eq!(
                    got.map(|p| (p.secs, p.beats_in_measure)),
                    expected,
                    "{name}: grid position {measure}/{beat}"
                );
                if let Some(p) = got {
                    assert_eq!((p.measure, p.beat), (measure, beat), "{name}: echo");
                    assert_eq!(
                        grid.beats[p.beat_index as usize],
                        grid.beat_time(measure as usize - 1, beat.floor()).unwrap(),
                        "{name}: beat_index is the whole beat's"
                    );
                    // Snapped within the bar, every step up to the bar's end
                    // resolves and nothing past it moves out of the bar.
                    let within = GridQuery::Position {
                        measure,
                        beat: beat + 0.3,
                        snap: Some(Snap {
                            step: 0.5,
                            within_bar: true,
                        }),
                    }
                    .answer(&grid)
                    .expect("a held position snaps within its bar");
                    assert_eq!(
                        within.measure, measure,
                        "{name}: within bar {measure}/{beat}"
                    );
                    assert_eq!((within.beat - 1.0) % 0.5, 0.0);
                    assert!(within.beat < p.beats_in_measure as f64 + 1.0);
                }
                positions.push(((measure, beat), got));
            }
        }
        let mut times = vec![];
        let mut snapped_times = vec![];
        let end = grid.beats.last().copied().unwrap_or(0.0);
        let mut sweep = secs_sweep(end);
        sweep.extend(grid.beats.iter().flat_map(|b| [*b, b + 1e-4, b - 1e-4]));
        for secs in sweep {
            let got = GridQuery::Time { secs, snap: None }.answer(&grid);
            let expected = grid.position_at(secs).map(|p| {
                (
                    p.measure as u32,
                    p.beat,
                    grid.time_of(&p).expect("the position resolves"),
                    grid.beats_in(p.measure).expect("its measure exists") as u32,
                )
            });
            assert_eq!(
                got.map(|p| (p.measure, p.beat, p.secs, p.beats_in_measure)),
                expected,
                "{name}: grid time {secs}"
            );
            // A fractional position is the time again; a snapped one is the
            // grid's own rounding, and never a beat its measure lacks.
            if let Some(p) = got {
                let on_grid = secs.clamp(grid.beats[0], *grid.beats.last().unwrap());
                assert!((p.secs - on_grid).abs() < 1e-9, "{name}: round trip {secs}");
            }
            for step in [1.0, 0.5] {
                let snapped = GridQuery::Time {
                    secs,
                    snap: Some(Snap {
                        step,
                        within_bar: false,
                    }),
                }
                .answer(&grid);
                let expected = grid
                    .snapped_position_at(secs, step, false)
                    .map(|p| (p.measure as u32, p.beat));
                assert_eq!(
                    snapped.map(|p| (p.measure, p.beat)),
                    expected,
                    "{name}: snapped {secs} by {step}"
                );
                if let Some(p) = snapped {
                    assert!(
                        p.beat < p.beats_in_measure as f64 + 1.0,
                        "{name}: snapped {secs} to a beat its measure lacks: {p:?}"
                    );
                    assert_eq!((p.beat - 1.0) % step, 0.0);
                }
            }
            times.push((secs, got));
            snapped_times.push((
                secs,
                GridQuery::Time {
                    secs,
                    snap: Some(Snap {
                        step: 1.0,
                        within_bar: false,
                    }),
                }
                .answer(&grid),
            ));
        }
        dump.grids.push(DumpGrid {
            name: name.into(),
            grid,
            positions,
            times,
            snapped: snapped_times,
        });
    }

    for (name, config) in tempos() {
        let source = TempoSource::Config {
            config: config.clone(),
        };
        let verdict = config.validate().err();
        assert_eq!(verdict, config.to_tempo_map().err(), "{name}: verdict");
        let (normalized_config, duplicates) = config.normalized();
        let normalized = NormalizedTempo {
            config: normalized_config,
            duplicates: duplicates as u32,
        };
        if verdict.is_none() {
            assert_eq!(
                normalized.config, config,
                "{name}: a valid config is left as it is"
            );
            assert_eq!(normalized.duplicates, 0);
        }
        let map = config.to_tempo_map().ok();

        let mut positions = vec![];
        for measure in 0..=14 {
            for beat in half_beats(9) {
                let got = TempoQuery::Position { measure, beat }.answer(&source);
                let expected = map.as_ref().and_then(|map| {
                    let at = map.measure_to_time_with_offset(measure, beat, 0, 0.0)?;
                    // The bar's signature (read at its downbeat), for a beat
                    // the bar holds in the map's quarter-note units; a beat
                    // past it is in the next bar.
                    let downbeat = map.measure_to_time_with_offset(measure, 1.0, 0, 0.0)?;
                    let bar = map.time_signature_at_time(downbeat, 0.0);
                    let sig = if beat < 1.0 + bar.beats_per_measure() {
                        bar
                    } else {
                        map.time_signature_at_time(at, 0.0)
                    };
                    Some((
                        at.as_secs_f64() * 1000.0,
                        map.bpm_at_time(at, 0.0),
                        (sig.numerator, sig.denominator),
                        format!("@{measure}/{beat:.2}"),
                    ))
                });
                assert_eq!(
                    got.as_ref().map(|p| (p.ms, p.bpm, p.sig, p.label.clone())),
                    expected,
                    "{name}: tempo position {measure}/{beat}"
                );
                if let (Some(p), Some(map)) = (&got, &map) {
                    let at = Duration::from_secs_f64(p.ms / 1000.0);
                    let sig = map.time_signature_at_time(at, 0.0);
                    assert_eq!(p.sig, (sig.numerator, sig.denominator), "{name}: bar sig");
                }
                positions.push(((measure, beat), got));
            }
        }
        let mut times = vec![];
        for secs in secs_sweep(40.0) {
            let ms = secs * 1000.0;
            let got = TempoQuery::Time { ms }.answer(&source);
            assert_eq!(
                got.is_some(),
                map.is_some() && ms >= 0.0,
                "{name}: tempo time {ms}"
            );
            if let (Some(p), Some(map)) = (&got, &map) {
                let at = Duration::from_secs_f64(secs);
                // The position is the inverse of the player's forward map:
                // its time, back through `measure_to_time_with_offset`, is
                // the time asked about (a time before bar 1 is before it).
                // (A time before bar 1 is bar 1 beat 1, as the player has it.)
                let back = map
                    .measure_to_time_with_offset(p.measure, p.beat, 0, 0.0)
                    .expect("a position the map gave resolves");
                let back_ms = back.as_secs_f64() * 1000.0;
                let bar_one_ms = map.start_offset.as_secs_f64() * 1000.0;
                assert!(
                    (back_ms - ms.max(bar_one_ms)).abs() < 1e-3,
                    "{name}: {ms} -> {}/{} -> {back_ms}",
                    p.measure,
                    p.beat
                );
                let sig = map.time_signature_at_time(at, 0.0);
                assert_eq!(
                    p.sig,
                    (sig.numerator, sig.denominator),
                    "{name}: sig at {ms}"
                );
                assert_eq!(p.bpm, map.bpm_at_time(at, 0.0), "{name}: bpm at {ms}");
                assert_eq!(p.label, format!("@{}/{:.2}", p.measure, p.beat));
                // Back through `Duration` (nanoseconds) and out again.
                assert!((p.ms - ms).abs() < 1e-6, "{name}: ms echo {ms} vs {}", p.ms);
            }
            times.push((ms, got));
        }
        dump.tempos.push(DumpTempo {
            name: name.into(),
            config,
            verdict,
            normalized,
            positions,
            times,
        });
    }

    // A mid-bar signature change: the bar keeps its metre (`sig`), the
    // marker sets the new one (`sig_set`), and the next bar has it.
    let mid_bar = TempoSource::Config {
        config: tempos()
            .into_iter()
            .find(|(n, _)| *n == "mid-bar-signature")
            .unwrap()
            .1,
    };
    let at = |measure, beat| {
        TempoQuery::Position { measure, beat }
            .answer(&mid_bar)
            .unwrap()
    };
    assert_eq!((at(3, 3.0).sig, at(3, 3.0).sig_set), ((4, 4), (7, 8)));
    assert_eq!((at(3, 2.5).sig, at(3, 2.5).sig_set), ((4, 4), (4, 4)));
    assert_eq!((at(4, 1.0).sig, at(4, 1.0).sig_set), ((7, 8), (7, 8)));
    let by_time = TempoQuery::Time { ms: at(3, 3.0).ms }
        .answer(&mid_bar)
        .unwrap();
    assert_eq!((by_time.sig, by_time.sig_set), ((4, 4), (7, 8)));

    // A song without a tempo block: its grid's own map.
    let from_grid = TempoSource::Grid {
        grid: steady_grid(0.0, 120.0, &[4, 4, 4, 4]),
    };
    let p = TempoQuery::Time { ms: 2250.0 }.answer(&from_grid).unwrap();
    assert_eq!((p.measure, p.beat, p.sig), (2, 1.5, (4, 4)));
    assert!((p.bpm - 120.0).abs() < 0.01);
    let too_small = TempoSource::Grid {
        grid: BeatGrid {
            beats: vec![0.0, 0.5],
            measure_starts: vec![0],
        },
    };
    assert_eq!(TempoQuery::Time { ms: 0.0 }.answer(&too_small), None);

    for (name, config) in metronomes() {
        let mut patterns = vec![];
        for measure in 1..=8 {
            for beats in 1..=9 {
                patterns.push((
                    (measure, beats),
                    config.accent_pattern_at(measure, beats as usize),
                ));
            }
        }
        dump.metronomes.push(DumpMetronome {
            name: name.into(),
            config,
            patterns,
        });
    }

    for raw in [
        "4/4", "7/8", "6/8", " 3 / 4 ", "5/3", "0/4", "4/0", "4", "", "abc", "12/16", "33/8",
        "4/64",
    ] {
        let got = signature_parse(Some(raw.into())).map(|s| s.0);
        let expected = mtrack_rules_wasm::tempo_config::parse_time_signature(raw)
            .ok()
            .map(|s| (s.numerator, s.denominator));
        assert_eq!(got, expected, "signature {raw:?}");
        dump.signatures.push((raw.into(), got));
    }

    if let Ok(path) = std::env::var("MTRACK_RULES_DUMP") {
        std::fs::write(path, serde_json::to_string(&dump).unwrap()).unwrap();
    }
}

/// A value that is not what its type says -- the shapes a hand-edited
/// song.yaml or a stale page could hand over -- is refused by the DTOs'
/// deserialization (the exports answer `None`, or pass the message on as
/// `tempo_validate`'s verdict) rather than read as something else. The
/// page's values come through serde-wasm-bindgen; serde_json exercises the
/// same `Deserialize` impls natively.
#[test]
fn values_that_are_not_their_type_are_refused_not_coerced() {
    for bad in [
        json!({ "measure": 1.5, "beat": 1 }),
        json!({ "measure": -1, "beat": 1 }),
        json!({ "measure": "1", "beat": 1 }),
        json!({ "secs": "now" }),
        json!({ "secs": 1, "snap": "nearest" }),
        json!({ "secs": 1, "snap": 1 }),
        json!({ "secs": 1, "snap": { "within_bar": true } }),
        json!({}),
    ] {
        assert!(
            serde_json::from_value::<GridQuery>(bad.clone()).is_err(),
            "grid query {bad}"
        );
    }
    for bad in [
        json!({ "ms": "0" }),
        json!({ "measure": 2.5, "beat": 1 }),
        json!({ "measure": 1 }),
    ] {
        assert!(
            serde_json::from_value::<TempoQuery>(bad.clone()).is_err(),
            "tempo query {bad}"
        );
    }
    for bad in [
        json!({ "bpm": "fast" }),
        json!({ "bpm": 120, "changes": [{ "measure": 1.5, "bpm": 100 }] }),
        json!({ "bpm": 120, "changes": [{ "measure": -3, "bpm": 100 }] }),
        json!({ "bpm": 120, "changes": [{ "bpm": 100 }] }),
    ] {
        assert!(
            serde_json::from_value::<TempoConfig>(bad.clone()).is_err(),
            "tempo config {bad}"
        );
        assert!(serde_json::from_value::<TempoSource>(json!({ "config": bad })).is_err());
    }
    assert!(serde_json::from_value::<MetronomeConfig>(json!({ "accents": [1, -1] })).is_err());
    assert!(
        serde_json::from_value::<MetronomeConfig>(json!({ "changes": [{ "measure": 0.5 }] }))
            .is_err()
    );
    // What the types do take, through the same door.
    let query: GridQuery =
        serde_json::from_value(json!({ "secs": 1.25, "snap": { "step": 0.5 } })).unwrap();
    assert_eq!(
        query,
        GridQuery::Time {
            secs: 1.25,
            snap: Some(Snap {
                step: 0.5,
                within_bar: false
            })
        }
    );
    let query: GridQuery = serde_json::from_value(json!({ "secs": 1.25 })).unwrap();
    assert_eq!(
        query,
        GridQuery::Time {
            secs: 1.25,
            snap: None
        }
    );
    let config: TempoConfig = serde_json::from_value(json!({ "bpm": 120 })).unwrap();
    assert_eq!(config.validate(), Ok(()));
    // Values that are their type but make no map: the verdict, not a trap.
    let config: TempoConfig = serde_json::from_value(json!({ "bpm": 0 })).unwrap();
    assert!(config.validate().is_err());
    let query: GridQuery =
        serde_json::from_value(json!({ "secs": 1.0, "snap": { "step": -1 } })).unwrap();
    assert_eq!(query.answer(&steady_grid(0.0, 120.0, &[4])), None);
}

/// Magnitudes no song has are refused before the player's functions walk
/// them bar by bar or turn them into a `Duration` that cannot hold them.
#[test]
fn absurd_magnitudes_are_refused_not_walked() {
    let grid = steady_grid(0.0, 120.0, &[4, 4]);
    for query in [
        GridQuery::Time {
            secs: 1e300,
            snap: None,
        },
        GridQuery::Time {
            secs: f64::INFINITY,
            snap: None,
        },
        GridQuery::Time {
            secs: 1.0,
            snap: Some(Snap {
                step: 1e300,
                within_bar: true,
            }),
        },
        GridQuery::Position {
            measure: u32::MAX,
            beat: 1.0,
            snap: None,
        },
        GridQuery::Position {
            measure: 1,
            beat: 1e300,
            snap: None,
        },
        GridQuery::Position {
            measure: 1,
            beat: f64::NAN,
            snap: None,
        },
    ] {
        assert_eq!(query.answer(&grid), None, "{query:?}");
    }
    let far = BeatGrid {
        beats: vec![0.0, 1e300],
        measure_starts: vec![0],
    };
    assert_eq!(
        GridQuery::Time {
            secs: 0.0,
            snap: None
        }
        .answer(&far),
        None
    );
    assert!(TempoSource::Grid { grid: far }.to_tempo_map().is_err());

    let config = TempoConfig {
        bpm: 120.0,
        time_signature: "4/4".into(),
        start: None,
        changes: vec![],
    };
    let source = TempoSource::Config {
        config: config.clone(),
    };
    for query in [
        TempoQuery::Time { ms: 1e300 },
        TempoQuery::Time { ms: 1e18 },
        TempoQuery::Time { ms: f64::NAN },
        TempoQuery::Position {
            measure: u32::MAX,
            beat: 1.0,
        },
        TempoQuery::Position {
            measure: 1,
            beat: 1e300,
        },
    ] {
        assert_eq!(query.answer(&source), None, "{query:?}");
    }
    assert!(TempoQuery::Time { ms: 1e12 }.answer(&source).is_some());
    let beyond = [
        TempoConfig {
            start: Some(1e300),
            ..config.clone()
        },
        TempoConfig {
            changes: vec![TempoChangeConfig {
                bpm: Some(100.0),
                ..change(u32::MAX, None)
            }],
            ..config.clone()
        },
        TempoConfig {
            changes: vec![TempoChangeConfig {
                bpm: Some(100.0),
                ..change(2, Some(1e300))
            }],
            ..config.clone()
        },
        TempoConfig {
            changes: vec![TempoChangeConfig {
                bpm: Some(100.0),
                transition: Some(TransitionConfig::Measures(1e300)),
                ..change(2, None)
            }],
            ..config.clone()
        },
    ];
    for config in beyond {
        let why = bounds::tempo_config(&config);
        assert!(why.is_some(), "{config:?}");
        assert_eq!(TempoSource::Config { config }.to_tempo_map().err(), why);
    }
    assert_eq!(bounds::tempo_config(&config), None);
    assert_eq!(signature_parse(None), None);
    assert_eq!(
        signature_parse(Some("7/8".into())).map(|s| s.0),
        Some((7, 8))
    );
}
