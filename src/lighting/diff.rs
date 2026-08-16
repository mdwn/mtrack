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
//! What changed between two versions of a light show.
//!
//! Diffing the text is close to useless here. Changing one bed's duration from
//! `8measures` to `31beats` is a one-line textual change that moves a section
//! boundary by a beat and opens a blackout; a cue moving from `@63/1` to
//! `@63/3` looks equally small and is a completely different edit. Worse, two
//! cues can be textually identical and land in different places if the tempo
//! block changed between versions.
//!
//! So this compares *resolved* effects — real times, real durations, after the
//! tempo map has been applied — and reports the change in coverage alongside,
//! since a revision is usually about what the rig does rather than what the
//! file says.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::lighting::analyze::{analyze_show, DarkWindow};
use crate::lighting::effects::EffectLayer;
use crate::lighting::parser::LightShow;
use crate::tempo::TempoMap;

/// One effect, resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffEntry {
    pub time: Duration,
    /// Musical position of `time`, when the show has a tempo map.
    pub position: Option<String>,
    /// The groups the effect targets, as authored.
    pub groups: String,
    /// Effect kind, e.g. `Static`.
    pub effect_type: &'static str,
    pub layer: EffectLayer,
    pub duration: Duration,
}

/// A field that differs between two versions of the same effect.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    pub field: &'static str,
    pub from: String,
    pub to: String,
}

/// An effect present in both versions but not identical.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedEntry {
    pub before: DiffEntry,
    pub after: DiffEntry,
    pub fields: Vec<FieldChange>,
}

/// How two shows differ.
#[derive(Clone, Debug, Default)]
pub struct ShowDiff {
    pub added: Vec<DiffEntry>,
    pub removed: Vec<DiffEntry>,
    pub changed: Vec<ChangedEntry>,
    /// Dark windows the second version has that the first does not.
    pub dark_windows_added: Vec<DarkWindow>,
    /// Dark windows the first version had that the second does not.
    pub dark_windows_removed: Vec<DarkWindow>,
}

