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
//! The light show exports answer what the player's tempo map and parser
//! answer: the positions, ramps, metres and offsets the web UI's timeline
//! used to work out for itself (its `timeline-tempo-math` cases, held to
//! the server's values), now read off the player through `tempo_query`,
//! `tempo_distance`, `layout` and `document`.

use std::time::Duration;

use mtrack_rules_wasm::lighting::parser::document::document;
use mtrack_rules_wasm::{
    IncludeSource, Layout, LayoutRequest, TempoDistance, TempoQuery, TempoSource,
};

/// A light show `tempo { }` block, as the editor's serializer writes one.
fn dsl(bpm: f64, sig: (u32, u32), changes: &[&str], start_ms: f64) -> TempoSource {
    let mut block = format!(
        "tempo {{\n    start: {}ms\n    bpm: {bpm}\n    time_signature: {}/{}\n",
        start_ms, sig.0, sig.1
    );
    if !changes.is_empty() {
        block.push_str("    changes: [\n");
        block.push_str(&changes.join(",\n"));
        block.push_str("\n    ]\n");
    }
    block.push_str("}\n");
    TempoSource::Dsl { dsl: block }
}

fn at(source: &TempoSource, measure: u32, beat: f64) -> f64 {
    TempoQuery::Position { measure, beat }
        .answer(Some(source))
        .unwrap_or_else(|| panic!("@{measure}/{beat}"))
        .ms
}

fn bpm_at(source: &TempoSource, ms: f64) -> f64 {
    TempoQuery::Time { ms }.answer(Some(source)).unwrap().bpm
}

fn span(source: Option<&TempoSource>, from_ms: f64, duration: &str) -> f64 {
    TempoQuery::Span {
        from_ms,
        duration: duration.to_string(),
    }
    .answer(source)
    .unwrap_or_else(|| panic!("{duration} from {from_ms}"))
    .ms - from_ms
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() < 1e-6, "{what}: got {got}, want {want}");
}

fn lay(source: &str, song_tempo: Option<TempoSource>, until_ms: f64) -> Layout {
    LayoutRequest {
        source: source.to_string(),
        path: None,
        song_tempo,
        includes: Vec::new(),
        until_ms,
    }
    .answer()
}

#[test]
fn a_ramp_over_n_beats_spans_exactly_n_beats() {
    for (from, to) in [
        (120.0, 180.0),
        (180.0, 120.0),
        (90.0, 140.0),
        (100.0, 100.0),
    ] {
        for beats in [1.0, 4.0, 6.5, 8.0] {
            let t = dsl(
                from,
                (4, 4),
                &[&format!(
                    "        @3/1 {{ bpm: {to}, transition: {beats} }}"
                )],
                500.0,
            );
            let start = 500.0 + 8.0 * 60000.0 / from;
            let length = beats * 60000.0 / ((from + to) / 2.0);
            close(at(&t, 3, 1.0), start, "ramp start");
            let whole = beats.floor();
            let end_bar = 3 + (whole / 4.0).floor() as u32;
            let end_beat = 1.0 + (beats - (whole / 4.0).floor() * 4.0);
            close(
                at(&t, end_bar, end_beat),
                start + length,
                &format!("{from}->{to}/{beats}"),
            );
            close(
                span(Some(&t), start, &format!("{beats}beats")),
                length,
                "beats from the start",
            );
            close(bpm_at(&t, start + length + 1e-3), to, "the target after");
            close(
                span(Some(&t), start + length, "4beats"),
                4.0 * 60000.0 / to,
                "beats after the ramp",
            );
            if from != to {
                assert!((bpm_at(&t, start + length - 1.0) - to).abs() > 1e-3);
            }
        }
    }
}

#[test]
fn the_tempo_moves_linearly_in_time_through_a_ramp() {
    let t = dsl(
        120.0,
        (4, 4),
        &["        @3/1 { bpm: 180, transition: 8 }"],
        0.0,
    );
    // Bar 3 at 4s; 8 beats averaging 150 take 3.2s.
    close(bpm_at(&t, 4000.0), 120.0, "at the change");
    close(bpm_at(&t, 5600.0), 150.0, "halfway");
    close(bpm_at(&t, 7200.0), 180.0, "at the end");
}

