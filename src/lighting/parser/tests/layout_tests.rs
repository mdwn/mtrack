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
//! The web UI's layout holds to the player: every cue the timeline fires
//! is laid out at that time, and every effect's written end is the
//! engine's reckoning of it (`timeline::effect_ends` takes the written end
//! as the cue's time plus the effect's total duration).

use std::time::Duration;

use crate::lighting::effects::LayerSet;
use crate::lighting::parser::layout::layout;
use crate::lighting::parser::{parse_light_shows, Sources};
use crate::lighting::timeline::LightingTimeline;

/// Every cue of every show lands where the player's timeline fires it,
/// and every effect ends where the engine reckons its written end; a file
/// the player refuses is refused with the same words.
fn agrees_with_the_player(source: &str) {
    let lay = |until| {
        layout(
            LayerSet::standard(),
            source,
            None,
            None,
            &Sources::none(),
            until,
        )
    };
    let shows = match parse_light_shows(source) {
        Ok(shows) => shows,
        Err(error) => {
            let laid = lay(Duration::ZERO);
            assert_eq!(laid.error.as_deref(), Some(error.to_string().as_str()));
            return;
        }
    };
    let timeline = LightingTimeline::new(shows.values().cloned().collect());
    let mut fired: Vec<Duration> = timeline.cues().into_iter().map(|(t, _)| t).collect();
    fired.sort();
    fired.dedup();
    // Laid out to the last cue the player fires, so a `loop` reference's
    // passes are all there to compare.
    let laid = lay(fired.last().copied().unwrap_or_default());
    assert_eq!(laid.error, None, "{source}");
    let at = |ms: f64| Duration::from_secs_f64(ms / 1000.0);
    let near = |a: Duration, b: Duration| a.abs_diff(b) <= Duration::from_nanos(1);
    let mut laid_times: Vec<Duration> = laid
        .shows
        .iter()
        .flat_map(|show| show.cues.iter())
        .flat_map(|cue| {
            // Every placed cue of the written cue: its own, its loops' and
            // its passes'.
            std::iter::once(cue.ms)
                .chain(cue.effects.iter().map(|e| e.start_ms))
                .map(at)
        })
        .collect();
    laid_times.sort();
    laid_times.dedup();
    for time in &fired {
        assert!(
            laid_times.iter().any(|t| near(*t, *time)),
            "the player fires a cue at {time:?} the layout has not"
        );
    }
    let laid_ends: Vec<(Duration, Duration)> = laid
        .shows
        .iter()
        .flat_map(|show| show.cues.iter())
        .flat_map(|cue| cue.effects.iter())
        .map(|e| (at(e.start_ms), at(e.end_ms)))
        .collect();
    for cue in shows.values().flat_map(|show| show.cues.iter()) {
        for effect in &cue.effects {
            let end = cue.time.saturating_add(effect.total_duration());
            assert!(
                laid_ends
                    .iter()
                    .any(|(s, e)| near(*s, cue.time) && near(*e, end)),
                "an effect {:?}..{end:?} the layout has not",
                cue.time
            );
        }
    }
}

#[test]
fn the_examples_agree_with_the_player() {
    for entry in std::fs::read_dir("examples/lighting/shows").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("light") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        agrees_with_the_player(&source);
    }
}

/// `MTRACK_LAYOUT_FILE=<path>`: one more file to hold to the player (a
/// corpus run feeds them one at a time, each under its own limits).
#[test]
fn a_corpus_file_agrees_with_the_player() {
    let Ok(path) = std::env::var("MTRACK_LAYOUT_FILE") else {
        return;
    };
    let source = std::fs::read_to_string(&path).unwrap();
    agrees_with_the_player(&source);
}
