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

//! What the strobe curve's automatic rule chooses for every strobe function
//! in the bring-your-own corpora (`tests/gdtf-corpus/*.gdtf` and the GDTFs
//! embedded in `tests/mvr-corpus/*.mvr`). A GDTF fixture with no
//! `strobe_curve` in its record follows its file (`declared`): the
//! function's own table when it has one, linear in hertz between the
//! endpoints when it has none. Printed for eyeballing:
//!
//! ```sh
//! cargo test --test strobe_curve_corpus -- --ignored --nocapture
//! ```
//!
//! And the round trip the state stream relies on: the byte the engine
//! writes for a rate reads back (`FixtureInfo::strobe_hz`) as that rate,
//! for every byte of every strobe rule — the example fixture types always,
//! the corpus with `--ignored`.

use std::collections::{BTreeMap, HashMap};

use mtrack::lighting::effects::{coarse_byte, FixtureInfo, StrobeMap};
use mtrack::lighting::system::fixture_info_for;
use mtrack::lighting::types::{
    ChannelDef, ChannelFunction, FixtureType, FunctionStep, PhysicalRange, PhysicalUnit,
    StrobeCurve,
};
use mtrack::lighting::{gdtf, mvr};

/// Every GDTF of both corpora, by (manufacturer, fixture name), first seen.
fn corpus_gdtfs() -> BTreeMap<(String, String), gdtf::Description> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let files = |dir: &str, ext: &str| -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == ext))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    };
    let mut out = BTreeMap::new();
    let mut add = |bytes: &[u8]| {
        if let Ok(description) = gdtf::parse_archive(bytes) {
            out.entry((description.manufacturer.clone(), description.name.clone()))
                .or_insert(description);
        }
    };
    for path in files("gdtf-corpus", "gdtf") {
        add(&std::fs::read(&path).expect("readable gdtf"));
    }
    for path in files("mvr-corpus", "mvr") {
        let bytes = std::fs::read(&path).expect("readable mvr");
        for entry in mvr::list_gdtf_entries(&bytes).expect("entry listing") {
            if let Ok(gdtf_bytes) = mvr::read_gdtf_entry(&bytes, &entry) {
                add(&gdtf_bytes);
            }
        }
    }
    out
}