#[test]
fn a_ramp_over_n_measures_ends_on_the_barline_n_bars_on() {
    for sig in [(4, 4), (3, 4), (6, 8), (7, 8)] {
        let quarters = f64::from(sig.0) * 4.0 / f64::from(sig.1);
        let t = dsl(
            120.0,
            sig,
            &["        @3/1 { bpm: 180, transition: 2m }"],
            0.0,
        );
        let bar3 = 2.0 * quarters * 60000.0 / 120.0;
        let bar5 = bar3 + 2.0 * quarters * 60000.0 / 150.0;
        close(at(&t, 3, 1.0), bar3, &format!("{sig:?} bar 3"));
        close(at(&t, 5, 1.0), bar5, &format!("{sig:?} bar 5"));
        close(
            at(&t, 6, 1.0),
            bar5 + quarters * 60000.0 / 180.0,
            &format!("{sig:?} bar 6"),
        );
        close(
            span(Some(&t), bar3, "2measures"),
            bar5 - bar3,
            &format!("{sig:?} 2 measures"),
        );
        let distance = TempoDistance::between(&t, bar3, bar5).unwrap();
        close(distance.measures, 2.0, "two whole bars");
        close(distance.beats, 2.0 * quarters, "their beats");
    }
}

#[test]
fn a_change_written_after_a_ramp_lands_on_its_own_bar() {
    let t = dsl(
        120.0,
        (4, 4),
        &[
            "        @3/1 { bpm: 180, transition: 8 }",
            "        @7/1 { bpm: 90 }",
        ],
        0.0,
    );
    // Bar 5 at 7.2s, two bars at 180 put bar 7 at 9.8667s, then 90.
    let bar7 = 7200.0 + 8.0 * 60000.0 / 180.0;
    close(at(&t, 5, 1.0), 7200.0, "bar 5");
    close(at(&t, 7, 1.0), bar7, "bar 7");
    close(at(&t, 8, 1.0), bar7 + 4.0 * 60000.0 / 90.0, "bar 8");
    let p = TempoQuery::Time { ms: bar7 }.answer(Some(&t)).unwrap();
    assert_eq!((p.measure, p.beat.round()), (7, 1.0));
    // The grid draws its barlines there too.
    let TempoSource::Dsl { dsl } = &t else {
        unreachable!()
    };
    let laid = lay(
        &format!("{dsl}show \"S\" {{\n    @1/1\n    a: static, color: \"red\", duration: 1s\n}}\n"),
        None,
        12000.0,
    );
    let bars: Vec<f64> = laid
        .tempo
        .unwrap()
        .grid
        .iter()
        .filter(|l| l.beat == 1)
        .map(|l| l.ms)
        .collect();
    let bar4 = 4000.0 + span(Some(&t), 4000.0, "4beats");
    let want = [
        0.0,
        2000.0,
        4000.0,
        bar4,
        7200.0,
        7200.0 + 4000.0 / 3.0,
        bar7,
    ];
    for (i, ms) in want.iter().enumerate() {
        close(bars[i], *ms, &format!("barline {}", i + 1));
    }
}

#[test]
fn a_time_positioned_change_after_a_ramp_counts_through_the_ramp() {
    let t = dsl(
        120.0,
        (4, 4),
        &[
            "        @1/1 { bpm: 60, transition: 4 }",
            "        @00:10.000 { bpm: 100 }",
            "        @20/1 { bpm: 80 }",
        ],
        0.0,
    );
    // 4 beats in 8/3s, 22/3 beats at 60 to 10s, then 194/3 beats at 100 to bar 20.
    close(at(&t, 20, 1.0), 48800.0, "bar 20");
}

