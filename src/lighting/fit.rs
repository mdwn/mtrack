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

//! Fitting a project's shows to a venue (design `lighting_ui_design.md` §9).
//!
//! An imported venue arrives untagged, so every group a show uses finds
//! nothing. This module answers "which fixtures should carry which tags?" as a
//! *suggestion* a user can check: a set of fixtures, the tags that would put
//! them in the group, and a structured reason. The rules are pure functions
//! over plain data ([`suggest`]) so they are unit-testable and so the web UI
//! and the MCP `suggest_group_tags` tool, which both go through
//! [`FitReport::gather`], cannot disagree.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::lighting::{GroupConstraint, LogicalGroup};
use crate::lighting::effects::{EffectType, FixtureCapabilities, FixtureInfo, MoveTarget};
use crate::lighting::parser::LightShow;

/// Something a show asks of a group, judged by the same rules as lint's
/// `capability-gap` (and its `cells-absent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Want {
    /// A `move`: pan or tilt channels.
    Move,
    /// A colour cue (static colour, cycle, rainbow): colour channels.
    Color,
    /// A `strobe`: a strobe channel.
    Strobe,
    /// `per: cell`: a pixel fixture.
    Cells,
    /// A `dimmer` or `pulse`: a dimmer channel or colour channels.
    Dimmer,
}

impl Want {
    /// The name used in the API.
    pub fn name(self) -> &'static str {
        match self {
            Want::Move => "move",
            Want::Color => "color",
            Want::Strobe => "strobe",
            Want::Cells => "cells",
            Want::Dimmer => "dimmer",
        }
    }
}

/// What the shows ask of each group they target.
pub fn wants_by_group(shows: &[LightShow]) -> BTreeMap<String, BTreeSet<Want>> {
    let mut out: BTreeMap<String, BTreeSet<Want>> = BTreeMap::new();
    for effect in shows
        .iter()
        .flat_map(|s| s.cues.iter())
        .flat_map(|c| c.effects.iter())
    {
        let mut wants = BTreeSet::new();
        match &effect.effect_type {
            EffectType::Strobe { .. } => {
                wants.insert(Want::Strobe);
            }
            EffectType::Move { .. } => {
                wants.insert(Want::Move);
            }
            EffectType::Dimmer { .. } | EffectType::Pulse { .. } => {
                wants.insert(Want::Dimmer);
            }
            EffectType::ColorCycle { .. } | EffectType::Rainbow { .. } => {
                wants.insert(Want::Color);
            }
            // A static's colour arrives as red, green and blue.
            EffectType::Static { parameters, .. }
                if ["red", "green", "blue"]
                    .iter()
                    .any(|c| parameters.contains_key(*c)) =>
            {
                wants.insert(Want::Color);
            }
            _ => {}
        }
        if effect.per_cell {
            wants.insert(Want::Cells);
        }
        for group in &effect.groups {
            out.entry(group.clone()).or_default().extend(&wants);
        }
    }
    out
}

/// Every focus point the shows aim a `move` at, with no regard to the venue,
/// keyed by name and mapped to the shows' names.
pub fn focus_names(shows: &[LightShow]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for effect in shows
        .iter()
        .flat_map(|s| s.cues.iter())
        .flat_map(|c| c.effects.iter())
    {
        if let EffectType::Move { to, from, .. } = &effect.effect_type {
            for target in [Some(to), from.as_ref()].into_iter().flatten() {
                if let MoveTarget::Focus(name) = target {
                    out.insert(name.clone());
                }
            }
        }
    }
    out
}

/// The tags a group's constraints ask of a fixture.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Needs {
    /// Every one must be present.
    pub all_of: Vec<String>,
    /// At least one must be present.
    pub any_of: Vec<String>,
    /// Preferred, never required.
    pub prefer: Vec<String>,
}

