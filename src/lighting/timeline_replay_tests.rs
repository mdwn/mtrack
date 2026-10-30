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

//! A seek's replay of a show's history, and the show's end it gives,
//! against the cues as the timeline fires them live: same-time cues that
//! start and stop sequences.

use super::*;

fn parse(src: &str) -> Vec<LightShow> {
    crate::lighting::parser::parse_light_shows(src)
        .unwrap()
        .into_values()
        .collect()
}

/// Live, the effects update() fires up to `t` from a fresh start, stopped
/// sequences taken out as the engine does, and those still running at `t`
/// by authored duration (no holds in these shows).
fn live_running_at(src: &str, t: Duration) -> Vec<String> {
    let mut tl = LightingTimeline::new(parse(src));
    tl.start();
    let mut running: Vec<(String, Duration, Duration, String)> = Vec::new();
    let mut at = Duration::ZERO;
    while at <= t {
        let u = tl.update(at);
        for s in &u.stop_sequences {
            let p = format!("seq_{}_", s);
            running.retain(|(id, ..)| !id.starts_with(&p));
        }
        for e in u.effects {
            running.push((
                e.id.clone(),
                e.cue_time.unwrap(),
                e.total_duration(),
                format!("{}@{:?}", e.target_fixtures.join(","), e.cue_time.unwrap()),
            ));
        }
        at += Duration::from_millis(10);
    }
    running
        .into_iter()
        .filter(|(_, c, d, _)| *c + *d > t)
        .map(|(.., g)| g)
        .collect()
}

fn seek_running_at(src: &str, t: Duration) -> Vec<String> {
    let mut tl = LightingTimeline::new(parse(src));
    tl.start_at(t)
        .effects_with_elapsed
        .values()
        .map(|(e, _)| format!("{}@{:?}", e.target_fixtures.join(","), e.cue_time.unwrap()))
        .collect()
}

const RESTART: &str = "sequence \"x\" {\n    @00:00.000\n    spots: static red: 100%, duration: 1s\n    \
    @00:02.000\n    wash: static blue: 100%, duration: 2s\n    @00:03.000\n    spots: static red: 0%, duration: 1s\n}\n\n\
    show \"s\" {\n    @00:01.000\n    sequence \"x\", loop: 3\n    @00:02.000\n    stop sequence \"x\"\n    \
    @00:07.000\n    sequence \"x\"\n}\n";

#[test]
fn replay_restart_same_time_as_stopped_invocation_cue() {
    let t = Duration::from_millis(7500);
    let mut live = live_running_at(RESTART, t);
    let mut seek = seek_running_at(RESTART, t);
    live.sort();
    seek.sort();
    assert_eq!(seek, live, "seek vs live at 7.5 s");
}

const STOP_SAME_TIME: &str =
    "sequence \"y\" {\n    @00:00.000\n    spots: static red: 100%, duration: 10s\n    \
    @00:03.000\n    wash: static blue: 100%, duration: 10s\n}\n\n\
    show \"s\" {\n    @00:00.000\n    sequence \"y\"\n    @00:03.000\n    stop sequence \"y\"\n    \
    @00:04.000\n    freeze(layer: foreground)\n}\n";

#[test]
fn replay_stop_at_same_time_as_sequence_cue_show_end() {
    // Live, the wash cued at 3 s starts after the stop applies, and runs.
    let live = live_running_at(STOP_SAME_TIME, Duration::from_secs(5));
    assert_eq!(live, vec!["wash@3s".to_string()], "live");
    // Without the unrelated freeze, the lint reckoning says 13 s.
    let no_hold = STOP_SAME_TIME.replace("    @00:04.000\n    freeze(layer: foreground)\n", "");
    assert_eq!(
        LightingTimeline::new(parse(&no_hold)).show_end(),
        Duration::from_secs(13)
    );
    // An unrelated freeze sends it through the replay, which has to keep the
    // wash too.
    let tl = LightingTimeline::new(parse(STOP_SAME_TIME));
    assert_eq!(
        tl.show_end(),
        Duration::from_secs(13),
        "with a freeze elsewhere"
    );
}

const STOP_THEN_RESTART: &str =
    "sequence \"x\" {\n    @00:00.000\n    spots: static red: 100%, duration: 10s\n}\n\n\
    show \"s\" {\n    @00:00.000\n    sequence \"x\"\n    @00:04.000\n    stop sequence \"x\"\n    \
    @00:04.000\n    sequence \"x\"\n}\n";

#[test]
fn replay_stop_then_restart_at_same_time() {
    let t = Duration::from_secs(5);
    let live = live_running_at(STOP_THEN_RESTART, t);
    assert_eq!(live, vec!["spots@4s".to_string()], "live");
    assert_eq!(seek_running_at(STOP_THEN_RESTART, t), live, "seek");
}