#[test]
fn a_ramp_in_measures_counts_a_signature_set_beside_it_in_either_order() {
    let ramp = "        @5/1 { bpm: 100, transition: 2m }";
    let metre = "        @5/1 { time_signature: 3/4 }";
    for changes in [[ramp, metre], [metre, ramp]] {
        let t = dsl(120.0, (4, 4), &changes, 0.0);
        // Bar 5 at 8s; 2 bars of 3/4 averaging 110 take 36/11s.
        close(at(&t, 7, 1.0), 8000.0 + 36000.0 / 11.0, "bar 7");
    }
    let timed_metre = "        @00:08.000 { time_signature: 3/4 }";
    let timed_ramp = "        @00:08.000 { bpm: 100, transition: 2m }";
    for changes in [
        [ramp, timed_metre],
        [timed_metre, ramp],
        [timed_ramp, metre],
        [metre, timed_ramp],
    ] {
        let t = dsl(120.0, (4, 4), &changes, 0.0);
        close(
            at(&t, 7, 1.0),
            8000.0 + 36000.0 / 11.0,
            &format!("{changes:?}"),
        );
    }
}

#[test]
fn a_mid_bar_signature_change_governs_from_the_next_bar() {
    let t = dsl(
        120.0,
        (4, 4),
        &["        @2/3 { time_signature: 3/4 }"],
        0.0,
    );
    close(at(&t, 3, 1.0), 4000.0, "bar 3");
    close(at(&t, 4, 1.0), 5500.0, "bar 4");
    // Written past its bar's end (@4/6 in 4/4): from bar 5, the bar the
    // position falls in, not the one after it.
    let t = dsl(
        120.0,
        (4, 4),
        &["        @4/6 { time_signature: 3/4 }"],
        0.0,
    );
    close(at(&t, 5, 1.0), 8000.0, "bar 5");
    close(at(&t, 6, 1.0), 9500.0, "bar 6");
}

#[test]
fn snaps_and_steady_tempos_time_as_before() {
    let t = dsl(
        120.0,
        (4, 4),
        &[
            "        @3/1 { bpm: 150 }",
            "        @5/1 { bpm: 100, time_signature: 3/4 }",
            "        @00:20.000 { bpm: 60 }",
        ],
        1000.0,
    );
    close(at(&t, 1, 1.0), 1000.0, "bar 1");
    close(at(&t, 3, 1.0), 5000.0, "bar 3");
    close(at(&t, 5, 1.0), 8200.0, "bar 5");
    close(at(&t, 7, 1.0), 11800.0, "bar 7");
    // 20s is bar 11 beat 2 2/3; bar 12 is 1 1/3 beats on at 60.
    close(at(&t, 12, 1.0), 20000.0 + 4000.0 / 3.0, "bar 12");
    let steady = dsl(
        120.0,
        (4, 4),
        &["        @2/1 { bpm: 120, transition: 2m }"],
        0.0,
    );
    for m in 1..10 {
        close(at(&steady, m, 1.0), f64::from(m - 1) * 2000.0, "steady");
    }
}

#[test]
fn a_span_in_seconds_needs_no_tempo() {
    close(span(None, 1500.0, "5s"), 5000.0, "seconds");
    close(span(None, 0.0, "500ms"), 500.0, "milliseconds");
    close(span(None, 0.0, "2.5"), 2500.0, "a bare number");
    assert_eq!(
        TempoQuery::Span {
            from_ms: 0.0,
            duration: "2beats".into()
        }
        .answer(None),
        None
    );
}

const TEMPO: &str = "tempo {\n    bpm: 120\n    time_signature: 4/4\n}\n";

fn cue(measure: u32, beat: f64, extra: &str) -> String {
    format!("    @{measure}/{beat}\n    a: static, color: \"red\", duration: 1beat\n{extra}")
}

#[test]
fn an_offset_lasts_its_bars_at_the_one_tempo_where_it_is_issued() {
    // As the player sizes it: the change at bar 3 inside the offset's two
    // bars does not stretch them.
    let source = format!(
        "tempo {{\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @3/1 {{ bpm: 60 }}\n    ]\n}}\nshow \"S\" {{\n{}{}}}\n",
        cue(2, 1.0, "    offset 2 measures\n"),
        cue(4, 1.0, "")
    );
    let laid = lay(&source, None, 0.0);
    let cues = &laid.shows[0].cues;
    close(cues[0].next_offset_ms, 4000.0, "two bars at 120");
    close(cues[1].score_ms, 8000.0, "bar 4 in the score");
    close(cues[1].ms, 12000.0, "bar 4 under the offset");
}