impl Needs {
    pub fn of(group: &LogicalGroup) -> Needs {
        let mut needs = Needs::default();
        for constraint in group.constraints() {
            match constraint {
                GroupConstraint::AllOf(tags) => needs.all_of.extend(tags.iter().cloned()),
                GroupConstraint::AnyOf(tags) => needs.any_of.extend(tags.iter().cloned()),
                GroupConstraint::Prefer(tags) => needs.prefer.extend(tags.iter().cloned()),
                _ => {}
            }
        }
        needs
    }

    /// The tags that would put a fixture in the group: every `AllOf` tag, plus
    /// the first of `AnyOf` (any one satisfies it; the first is the author's
    /// first choice).
    pub fn tags_to_apply(&self) -> Vec<String> {
        let mut tags = self.all_of.clone();
        if let Some(first) = self.any_of.first() {
            tags.push(first.clone());
        }
        let mut seen = BTreeSet::new();
        tags.retain(|t| seen.insert(t.clone()));
        tags
    }

    /// Nothing to tag for.
    pub fn is_empty(&self) -> bool {
        self.all_of.is_empty() && self.any_of.is_empty()
    }
}

/// How high a fixture hangs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Height {
    /// z < 0.5 m.
    Deck,
    /// z < 2.5 m.
    Low,
    /// Above.
    Truss,
}

/// How far upstage a fixture hangs, by thirds of the venue's y-extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Downstage,
    Mid,
    Upstage,
}

impl Height {
    pub fn of(z: f64) -> Height {
        if z < 0.5 {
            Height::Deck
        } else if z < 2.5 {
            Height::Low
        } else {
            Height::Truss
        }
    }

    fn word(self) -> &'static str {
        match self {
            Height::Deck => "deck",
            Height::Low => "low rig",
            Height::Truss => "truss",
        }
    }
}

impl Depth {
    fn word(self) -> &'static str {
        match self {
            Depth::Downstage => "downstage",
            Depth::Mid => "midstage",
            Depth::Upstage => "upstage",
        }
    }
}

/// A venue fixture, reduced to what fitting needs.
#[derive(Clone, Debug)]
pub struct FitFixture {
    pub name: String,
    pub fixture_type: String,
    pub tags: Vec<String>,
    pub position: Option<[f64; 3]>,
    pub capabilities: FixtureCapabilities,
    pub cells: bool,
    /// Colour comes from a wheel (`color1`, `color2`, ...) rather than a
    /// mix: the engine models no wheel, so it is not `color`.
    pub color_wheel: bool,
}

impl FitFixture {
    /// `info` from the loaded system, `tags` from the venue's fixture.
    pub fn from_info(info: &FixtureInfo, tags: &[String]) -> FitFixture {
        FitFixture {
            name: info.name.clone(),
            fixture_type: info.fixture_type.clone(),
            tags: tags.to_vec(),
            position: info.position,
            capabilities: info.capabilities(),
            cells: !info.cells.is_empty(),
            color_wheel: !info.capabilities().contains(FixtureCapabilities::RGB_COLOR)
                && info.channels.keys().any(|name| is_wheel_channel(name)),
        }
    }

    /// Whether the fixture can do what `want` asks, by lint's rules.
    pub fn can(&self, want: Want) -> bool {
        match want {
            Want::Move => {
                self.capabilities.contains(FixtureCapabilities::PANNING)
                    || self.capabilities.contains(FixtureCapabilities::TILTING)
            }
            Want::Color => self.capabilities.contains(FixtureCapabilities::RGB_COLOR),
            Want::Strobe => self.capabilities.contains(FixtureCapabilities::STROBING),
            Want::Cells => self.cells,
            Want::Dimmer => {
                self.capabilities.contains(FixtureCapabilities::DIMMING)
                    || self.capabilities.contains(FixtureCapabilities::RGB_COLOR)
            }
        }
    }