impl ShowDiff {
    /// True when the two shows resolve identically.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Compares two versions of a show.
///
/// Effects are matched on what they target and what kind they are — the things
/// an author does not change when revising timing — so a cue moved from one
/// beat to another is reported as a changed `time` rather than as an unrelated
/// removal and addition. Effects that differ in group or type have genuinely
/// been swapped out, and are reported as such.
pub fn diff_shows(
    before: Vec<LightShow>,
    after: Vec<LightShow>,
    before_tempo: Option<&TempoMap>,
    after_tempo: Option<&TempoMap>,
) -> ShowDiff {
    let mut diff = ShowDiff::default();

    let mut old_entries = entries(&before, before_tempo);
    let mut new_entries = entries(&after, after_tempo);

    // Walk the union of keys so a group that only exists on one side still
    // gets reported rather than silently dropped.
    let mut keys: Vec<(String, &'static str)> = old_entries
        .keys()
        .chain(new_entries.keys())
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();

    for key in keys {
        let olds = old_entries.remove(&key).unwrap_or_default();
        let news = new_entries.remove(&key).unwrap_or_default();

        // Both sides are in time order, so zipping pairs the nth occurrence
        // with the nth. For a group whose count is unchanged this pairs every
        // effect with its counterpart; where the count differs, the surplus
        // falls out as added or removed.
        let paired = olds.len().min(news.len());
        for (old, new) in olds.iter().zip(news.iter()).take(paired) {
            let fields = field_changes(old, new);
            if !fields.is_empty() {
                diff.changed.push(ChangedEntry {
                    before: old.clone(),
                    after: new.clone(),
                    fields,
                });
            }
        }
        diff.removed.extend(olds.into_iter().skip(paired));
        diff.added.extend(news.into_iter().skip(paired));
    }

    diff.added.sort_by_key(|e| e.time);
    diff.removed.sort_by_key(|e| e.time);
    diff.changed.sort_by_key(|c| c.before.time);

    // The coverage change is usually the point of a revision — the author is
    // asking "did I create the blackouts I meant to, and no others".
    let before_dark = analyze_show(before, before_tempo).dark_windows;
    let after_dark = analyze_show(after, after_tempo).dark_windows;
    diff.dark_windows_added = difference(&after_dark, &before_dark);
    diff.dark_windows_removed = difference(&before_dark, &after_dark);

    diff
}

/// Resolved effects keyed by what they target and what kind they are, each list
/// in time order.
fn entries(
    shows: &[LightShow],
    tempo: Option<&TempoMap>,
) -> BTreeMap<(String, &'static str), Vec<DiffEntry>> {
    let mut out: BTreeMap<(String, &'static str), Vec<DiffEntry>> = BTreeMap::new();

    for show in shows {
        let map = show.tempo_map.as_ref().or(tempo);
        for cue in &show.cues {
            for effect in &cue.effects {
                let groups = effect.groups.join(", ");
                let effect_type = effect.effect_type.name();
                out.entry((groups.clone(), effect_type))
                    .or_default()
                    .push(DiffEntry {
                        time: cue.time,
                        position: map.map(|m| {
                            let (measure, beat) = m.time_to_measure_beat(cue.time);
                            format!("@{measure}/{beat:.2}")
                        }),
                        groups,
                        effect_type,
                        layer: effect.layer.unwrap_or(EffectLayer::Background),
                        duration: effect.total_duration(),
                    });
            }
        }
    }

    for list in out.values_mut() {
        list.sort_by_key(|e| e.time);
    }
    out
}

fn field_changes(old: &DiffEntry, new: &DiffEntry) -> Vec<FieldChange> {
    let mut fields = Vec::new();
    if old.time != new.time {
        fields.push(FieldChange {
            field: "time",
            from: seconds(old.time),
            to: seconds(new.time),
        });
    }
    if old.duration != new.duration {
        fields.push(FieldChange {
            field: "duration",
            from: seconds(old.duration),
            to: seconds(new.duration),
        });
    }
    if old.layer != new.layer {
        fields.push(FieldChange {
            field: "layer",
            from: format!("{:?}", old.layer).to_lowercase(),
            to: format!("{:?}", new.layer).to_lowercase(),
        });
    }
    fields
}

fn seconds(d: Duration) -> String {
    format!("{:.3}s", d.as_secs_f64())
}

/// Windows in `a` with no counterpart in `b`, compared on their bounds.
fn difference(a: &[DarkWindow], b: &[DarkWindow]) -> Vec<DarkWindow> {
    a.iter()
        .filter(|w| !b.iter().any(|o| o.from == w.from && o.to == w.to))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::parse_light_shows;

    fn shows(source: &str) -> Vec<LightShow> {
        let mut parsed: Vec<_> = parse_light_shows(source)
            .expect("test show should parse")
            .into_values()
            .collect();
        parsed.sort_by(|a, b| a.name.cmp(&b.name));
        parsed
    }

    fn diff(a: &str, b: &str) -> ShowDiff {
        diff_shows(shows(a), shows(b), None, None)
    }

    const BASE: &str = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:10.000
    wash: static color: "red", duration: 4s
}
"#;

    #[test]
    fn a_show_compared_with_itself_reports_nothing() {
        let d = diff(BASE, BASE);
        assert!(d.is_empty(), "{d:?}");
        assert!(d.dark_windows_added.is_empty());
        assert!(d.dark_windows_removed.is_empty());
    }

    #[test]
    fn an_added_cue_is_reported() {
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:10.000
    wash: static color: "red", duration: 4s

    @00:20.000
    wash: static color: "green", duration: 4s
}
"#;
        let d = diff(BASE, after);
        assert_eq!(d.added.len(), 1, "{d:?}");
        assert_eq!(d.added[0].time, Duration::from_secs(20));
        assert!(d.removed.is_empty());
        assert!(d.changed.is_empty());
    }

    #[test]
    fn a_removed_cue_is_reported() {
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s
}
"#;
        let d = diff(BASE, after);
        assert_eq!(d.removed.len(), 1, "{d:?}");
        assert_eq!(d.removed[0].time, Duration::from_secs(10));
        assert!(d.added.is_empty());
    }