#[test]
fn replay_stop_then_restart_keeps_later_passes_after_seek() {
    let looped = STOP_THEN_RESTART.replace(
        "    @00:04.000\n    sequence \"x\"\n",
        "    @00:04.000\n    sequence \"x\", loop: 3\n",
    );
    let mut tl = LightingTimeline::new(parse(&looped));
    let _ = tl.start_at(Duration::from_secs(5));
    let later = tl.update(Duration::from_secs(15));
    assert_eq!(later.effects.len(), 1, "the restarted sequence's 14 s pass");
}

#[test]
fn replay_stop_then_restart_at_same_time_show_end() {
    // With an unrelated freeze, the show end goes through the replay too.
    let held = STOP_THEN_RESTART.replace(
        "    sequence \"x\"\n}\n",
        "    sequence \"x\"\n    @00:06.000\n    freeze(layer: foreground)\n}\n",
    );
    assert!(held.contains("@00:06.000"));
    assert_eq!(
        LightingTimeline::new(parse(&held)).show_end(),
        Duration::from_secs(14),
        "the restarted spots run 4-14 s"
    );
}

#[test]
fn replay_stop_at_same_time_as_sequence_cue_seek() {
    let t = Duration::from_secs(5);
    assert_eq!(
        seek_running_at(STOP_SAME_TIME, t),
        live_running_at(STOP_SAME_TIME, t)
    );
}

/// A freeze or speed master that holds nothing leaves the show's end where
/// it was written: not at the last cue.
#[test]
fn replay_a_hold_on_nothing_keeps_the_written_end() {
    let plain = "show \"s\" {\n    @00:00.000\n    spots: static red: 100%, duration: 4s\n    \
                 @00:10.000\n    clear(layer: foreground)\n}\n";
    let held = plain.replace(
        "    @00:10.000\n",
        "    @00:01.000\n    master(layer: foreground, speed: 1)\n    @00:10.000\n",
    );
    assert_eq!(
        LightingTimeline::new(parse(plain)).show_end(),
        Duration::from_secs(4)
    );
    assert_eq!(
        LightingTimeline::new(parse(&held)).show_end(),
        Duration::from_secs(4)
    );
}

fn end_of(src: &str) -> Duration {
    LightingTimeline::new(parse(src)).show_end()
}

/// An arrived move stays in the replay for its pose, but it ended when it
/// arrived: later commands on its layer do not carry the show's end on.
#[test]
fn replay_an_arrived_move_ends_the_show_when_it_arrives() {
    let tail = "    @00:03.000\n    master(layer: background, speed: 0.5)\n    \
                @00:05.000\n    master(layer: foreground, intensity: 50%)\n}\n";
    let stat = format!(
        "show \"s\" {{\n    @00:00.000\n    w: static, color: \"blue\", duration: 1s\n{tail}"
    );
    let mv = format!(
        "show \"s\" {{\n    @00:00.000\n    m: move from_pan: 0deg, from_tilt: 0deg, pan: 90deg, tilt: 0deg, duration: 1s\n{tail}"
    );
    assert_eq!(end_of(&stat), Duration::from_secs(1), "static");
    assert_eq!(end_of(&mv), Duration::from_secs(1), "move over at 1 s too");
}

/// An intensity-only master at the seek point is not a freeze, unfreeze or
/// speed change: an effect ending exactly there is over in the seek.
#[test]
fn replay_an_intensity_master_does_not_hold_a_look_at_its_end() {
    let src = "show \"s\" {\n    @00:00.000\n    w: static, color: \"blue\", duration: 2s\n    \
               @00:02.000\n    master(layer: background, intensity: 50%)\n}\n";
    let mut tl = LightingTimeline::new(parse(src));
    let h = tl.start_at(Duration::from_secs(2));
    assert_eq!(
        h.effects_with_elapsed.len(),
        0,
        "{:?}",
        h.effects_with_elapsed
            .values()
            .map(|(_, e)| *e)
            .collect::<Vec<_>>()
    );
}

/// A full clear cuts only what is still running: a look long over ended
/// on its own, not at the clear.
#[test]
fn replay_a_full_clear_does_not_carry_the_show_end() {
    let src = "show \"s\" {\n    @00:00.000\n    w: static, color: \"blue\", duration: 1s\n    \
               @00:01.000\n    master(layer: foreground, speed: 0.5)\n    \
               @00:03.000\n    clear()\n    \
               @00:05.000\n    master(layer: foreground, intensity: 50%)\n}\n";
    assert_eq!(end_of(src), Duration::from_secs(1));
}