    /// The capability names the pages show: the four shows can ask for
    /// (`color`, `pan_tilt`, `strobe`, `cells`) first, then what else the
    /// fixture has — `dimmer`, `white`, `zoom`, `focus`, `gobo` and
    /// `color_wheel`.
    pub fn capability_names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.can(Want::Color) {
            out.push("color");
        }
        if self.can(Want::Move) {
            out.push("pan_tilt");
        }
        if self.can(Want::Strobe) {
            out.push("strobe");
        }
        if self.can(Want::Cells) {
            out.push("cells");
        }
        for (bit, name) in [
            (FixtureCapabilities::DIMMING, "dimmer"),
            (FixtureCapabilities::WHITE_COLOR, "white"),
            (FixtureCapabilities::ZOOMING, "zoom"),
            (FixtureCapabilities::FOCUSING, "focus"),
            (FixtureCapabilities::GOBO, "gobo"),
        ] {
            if self.capabilities.contains(bit) {
                out.push(name);
            }
        }
        if self.color_wheel {
            out.push("color_wheel");
        }
        out
    }
}

/// Whether a channel name is a colour wheel's: `color1`, `color2`, ... (what
/// the GDTF distiller names an attribute it has no canonical name for).
fn is_wheel_channel(name: &str) -> bool {
    name.strip_prefix("color")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// Where a cluster hangs. Both `None` when the venue does not place it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Place {
    height: Option<Height>,
    depth: Option<Depth>,
}

impl Place {
    fn words(self) -> Option<String> {
        match (self.depth, self.height) {
            (Some(d), Some(h)) => Some(format!("{} {}", d.word(), h.word())),
            (None, Some(h)) => Some(h.word().to_string()),
            (Some(d), None) => Some(d.word().to_string()),
            (None, None) => None,
        }
    }
}

/// One cluster of like fixtures: the same type, hanging in the same place.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Cluster {
    pub fixtures: Vec<String>,
    #[serde(rename = "type")]
    pub fixture_type: String,
    /// The place in words ("upstage truss"); absent when unplaced.
    #[serde(rename = "where")]
    pub place: Option<String>,
    pub height: Option<Height>,
    pub depth: Option<Depth>,
}

/// Why a cluster is the suggestion.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Reason {
    pub count: usize,
    #[serde(rename = "type")]
    pub fixture_type: String,
    #[serde(rename = "where")]
    pub place: Option<String>,
    pub height: Option<Height>,
    pub depth: Option<Depth>,
    /// What the shows ask of the group, which the cluster can do.
    pub can: Vec<Want>,
}

/// Fixtures and the tags that would put them in a group.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub fixtures: Vec<String>,
    pub tags: Vec<String>,
    pub reason: Reason,
}

/// The answer for one group.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Fit {
    pub suggestion: Option<Suggestion>,
    pub others: Vec<Cluster>,
    /// With no suggestion: the wants no fixture in the venue meets.
    pub unmet: Vec<Want>,
    /// With no suggestion and every want met by *some* fixture: none meets them
    /// all at once.
    pub unmet_together: bool,
}

/// Bands the depth of `y` in thirds of `[min, max]`; `None` when the venue's
/// extent is nil, since one depth is no information.
fn depth_of(y: f64, range: Option<(f64, f64)>) -> Option<Depth> {
    let (min, max) = range?;
    let third = (max - min) / 3.0;
    if third <= 0.0 {
        return None;
    }
    Some(if y < min + third {
        Depth::Downstage
    } else if y < min + 2.0 * third {
        Depth::Mid
    } else {
        Depth::Upstage
    })
}