#[test]
fn an_offset_is_sized_where_its_cue_sits_before_any_offset() {
    // The second offset is issued at bar 4 beat 3 (7s), at 120: shifted by
    // the first offset it would be at 11s, past the change to 60 at bar 5.
    let source = format!(
        "tempo {{\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @5/1 {{ bpm: 60 }}\n    ]\n}}\nshow \"S\" {{\n{}{}{}}}\n",
        cue(4, 1.0, "    offset 2 measures\n"),
        cue(4, 3.0, "    offset 1 measures\n"),
        cue(6, 1.0, "")
    );
    let laid = lay(&source, None, 0.0);
    let cues = &laid.shows[0].cues;
    close(cues[0].next_offset_ms, 4000.0, "first offset");
    close(
        cues[1].next_offset_ms - cues[1].offset_ms,
        2000.0,
        "second offset",
    );
    close(cues[2].ms, 12000.0 + 6000.0, "bar 6");
}

#[test]
fn a_reset_drops_the_offset_for_the_cues_after_it() {
    let source = format!(
        "{TEMPO}show \"S\" {{\n{}{}{}}}\n",
        cue(1, 2.0, "    offset 1 measures\n"),
        cue(2, 1.0, "    reset_measures\n"),
        cue(3, 1.0, "")
    );
    let laid = lay(&source, None, 0.0);
    let cues = &laid.shows[0].cues;
    close(cues[1].ms, 2000.0 + 2000.0, "bar 2 under the offset");
    close(cues[2].ms, 4000.0, "bar 3 after the reset");
}

#[test]
fn under_an_offset_the_whole_tempo_map_moves_with_it() {
    // The values the server gives (show_edge_cases_tests.rs,
    // test_offset_shifts_time_positioned_tempo_changes): a change positioned
    // in time moves with the offset like one positioned by bar, so the cue
    // is its score time plus the offset.
    for (transition, want) in [("", 12000.0), (", transition: 4", 10666.666667)] {
        let source = format!(
            "tempo {{\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @00:06.000 {{ bpm: 60{transition} }}\n    ]\n}}\nshow \"S\" {{\n{}{}}}\n",
            cue(1, 1.0, "    offset 1 measures\n"),
            cue(5, 1.0, "")
        );
        let laid = lay(&source, None, 0.0);
        let cues = &laid.shows[0].cues;
        close(cues[1].ms, want, transition);
        let t = TempoSource::Dsl {
            dsl: source.clone(),
        };
        close(cues[1].score_ms, at(&t, 5, 1.0), "its own place in the map");
    }
    let source = format!(
        "tempo {{\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @4/1 {{ bpm: 60 }}\n    ]\n}}\nshow \"S\" {{\n{}{}}}\n",
        cue(1, 1.0, "    offset 1 measures\n"),
        cue(5, 1.0, "")
    );
    close(
        lay(&source, None, 0.0).shows[0].cues[1].ms,
        10000.0 + 2000.0,
        "a change positioned by bar stays on its bar",
    );
}

#[test]
fn placing_a_cue_under_an_offset_round_trips_with_its_layout() {
    // Offsets, a time-positioned ramp and a bar-positioned ramp: a cue
    // placed at the time it is drawn at comes back as the same bar and
    // beat, and is drawn there again.
    let tempo = "tempo {\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @00:06.000 { bpm: 60, transition: 4 },\n        @9/1 { bpm: 150, transition: 1m }\n    ]\n}\n";
    let t = TempoSource::Dsl {
        dsl: tempo.to_string(),
    };
    for m in 3..=12 {
        for b in 1..=4 {
            let source = format!(
                "{tempo}show \"S\" {{\n{}{}{}}}\n",
                cue(1, 1.0, "    offset 1 measures\n"),
                cue(3, 3.0, "    offset 2 measures\n"),
                cue(m, f64::from(b), "")
            );
            let placed = &lay(&source, None, 0.0).shows[0].cues[2];
            let score = placed.ms - placed.offset_ms;
            let what = format!("@{m}/{b}");
            close(placed.score_ms, score, &what);
            let p = TempoQuery::Time { ms: score }.answer(Some(&t)).unwrap();
            assert_eq!((p.measure, p.beat.round() as u32), (m, b), "{what}");
            close(at(&t, m, f64::from(b)), score, &what);
        }
    }
}