#[test]
#[ignore = "bring-your-own corpus: tests/gdtf-corpus/ and tests/mvr-corpus/, run with --ignored"]
fn what_the_automatic_strobe_curve_chooses_across_the_corpus() {
    let gdtfs = corpus_gdtfs();
    assert!(
        !gdtfs.is_empty(),
        "no corpus: put .gdtf files in tests/gdtf-corpus/ or .mvr files in tests/mvr-corpus/"
    );
    println!("| fixture | strobe functions | Hz | steps | automatic curve |");
    println!("|---|---|---|---|---|");
    let (mut tabled, mut endpoints, mut none) = (0, 0, 0);
    for ((manufacturer, name), description) in &gdtfs {
        let tables = gdtf::strobe_tables(description);
        if tables.is_empty() {
            none += 1;
            println!("| {manufacturer} {name} | 0 | | | no strobe rate |");
            continue;
        }
        let mut steps: Vec<usize> = tables.iter().map(|t| t.steps).collect();
        steps.sort();
        steps.dedup();
        let mut hz: Vec<String> = tables
            .iter()
            .map(|t| format!("{}–{}", t.from_hz, t.to_hz))
            .collect();
        hz.sort();
        hz.dedup();
        let most = *steps.last().expect("non-empty");
        let choice = if most > 0 {
            tabled += 1;
            format!("declared: its {most}-step table")
        } else {
            endpoints += 1;
            "declared: linear between the endpoints".to_string()
        };
        println!(
            "| {manufacturer} {name} | {} | {} | {} | {choice} |",
            tables.len(),
            hz.join(", "),
            steps
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    println!(
        "\n{} fixtures: {tabled} with a table, {endpoints} with endpoints only, {none} with no \
         strobe rate",
        gdtfs.len()
    );
}

/// Every byte of `fixture`'s strobe channel reads back as a rate the
/// engine would write that byte for: byte → `strobe_hz` → the engine's
/// forward mapping → the byte. A tabled rule can give two bytes the same
/// rate (a held step, overlapping steps), and then the forward mapping
/// picks the first step and rounds to its byte; there the rate read back
/// from that byte is the same rate to within the half byte the rounding
/// moved it, which is all the stage views need.
fn assert_round_trips(label: &str, fixture: &FixtureInfo) -> usize {
    let map = fixture.strobe_map();
    let steps: &[FunctionStep] = match &map {
        StrobeMap::Declared(steps) => steps,
        StrobeMap::Endpoints(step) => std::slice::from_ref(step),
        _ => &[],
    };
    // Half a byte of the step `byte` sits in, in hertz.
    let half_byte_hz = |byte: u8| {
        steps
            .iter()
            .find(|s| s.dmx_from <= byte && byte <= s.dmx_to)
            .map_or(0.0, |s| {
                let bytes = f64::from(s.dmx_to) - f64::from(s.dmx_from);
                if bytes == 0.0 {
                    0.0
                } else {
                    0.5 * (s.to - s.from).abs() / bytes
                }
            })
    };
    assert_eq!(fixture.strobe_hz(0), None, "{label}: byte 0 is off");
    let mut reported = 0;
    for byte in 0..=255u8 {
        let Some(hz) = fixture.strobe_hz(byte) else {
            continue;
        };
        reported += 1;
        assert!(hz.is_finite() && hz > 0.0, "{label}: byte {byte} → {hz} Hz");
        let back = coarse_byte(map.normalized(hz));
        if back == byte {
            continue;
        }
        assert!(
            !steps.is_empty(),
            "{label}: byte {byte} → {hz} Hz → byte {back} through {map:?}"
        );
        let again = fixture.strobe_hz(back);
        let tolerance = half_byte_hz(back) + 1e-9 * hz.max(1.0);
        assert!(
            again.is_some_and(|again| (again - hz).abs() <= tolerance),
            "{label}: byte {byte} → {hz} Hz → byte {back} → {again:?} Hz through {map:?}"
        );
    }
    reported
}

/// A GDTF-shaped fixture type with a strobe table: a "pulse close" step
/// descending in rate, the main ramp, and a held top step — the shapes
/// the corpus has (Robe's, say), without a corpus.
fn tabled_fixture_type() -> FixtureType {
    let step = |dmx_from, dmx_to, from, to| FunctionStep {
        dmx_from,
        dmx_to,
        from,
        to,
    };
    let mut strobe = ChannelDef::at(4);
    strobe.functions = vec![
        ChannelFunction {
            name: "open".to_string(),
            dmx_from: 0,
            dmx_to: 31,
            physical: None,
            steps: Vec::new(),
        },
        ChannelFunction {
            name: "strobe".to_string(),
            dmx_from: 32,
            dmx_to: 255,
            physical: Some(PhysicalRange {
                from: 0.3,
                to: 25.0,
                unit: PhysicalUnit::Hertz,
            }),
            steps: vec![
                step(32, 63, 5.0, 1.0),
                step(64, 223, 0.3, 25.0),
                step(224, 239, 25.0, 25.0),
                step(240, 255, 10.0, 30.0),
            ],
        },
    ];
    let channel_defs: HashMap<String, ChannelDef> = [
        ("red".to_string(), ChannelDef::at(1)),
        ("green".to_string(), ChannelDef::at(2)),
        ("blue".to_string(), ChannelDef::at(3)),
        ("strobe".to_string(), strobe),
    ]
    .into_iter()
    .collect();
    FixtureType::from_channel_defs("Tabled".to_string(), channel_defs)
}

/// The example fixture types, and the tabled one, each under the curve
/// its file states (or the automatic one) and under every curve.
fn example_fixtures() -> Vec<(String, FixtureInfo)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut types: Vec<FixtureType> = Vec::new();
    for name in [
        "astera_pixelbrick.light",
        "cheap_mover.fixture",
        "moving_head.light",
        "rgbw_par.light",
        "strobe.light",
    ] {
        let content =
            std::fs::read_to_string(root.join("examples/lighting/fixture_types").join(name))
                .expect("example fixture type");
        let parsed = mtrack::lighting::parser::parse_fixture_types(&content).expect(name);
        types.extend(parsed.into_values());
    }
    types.push(tabled_fixture_type());
    types.sort_by(|a, b| a.name().cmp(b.name()));
    let mut out = Vec::new();
    for fixture_type in types {
        let curves = [
            None,
            Some(StrobeCurve::Period),
            Some(StrobeCurve::Linear),
            Some(StrobeCurve::Declared),
        ];
        for curve in curves {
            let mut fixture_type = fixture_type.clone();
            fixture_type.set_strobe_curve(curve);
            let info = fixture_info_for("f", 1, 1, fixture_type.name(), &fixture_type);
            let label = format!(
                "{} ({})",
                fixture_type.name(),
                curve.map_or("automatic", StrobeCurve::keyword)
            );
            out.push((label, info));
        }
    }
    out
}

#[test]
fn every_strobe_byte_reads_back_as_a_rate_the_engine_writes_it_for() {
    let mut branches = std::collections::BTreeSet::new();
    for (label, fixture) in example_fixtures() {
        let map = fixture.strobe_map();
        branches.insert(match map {
            StrobeMap::Declared(_) => "declared",
            StrobeMap::Endpoints(_) => "endpoints",
            StrobeMap::LinearHz { .. } => "linear",
            StrobeMap::Period { .. } => "period",
            StrobeMap::Ratio { .. } => "ratio",
        });
        let reported = assert_round_trips(&label, &fixture);
        let strobes = fixture.channels.contains_key("strobe");
        assert_eq!(
            reported > 0,
            strobes,
            "{label}: {reported} bytes report a rate; strobe channel: {strobes}"
        );
    }
    assert_eq!(
        branches.into_iter().collect::<Vec<_>>(),
        ["declared", "endpoints", "linear", "period", "ratio"],
        "every strobe rule is exercised"
    );
}

/// The rate the stage views flashed at before the player said it: a
/// formula over the byte alone, the same for every fixture. Beside the
/// player's rate at a few bytes, per example type — the drift the state
/// stream's `strobe_hz` ends.
#[test]
#[ignore = "prints the old visualizer formula against the player's rate; run with --ignored --nocapture"]
fn the_old_visualizer_formula_against_the_player() {
    println!("| fixture (curve) | byte | old TS Hz | player Hz |");
    println!("|---|---|---|---|");
    for (label, fixture) in example_fixtures() {
        if !fixture.channels.contains_key("strobe") {
            continue;
        }
        for byte in [1u8, 64, 128, 192, 255] {
            let old = if byte > 10 {
                format!("{:.2}", 2.0 + f64::from(byte) / 255.0 * 18.0)
            } else {
                "steady".to_string()
            };
            let new = fixture
                .strobe_hz(byte)
                .map_or("not strobing".to_string(), |hz| format!("{hz:.2}"));
            println!("| {label} | {byte} | {old} | {new} |");
        }
    }
}

#[test]
#[ignore = "bring-your-own corpus: tests/gdtf-corpus/ and tests/mvr-corpus/, run with --ignored"]
fn every_corpus_strobe_byte_reads_back_as_a_rate_the_engine_writes_it_for() {
    let gdtfs = corpus_gdtfs();
    assert!(
        !gdtfs.is_empty(),
        "no corpus: put .gdtf files in tests/gdtf-corpus/ or .mvr files in tests/mvr-corpus/"
    );
    let mut checked = 0;
    for ((manufacturer, name), description) in &gdtfs {
        for mode in &description.modes {
            let Ok(distilled) = gdtf::distill(description, &mode.name, name) else {
                continue;
            };
            for curve in [
                None,
                Some(StrobeCurve::Period),
                Some(StrobeCurve::Linear),
                Some(StrobeCurve::Declared),
            ] {
                let mut fixture_type = distilled.fixture_type.clone();
                fixture_type.set_source(mtrack::lighting::types::GdtfSource {
                    path: format!("lighting/library/{name}.gdtf"),
                });
                fixture_type.set_strobe_curve(curve);
                let info = fixture_info_for("f", 1, 1, name, &fixture_type);
                let label = format!(
                    "{manufacturer} {name} / {} ({})",
                    mode.name,
                    curve.map_or("automatic", StrobeCurve::keyword)
                );
                assert_round_trips(&label, &info);
                checked += 1;
            }
        }
    }
    println!("{checked} fixture modes × curves round-trip");
}