    #[test]
    fn a_duration_change_is_reported_as_a_change_not_a_swap() {
        // The edit the issue calls out: `8measures` to `31beats` is one line of
        // text and a real change in what the rig does.
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:10.000
    wash: static color: "red", duration: 2s
}
"#;
        let d = diff(BASE, after);
        assert!(d.added.is_empty(), "{d:?}");
        assert!(d.removed.is_empty(), "{d:?}");
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].fields.len(), 1);
        assert_eq!(d.changed[0].fields[0].field, "duration");
        assert_eq!(d.changed[0].fields[0].from, "4.000s");
        assert_eq!(d.changed[0].fields[0].to, "2.000s");
    }

    #[test]
    fn a_moved_cue_is_a_time_change_not_an_add_and_a_remove() {
        // Matching on what an effect targets rather than when it fires means a
        // cue nudged by a beat reads as a move, which is what it is.
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:12.000
    wash: static color: "red", duration: 4s
}
"#;
        let d = diff(BASE, after);
        assert!(d.added.is_empty(), "{d:?}");
        assert!(d.removed.is_empty(), "{d:?}");
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].fields[0].field, "time");
        assert_eq!(d.changed[0].fields[0].from, "10.000s");
        assert_eq!(d.changed[0].fields[0].to, "12.000s");
    }

    #[test]
    fn a_different_group_is_a_swap_not_a_change() {
        // Retargeting an effect is not a tweak to the same cue.
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:10.000
    spot: static color: "red", duration: 4s
}
"#;
        let d = diff(BASE, after);
        assert_eq!(d.removed.len(), 1, "{d:?}");
        assert_eq!(d.removed[0].groups, "wash");
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].groups, "spot");
    }

    #[test]
    fn a_layer_move_is_reported() {
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s, layer: foreground

    @00:10.000
    wash: static color: "red", duration: 4s
}
"#;
        let d = diff(BASE, after);
        assert_eq!(d.changed.len(), 1, "{d:?}");
        assert_eq!(d.changed[0].fields[0].field, "layer");
        assert_eq!(d.changed[0].fields[0].to, "foreground");
    }

    // ── the field the issue said it would actually use ─────────────

    #[test]
    fn a_deliberately_created_blackout_shows_up_as_a_dark_window() {
        // The whole point of the reporter's v2 revision: end a section early to
        // open a blackout, and confirm you made the ones you meant to.
        let before = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s

    @00:10.000
    wash: static color: "red", duration: 4s
}
"#;
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 9s

    @00:10.000
    wash: static color: "red", duration: 4s
}
"#;
        let d = diff(before, after);
        assert_eq!(d.dark_windows_added.len(), 1, "{d:?}");
        let window = &d.dark_windows_added[0];
        assert_eq!(window.from, Duration::from_secs(9));
        assert_eq!(window.to, Duration::from_secs(10));
        assert!(d.dark_windows_removed.is_empty());
    }

    #[test]
    fn closing_a_gap_shows_up_as_a_removed_dark_window() {
        let after = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s

    @00:10.000
    wash: static color: "red", duration: 4s
}
"#;
        // BASE has a 4s effect at 0 and a gap from 4s to 10s.
        let d = diff(BASE, after);
        assert_eq!(d.dark_windows_removed.len(), 1, "{d:?}");
        assert_eq!(d.dark_windows_removed[0].from, Duration::from_secs(4));
        assert!(d.dark_windows_added.is_empty());
    }

    #[test]
    fn identical_cues_land_differently_when_the_tempo_block_changes() {
        // The case that makes a text diff useless: not one character of the cue
        // changed, and it fires two seconds later.
        let before = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @3/1
    wash: static color: "blue", duration: 1s
}
"#;
        let after = r#"
tempo {
    start: 0ms
    bpm: 60
    time_signature: 4/4
}

show "T" {
    @3/1
    wash: static color: "blue", duration: 1s
}
"#;
        let d = diff(before, after);
        assert_eq!(d.changed.len(), 1, "{d:?}");
        assert_eq!(d.changed[0].fields[0].field, "time");
        // 120bpm puts bar 3 at 4s; 60bpm puts it at 8s.
        assert_eq!(d.changed[0].fields[0].from, "4.000s");
        assert_eq!(d.changed[0].fields[0].to, "8.000s");
    }

    #[test]
    fn positions_are_reported_when_a_tempo_map_exists() {
        let before = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @3/1
    wash: static color: "blue", duration: 1s
}
"#;
        let after = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @3/1
    wash: static color: "blue", duration: 2s
}
"#;
        let d = diff(before, after);
        assert_eq!(d.changed.len(), 1);
        assert!(
            d.changed[0].before.position.is_some(),
            "{:?}",
            d.changed[0].before
        );
    }

    #[test]
    fn diffing_against_an_empty_show_reports_everything_removed() {
        let d = diff(BASE, "show \"T\" {\n}\n");
        assert_eq!(d.removed.len(), 2, "{d:?}");
        assert!(d.added.is_empty());
    }
}