/// Suggests which of `fixtures` should carry `needs` for a group the shows ask
/// `wants` of.
///
/// Candidates are the fixtures able to do every want; they are clustered by
/// type, then (for fixtures the venue places) by height and depth band. The
/// largest cluster is the suggestion (ties: type name, then place, then first
/// fixture name) and the rest are `others`. `fixtures` must be the whole venue,
/// because the depth bands are thirds of *its* y-extent.
pub fn suggest(needs: &Needs, wants: &BTreeSet<Want>, fixtures: &[FitFixture]) -> Fit {
    // Nothing to tag for: an undeclared group, or one selected by count alone.
    if needs.is_empty() {
        return Fit::default();
    }
    let y_range = fixtures
        .iter()
        .filter_map(|f| f.position.map(|p| p[1]))
        .fold(None, |acc: Option<(f64, f64)>, y| match acc {
            None => Some((y, y)),
            Some((lo, hi)) => Some((lo.min(y), hi.max(y))),
        });

    let mut clusters: BTreeMap<(String, Place), Vec<String>> = BTreeMap::new();
    for fixture in fixtures.iter().filter(|f| wants.iter().all(|w| f.can(*w))) {
        let place = match fixture.position {
            Some(p) => Place {
                height: Some(Height::of(p[2])),
                depth: depth_of(p[1], y_range),
            },
            None => Place {
                height: None,
                depth: None,
            },
        };
        clusters
            .entry((fixture.fixture_type.clone(), place))
            .or_default()
            .push(fixture.name.clone());
    }

    let mut found: Vec<(Place, Cluster)> = clusters
        .into_iter()
        .map(|((fixture_type, place), mut names)| {
            names.sort();
            (
                place,
                Cluster {
                    fixtures: names,
                    fixture_type,
                    place: place.words(),
                    height: place.height,
                    depth: place.depth,
                },
            )
        })
        .collect();
    // Largest first; the BTreeMap order already broke ties by type, then place.
    found.sort_by(|a, b| b.1.fixtures.len().cmp(&a.1.fixtures.len()));

    let mut clusters = found.into_iter().map(|(_, c)| c);
    let Some(first) = clusters.next() else {
        let unmet: Vec<Want> = wants
            .iter()
            .copied()
            .filter(|w| !fixtures.iter().any(|f| f.can(*w)))
            .collect();
        let together = unmet.is_empty() && !wants.is_empty();
        return Fit {
            suggestion: None,
            others: Vec::new(),
            unmet: if together {
                wants.iter().copied().collect()
            } else {
                unmet
            },
            unmet_together: together,
        };
    };
    Fit {
        suggestion: Some(Suggestion {
            fixtures: first.fixtures.clone(),
            tags: needs.tags_to_apply(),
            reason: Reason {
                count: first.fixtures.len(),
                fixture_type: first.fixture_type,
                place: first.place,
                height: first.height,
                depth: first.depth,
                can: wants.iter().copied().collect(),
            },
        }),
        others: clusters.collect(),
        unmet: Vec::new(),
        unmet_together: false,
    }
}

/// Everything the fit page and `suggest_group_tags` say, gathered from the
/// running player.
pub struct FitReport {
    /// `None` when the player has no DMX engine or no current venue.
    pub venue: Option<Value>,
    /// One entry per group any show targets, in name order.
    pub groups: Vec<Value>,
    pub focus_points_wanted: Vec<Value>,
    pub output: Value,
}