#[test]
fn a_long_show_with_many_tempo_changes_lays_out_quickly() {
    let mut changes = Vec::new();
    for m in 2..=301 {
        let bpm = 100 + (m % 7) * 10;
        let transition = match m % 3 {
            0 => ", transition: 1m",
            1 => ", transition: 2",
            _ => "",
        };
        let sig = if m % 50 == 0 {
            ", time_signature: 3/4"
        } else {
            ""
        };
        changes.push(format!("        @{m}/1 {{ bpm: {bpm}{transition}{sig} }}"));
    }
    let mut source = format!(
        "tempo {{\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n{}\n    ]\n}}\nshow \"S\" {{\n",
        changes.join(",\n")
    );
    for m in 1..=500 {
        source.push_str(&cue(
            m,
            1.0,
            if m % 25 == 0 {
                "    offset 1 measures\n"
            } else {
                ""
            },
        ));
    }
    source.push_str("}\n");
    let started = std::time::Instant::now();
    let laid = lay(&source, None, 600000.0);
    assert_eq!(laid.error, None);
    assert_eq!(laid.shows[0].cues.len(), 500);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_file_is_read_as_written_and_laid_out_with_its_includes() {
    let common = "sequence \"strike\"(hit, bed: \"#0000FF\"), length: 1measure {\n    @1/1\n    front: static, color: $hit, duration: 1beat\n}\n";
    let show = format!(
        "include \"../common/looks.light\"\n{TEMPO}show \"Song\" {{\n    @2/1\n    sequence \"strike\"(hit: \"#FF0000\"), loop: 2\n}}\n"
    );
    let doc = document(&show, Some(std::path::Path::new("band/song/show.light")))
        .document
        .unwrap();
    let include = &doc.includes.unwrap()[0];
    assert_eq!(include.path, "band/common/looks.light");
    let request = |source: Option<&str>, error: Option<&str>| LayoutRequest {
        source: show.clone(),
        path: Some("band/song/show.light".into()),
        song_tempo: None,
        includes: vec![IncludeSource {
            path: include.path.clone(),
            source: source.map(str::to_string),
            error: error.map(str::to_string),
        }],
        until_ms: 0.0,
    };
    let laid = request(Some(common), None).answer();
    assert_eq!(laid.error, None);
    let passes = &laid.shows[0].cues[0].passes[0];
    close(passes.start_ms, 2000.0, "bar 2");
    close(passes.end_ms, 6000.0, "two passes of a bar");
    let laid = request(
        None,
        Some("Lighting file not found: band/common/looks.light"),
    )
    .answer();
    assert_eq!(
        laid.error.as_deref(),
        Some(
            "line 1: include \"../common/looks.light\": cannot read band/song/../common/looks.light: Lighting file not found: band/common/looks.light"
        )
    );
    // A song's tempo stands in for a file with none.
    let laid = lay(
        "show \"S\" {\n    @2/1\n    a: static, color: \"red\", duration: 1beat\n}\n",
        Some(dsl(60.0, (4, 4), &[], 0.0)),
        0.0,
    );
    close(laid.shows[0].cues[0].ms, 4000.0, "bar 2 at 60");
    let laid = lay(
        "show \"S\" {\n    @2/1\n    a: static, color: \"red\", duration: 1beat\n}\n",
        None,
        0.0,
    );
    assert!(laid
        .error
        .unwrap()
        .contains("Measure-based timing requires a tempo section"));
}

#[test]
fn a_parameter_reference_is_the_whole_value_and_nothing_after_it() {
    assert!(mtrack_rules_wasm::param_ref("$x"));
    assert!(mtrack_rules_wasm::param_ref(" $level "));
    for text in ["$x # c", "$x // c", "$x\n# c", "$x y", "x", "$", "\"$x\""] {
        assert!(!mtrack_rules_wasm::param_ref(text), "{text:?}");
    }
}