impl FitReport {
    /// Blocking: takes the lighting-system mutex and asks olad. Async callers
    /// belong on the blocking pool.
    pub fn gather(player: &crate::player::Player) -> FitReport {
        use crate::lighting::readiness::{group_names, songs_with_lighting};

        let songs = player.songs();
        let with_lighting = songs_with_lighting(&songs);
        let all_shows: Vec<LightShow> = with_lighting
            .iter()
            .flat_map(|(_, shows)| shows.iter().cloned())
            .collect();
        let names = group_names(&all_shows);
        let wants = wants_by_group(&all_shows);
        let wanted_focus = focus_names(&all_shows);

        let dmx = player.dmx_engine();
        let configured = dmx
            .as_ref()
            .map(|d| d.configured_universes())
            .unwrap_or_default();
        let mut report = FitReport {
            venue: None,
            groups: Vec::new(),
            focus_points_wanted: Vec::new(),
            output: json!({
                "unconfigured": [],
                "unpatched": [],
                "reachable": null,
                "ola_http_port": dmx.as_ref().map(|d| d.ola_http_port()),
            }),
        };
        let Some(system) = dmx
            .as_ref()
            .and_then(|d| d.broadcast_handles().lighting_system)
        else {
            return report;
        };

        let mut used_universes: BTreeSet<u16> = BTreeSet::new();
        {
            let mut guard = system.lock();
            let Some(venue) = guard.get_current_venue() else {
                return report;
            };
            let venue_name = venue.name().to_string();
            let focus_points: Vec<String> = venue.focus_points().keys().cloned().collect();
            let infos = guard.get_current_venue_fixtures().unwrap_or_default();
            let mut fit_fixtures: Vec<FitFixture> = infos
                .iter()
                .filter_map(|info| {
                    let fixture = venue.fixtures().get(&info.name)?;
                    Some(FitFixture::from_info(info, fixture.tags()))
                })
                .collect();
            fit_fixtures.sort_by(|a, b| a.name.cmp(&b.name));
            for fixture in venue.fixtures().values() {
                used_universes.insert(fixture.universe());
            }

            report.venue = Some(json!({
                "name": venue_name,
                "fixtures": fit_fixtures.iter().map(|f| json!({
                    "name": f.name,
                    "type": f.fixture_type,
                    "tags": f.tags,
                    "position": f.position,
                    "capabilities": f.capability_names(),
                })).collect::<Vec<_>>(),
                "focus_points": focus_points,
            }));

            for name in &names {
                let members = guard.resolve_logical_group_graceful(name);
                let defined = guard
                    .logical_groups_iter()
                    .find(|(n, _)| n.as_str() == name)
                    .map(|(_, g)| Needs::of(g));
                let group_wants = wants.get(name).cloned().unwrap_or_default();
                let songs_using: Vec<&str> = with_lighting
                    .iter()
                    .filter(|(_, shows)| group_names(shows).contains(name))
                    .map(|(song, _)| song.name())
                    .collect();
                let fit = match (&defined, members.is_empty()) {
                    (Some(needs), true) => suggest(needs, &group_wants, &fit_fixtures),
                    _ => Fit::default(),
                };
                report.groups.push(json!({
                    "name": name,
                    "defined": defined.is_some(),
                    "needs": defined.clone().unwrap_or_default(),
                    "fixtures": members,
                    "songs": songs_using,
                    "wants": group_wants.iter().map(|w| w.name()).collect::<Vec<_>>(),
                    "suggestion": fit.suggestion,
                    "others": fit.others,
                    "unmet": fit.unmet,
                    "unmet_together": fit.unmet_together,
                }));
            }

            for focus in wanted_focus.iter().filter(|f| !focus_points.contains(f)) {
                let songs_aiming: Vec<&str> = with_lighting
                    .iter()
                    .filter(|(_, shows)| focus_names(shows).contains(focus))
                    .map(|(song, _)| song.name())
                    .collect();
                report
                    .focus_points_wanted
                    .push(json!({"name": focus, "songs": songs_aiming}));
            }
        }

        let unconfigured: Vec<u16> = used_universes
            .iter()
            .copied()
            .filter(|u| !configured.contains(u))
            .collect();
        // As the readiness hub: ask olad about the universes the venue streams
        // to that have an output; a universe with none is never streamed to.
        let to_probe: Vec<u16> = used_universes
            .iter()
            .copied()
            .filter(|u| configured.contains(u))
            .collect();
        let (unpatched, reachable) = match &dmx {
            Some(dmx) if !to_probe.is_empty() => {
                let probe =
                    crate::dmx::patch_check::probe_universes(dmx.ola_http_port(), &to_probe);
                (probe.unpatched, json!(probe.reachable))
            }
            _ => (Vec::new(), Value::Null),
        };
        report.output = json!({
            "unconfigured": unconfigured,
            "unpatched": unpatched,
            "reachable": reachable,
            "ola_http_port": dmx.as_ref().map(|d| d.ola_http_port()),
        });
        report
    }

    /// The JSON the endpoint serves.
    pub fn to_json(&self) -> Value {
        json!({
            "venue": self.venue,
            "groups": self.groups,
            "focus_points_wanted": self.focus_points_wanted,
            "output": self.output,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn caps(list: &[FixtureCapabilities]) -> FixtureCapabilities {
        list.iter()
            .fold(FixtureCapabilities::NONE, |acc, c| acc.with(*c))
    }

    fn fixture(
        name: &str,
        fixture_type: &str,
        position: Option<[f64; 3]>,
        capabilities: &[FixtureCapabilities],
    ) -> FitFixture {
        FitFixture {
            name: name.into(),
            fixture_type: fixture_type.into(),
            tags: vec![],
            position,
            capabilities: caps(capabilities),
            cells: false,
            color_wheel: false,
        }
    }

    const MOVER: &[FixtureCapabilities] = &[
        FixtureCapabilities::PANNING,
        FixtureCapabilities::TILTING,
        FixtureCapabilities::RGB_COLOR,
    ];
    const WASH: &[FixtureCapabilities] = &[FixtureCapabilities::RGB_COLOR];

    fn needs(all_of: &[&str], any_of: &[&str]) -> Needs {
        Needs {
            all_of: all_of.iter().map(|s| s.to_string()).collect(),
            any_of: any_of.iter().map(|s| s.to_string()).collect(),
            prefer: vec![],
        }
    }

    fn wants(list: &[Want]) -> BTreeSet<Want> {
        list.iter().copied().collect()
    }

    fn shows(src: &str) -> Vec<LightShow> {
        crate::lighting::parser::parse_light_shows_with_tempo(src, None)
            .unwrap()
            .into_values()
            .collect()
    }

    #[test]
    fn wants_follow_lints_capability_rules() {
        let shows = shows(
            "show \"S\" {\n    @00:00.000\n    movers: move focus: \"drummer\", duration: 1s\n    \
             movers: static color: \"red\", duration: 1s\n    \
             flash: strobe frequency: 8, duration: 1s\n    \
             bars: rainbow duration: 2s, per: cell\n    \
             dim: static dimmer: 0.5, duration: 1s\n}\n",
        );
        let wants = wants_by_group(&shows);
        assert_eq!(wants["movers"], self::wants(&[Want::Move, Want::Color]));
        assert_eq!(wants["flash"], self::wants(&[Want::Strobe]));
        assert_eq!(wants["bars"], self::wants(&[Want::Color, Want::Cells]));
        assert!(!wants.contains_key("nobody"));
        // A dimmer asks for nothing worth tagging on.
        assert!(wants["dim"].is_empty());
    }

    #[test]
    fn dimmer_and_pulse_want_a_dimmer_or_colour() {
        let shows = shows(
            "show \"S\" {\n    @00:00.000\n    a: dimmer start: 100%, end: 0%, duration: 1s\n    \
             b: pulse frequency: 1, intensity: 0.5, duration: 1s\n}\n",
        );
        let wants = wants_by_group(&shows);
        assert_eq!(wants["a"], self::wants(&[Want::Dimmer]));
        assert_eq!(wants["b"], self::wants(&[Want::Dimmer]));

        let only_dimmer = fixture("d", "Dim", None, &[FixtureCapabilities::DIMMING]);
        let only_rgb = fixture("c", "Rgb", None, WASH);
        let neither = fixture("n", "Pan", None, &[FixtureCapabilities::PANNING]);
        assert!(only_dimmer.can(Want::Dimmer));
        assert!(only_rgb.can(Want::Dimmer));
        assert!(!neither.can(Want::Dimmer));

        // A candidate must satisfy it, so applying leaves no capability-gap.
        let fixtures = vec![neither, only_dimmer];
        let s = suggest(
            &needs(&["g"], &[]),
            &self::wants(&[Want::Dimmer]),
            &fixtures,
        )
        .suggestion
        .unwrap();
        assert_eq!(s.fixtures, vec!["d".to_string()]);
    }

    #[test]
    fn focus_names_come_from_both_ends_of_a_move() {
        let shows = shows(
            "show \"S\" {\n    @00:00.000\n    movers: move from: \"a\", to: \"b\", duration: 1s\n    \
             movers: move pan: 10deg, tilt: 5deg, duration: 1s\n}\n",
        );
        let names: Vec<String> = focus_names(&shows).into_iter().collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn needs_take_all_of_and_the_first_any_of() {
        let group = LogicalGroup::new(
            "g".into(),
            vec![
                GroupConstraint::AllOf(vec!["moving_head".into(), "hero".into()]),
                GroupConstraint::AnyOf(vec!["left".into(), "right".into()]),
                GroupConstraint::Prefer(vec!["front".into()]),
                GroupConstraint::MinCount(2),
            ],
        );
        let n = Needs::of(&group);
        assert_eq!(n.prefer, vec!["front".to_string()]);
        assert_eq!(
            n.tags_to_apply(),
            vec![
                "moving_head".to_string(),
                "hero".to_string(),
                "left".to_string()
            ]
        );
    }

    #[test]
    fn candidates_must_do_everything_the_shows_ask() {
        let fixtures = vec![
            fixture("m1", "Viper", None, MOVER),
            fixture("w1", "Par", None, WASH),
        ];
        let fit = suggest(
            &needs(&["movers"], &[]),
            &wants(&[Want::Move, Want::Color]),
            &fixtures,
        );
        let s = fit.suggestion.unwrap();
        assert_eq!(s.fixtures, vec!["m1".to_string()]);
        assert_eq!(s.tags, vec!["movers".to_string()]);
        assert_eq!(s.reason.can, vec![Want::Move, Want::Color]);
        assert!(fit.others.is_empty());
    }

    #[test]
    fn an_unplaced_venue_clusters_by_type_alone() {
        let fixtures = vec![
            fixture("a", "Viper", None, MOVER),
            fixture("b", "Viper", None, MOVER),
            fixture("c", "Esprite", None, MOVER),
        ];
        let fit = suggest(&needs(&["m"], &[]), &wants(&[Want::Move]), &fixtures);
        let s = fit.suggestion.unwrap();
        assert_eq!(s.fixtures, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(s.reason.fixture_type, "Viper");
        assert_eq!(s.reason.place, None);
        assert_eq!(fit.others.len(), 1);
        assert_eq!(fit.others[0].fixture_type, "Esprite");
        assert_eq!(fit.others[0].fixtures, vec!["c".to_string()]);
    }

    #[test]
    fn a_placed_venue_clusters_by_height_and_depth_bands() {
        // y spans 0..9, so thirds split at 3 and 6.
        let fixtures = vec![
            fixture("up1", "Viper", Some([0.0, 9.0, 6.0]), MOVER),
            fixture("up2", "Viper", Some([1.0, 8.0, 6.0]), MOVER),
            fixture("up3", "Viper", Some([2.0, 7.0, 5.0]), MOVER),
            fixture("deck", "Viper", Some([0.0, 0.0, 0.0]), MOVER),
            fixture("low", "Viper", Some([0.0, 4.0, 2.0]), MOVER),
        ];
        let fit = suggest(&needs(&["m"], &[]), &wants(&[Want::Move]), &fixtures);
        let s = fit.suggestion.unwrap();
        assert_eq!(s.fixtures.len(), 3);
        assert_eq!(s.reason.count, 3);
        assert_eq!(s.reason.height, Some(Height::Truss));
        assert_eq!(s.reason.depth, Some(Depth::Upstage));
        assert_eq!(s.reason.place.as_deref(), Some("upstage truss"));
        let places: Vec<_> = fit.others.iter().map(|c| c.place.clone()).collect();
        assert_eq!(
            places,
            vec![
                Some("downstage deck".to_string()),
                Some("midstage low rig".to_string())
            ]
        );
    }

    #[test]
    fn height_band_edges() {
        assert_eq!(Height::of(0.49), Height::Deck);
        assert_eq!(Height::of(0.5), Height::Low);
        assert_eq!(Height::of(2.49), Height::Low);
        assert_eq!(Height::of(2.5), Height::Truss);
    }

    #[test]
    fn ties_break_by_type_then_place_and_are_stable() {
        let fixtures = vec![
            fixture("z1", "Zed", None, MOVER),
            fixture("a1", "Alpha", None, MOVER),
        ];
        let fit = suggest(&needs(&["m"], &[]), &wants(&[Want::Move]), &fixtures);
        assert_eq!(fit.suggestion.unwrap().reason.fixture_type, "Alpha");
        assert_eq!(fit.others[0].fixture_type, "Zed");
        let again = suggest(&needs(&["m"], &[]), &wants(&[Want::Move]), &fixtures);
        assert_eq!(
            again.suggestion.unwrap().fixtures,
            vec!["a1".to_string()],
            "same input, same suggestion"
        );
    }

    #[test]
    fn a_flat_venue_has_no_depth_band() {
        let fixtures = vec![
            fixture("a", "Viper", Some([0.0, 3.0, 6.0]), MOVER),
            fixture("b", "Viper", Some([1.0, 3.0, 6.0]), MOVER),
        ];
        let s = suggest(&needs(&["m"], &[]), &wants(&[Want::Move]), &fixtures)
            .suggestion
            .unwrap();
        assert_eq!(s.reason.depth, None);
        assert_eq!(s.reason.place.as_deref(), Some("truss"));
        assert_eq!(s.fixtures.len(), 2);
    }

    #[test]
    fn no_fitting_cluster_names_the_want_nothing_meets() {
        let fixtures = vec![fixture("w", "Par", None, WASH)];
        let fit = suggest(
            &needs(&["strobes"], &[]),
            &wants(&[Want::Color, Want::Strobe]),
            &fixtures,
        );
        assert!(fit.suggestion.is_none());
        assert_eq!(fit.unmet, vec![Want::Strobe]);
        assert!(!fit.unmet_together);
    }

    #[test]
    fn wants_met_only_separately_are_reported_together() {
        let fixtures = vec![
            fixture("mono", "Spot", None, &[FixtureCapabilities::PANNING]),
            fixture("par", "Par", None, WASH),
        ];
        let fit = suggest(
            &needs(&["m"], &[]),
            &wants(&[Want::Move, Want::Color]),
            &fixtures,
        );
        assert!(fit.suggestion.is_none());
        assert_eq!(fit.unmet, vec![Want::Move, Want::Color]);
        assert!(fit.unmet_together);
    }

    #[test]
    fn a_group_with_no_tag_constraints_has_nothing_to_suggest() {
        let fixtures = vec![fixture("a", "Par", None, WASH)];
        let fit = suggest(&Needs::default(), &wants(&[Want::Color]), &fixtures);
        assert_eq!(fit, Fit::default());
    }

    #[test]
    fn cells_want_needs_a_pixel_fixture() {
        let mut bar = fixture("bar", "Bar", None, WASH);
        bar.cells = true;
        let fixtures = vec![bar, fixture("par", "Par", None, WASH)];
        let s = suggest(
            &needs(&["bars"], &[]),
            &wants(&[Want::Cells, Want::Color]),
            &fixtures,
        )
        .suggestion
        .unwrap();
        assert_eq!(s.fixtures, vec!["bar".to_string()]);
    }

    fn named(channels: &[&str]) -> FitFixture {
        let map = channels
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i as u16 + 1))
            .collect();
        let info = FixtureInfo::new("f".into(), 1, 1, "T".into(), map, None);
        FitFixture::from_info(&info, &[])
    }

    #[test]
    fn capability_names_carry_what_else_the_fixture_has() {
        assert_eq!(
            named(&["red", "green", "blue", "white", "dimmer", "zoom"]).capability_names(),
            vec!["color", "dimmer", "white", "zoom"]
        );
        assert_eq!(named(&["pan", "tilt"]).capability_names(), vec!["pan_tilt"]);
    }

    #[test]
    fn a_wheel_is_colour_only_when_nothing_mixes() {
        assert_eq!(
            named(&["color1", "dimmer"]).capability_names(),
            vec!["dimmer", "color_wheel"]
        );
        // A mixing fixture with a wheel is a colour fixture: the wheel is extra.
        assert!(!named(&["red", "green", "blue", "color1"]).color_wheel);
        assert!(!named(&["color"]).color_wheel);
    }
}
