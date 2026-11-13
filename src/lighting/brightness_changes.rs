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

//! Where the one-brightness-model fix changes what a show looks like.
//!
//! The lighting engine used to treat brightness differently on fixtures with a
//! dimmer channel and on RGB-only ones. It now follows one rule on both: a
//! layer's dimming applies to that layer and the ones beneath it, never above;
//! a dimmer `replace` overrides the dimmers below it; colour with nothing
//! dimming it is full brightness; and a level with no colour beneath it is
//! white. Shows written against the old behaviour can look different.
//!
//! These are upgrade notes, not lint: they are noise once a show has been
//! reviewed, so [`lint_shows`](super::lint::lint_shows) does not run them.
//! `mtrack brightness-changes` does, against a project, before an upgrade.
//!
//! Four cases, each a [`Severity::Note`]:
//!
//! - `brightness_change_upper_colour`: colour on a layer above one that dims
//!   the same fixtures at the same time. The upper colour is no longer dimmed.
//! - `brightness_change_dimmer_replace`: a dimmer `replace` above a lower
//!   dimmer on RGB-only fixtures. It used to multiply; it now sets the level.
//! - `brightness_change_colour_runs_on`: on dimmer-channel fixtures, a colour
//!   still running after the intensity effects covering it have ended. The
//!   fixture used to hold the last dimmer byte; it now goes to full.
//! - `brightness_change_level_without_colour`: an intensity effect on colour
//!   fixtures with no colour beneath it. It now shows white.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use crate::lighting::effects::{
    BlendMode, EffectLayer, EffectType, FixtureInfo, DEFAULT_BLEND, DEFAULT_LAYER,
};
use crate::lighting::findings::{Severity, Warning};
use crate::lighting::lint::MIN_REPORTED_SPAN;
use crate::lighting::parser::{Cue, Effect, LightShow};

/// What a fixture can do, as far as brightness is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureTraits {
    pub name: String,
    /// Colour channels (RGB or CMY).
    pub rgb: bool,
    /// A dimmer channel.
    pub dimmer: bool,
    /// A strobe channel (without one, a strobe is an intensity effect).
    pub strobe: bool,
}

impl FixtureTraits {
    /// Reads the traits off a venue fixture.
    pub fn of(fixture: &FixtureInfo) -> FixtureTraits {
        use crate::lighting::effects::FixtureCapabilities as Cap;
        FixtureTraits {
            name: fixture.name.clone(),
            rgb: fixture.has_capability(Cap::RGB_COLOR),
            dimmer: fixture.has_dimmer(),
            strobe: fixture.has_capability(Cap::STROBING),
        }
    }
}

/// The venue as these checks need it: the fixtures each targeted group
/// resolves to. A group missing here (or resolving to nothing) is not
/// checked.
#[derive(Default)]
pub struct BrightnessContext {
    pub groups: HashMap<String, Vec<FixtureTraits>>,
}

/// An effect as it really plays: when, on which layer, and on which fixtures.
struct Item<'a> {
    show: &'a str,
    start: Duration,
    end: Duration,
    effect: &'a Effect,
    layer: EffectLayer,
    blend: BlendMode,
    fixtures: Vec<&'a FixtureTraits>,
}

impl Item<'_> {
    fn colour(&self) -> bool {
        sets_colour(&self.effect.effect_type)
    }

    /// Whether it is an intensity effect on `fixture`.
    fn intensity_on(&self, fixture: &FixtureTraits) -> bool {
        match &self.effect.effect_type {
            EffectType::Dimmer { .. } | EffectType::Pulse { .. } => true,
            // A chase that lifts (add, screen) emits colour; the others gate.
            EffectType::Chase { .. } => !matches!(self.blend, BlendMode::Add | BlendMode::Screen),
            // With a strobe channel the fixture's own hardware strobes.
            EffectType::Strobe { .. } => !fixture.strobe,
            EffectType::Static { parameters, .. } => parameters.contains_key("dimmer"),
            _ => false,
        }
    }

    /// Whether it is an intensity effect on `fixture` that can darken it.
    fn dims_on(&self, fixture: &FixtureTraits) -> bool {
        if !self.intensity_on(fixture) {
            return false;
        }
        match &self.effect.effect_type {
            EffectType::Dimmer {
                start_level,
                end_level,
                ..
            } => start_level.min(*end_level) < 1.0,
            EffectType::Static { parameters, .. } => {
                parameters.get("dimmer").is_some_and(|level| *level < 1.0)
            }
            _ => true,
        }
    }

    /// Whether it sets a level: a dimmer, or a static naming a `dimmer`.
    fn sets_level(&self) -> bool {
        match &self.effect.effect_type {
            EffectType::Dimmer { .. } => true,
            EffectType::Static { parameters, .. } => parameters.contains_key("dimmer"),
            _ => false,
        }
    }
}

/// Whether an effect sets the colour of the fixtures it runs on.
fn sets_colour(effect_type: &EffectType) -> bool {
    match effect_type {
        EffectType::ColorCycle { .. } | EffectType::Rainbow { .. } => true,
        EffectType::Static { parameters, .. } => ["red", "green", "blue"]
            .iter()
            .any(|channel| parameters.contains_key(*channel)),
        _ => false,
    }
}

/// The venue's fixtures, resolved for every group `shows` targets.
///
/// Empty when no venue is loaded or it does not register, which makes every
/// check skip rather than guess.
pub fn context_for(
    system: &mut crate::lighting::system::LightingSystem,
    shows: &[LightShow],
) -> BrightnessContext {
    let mut ctx = BrightnessContext::default();
    if system.get_current_venue().is_none() {
        return ctx;
    }
    let Ok(fixtures) = system.load_current_venue_fixtures() else {
        return ctx;
    };
    for name in crate::lighting::readiness::group_names(shows) {
        let members = system.resolve_logical_group_graceful(&name);
        let traits = fixtures
            .iter()
            .filter(|f| members.contains(&f.name))
            .map(FixtureTraits::of)
            .collect();
        ctx.groups.insert(name, traits);
    }
    ctx
}

/// Every song's findings, songs in list order, those with none left out.
pub fn project_findings(
    system: &mut crate::lighting::system::LightingSystem,
    songs: &crate::songs::Songs,
) -> Vec<(String, Vec<Warning>)> {
    let mut out = Vec::new();
    for (song, shows) in crate::lighting::readiness::songs_with_lighting(songs) {
        let ctx = context_for(system, &shows);
        let findings = brightness_changes(&shows, &ctx);
        if !findings.is_empty() {
            out.push((song.name().to_string(), findings));
        }
    }
    out
}

/// The places in `shows` where brightness will look different, against the
/// venue in `ctx`. Shows are taken together, as the lint takes them: the
/// shows of one song's files play as one timeline.
pub fn brightness_changes(shows: &[LightShow], ctx: &BrightnessContext) -> Vec<Warning> {
    let mut cues: Vec<&Cue> = Vec::new();
    let mut show_of: Vec<&str> = Vec::new();
    for show in shows {
        for cue in &show.cues {
            cues.push(cue);
            show_of.push(&show.name);
        }
    }
    let ends = crate::lighting::timeline::effect_ends(cues.iter().copied());

    let mut items: Vec<Item> = Vec::new();
    for end in &ends {
        let (cue, effect) = (cues[end.cue], &cues[end.cue].effects[end.effect]);
        let until = end.until();
        if until <= cue.time {
            continue;
        }
        let mut fixtures: Vec<&FixtureTraits> = Vec::new();
        for group in &effect.groups {
            for fixture in ctx.groups.get(group).into_iter().flatten() {
                if !fixtures.iter().any(|f| f.name == fixture.name) {
                    fixtures.push(fixture);
                }
            }
        }
        if fixtures.is_empty() {
            continue;
        }
        items.push(Item {
            show: show_of[end.cue],
            start: cue.time,
            end: until,
            effect,
            layer: effect.layer.unwrap_or(DEFAULT_LAYER),
            blend: effect.blend_mode.unwrap_or(DEFAULT_BLEND),
            fixtures,
        });
    }
    items.sort_by_key(|item| (item.start, item.end));

    let mut out = Vec::new();
    upper_colour_and_dimmer_replace(&items, &mut out);
    colour_runs_on(&items, &mut out);
    level_without_colour(&items, &mut out);
    out
}

fn overlap(a: &Item, b: &Item) -> Option<(Duration, Duration)> {
    let (start, end) = (a.start.max(b.start), a.end.min(b.end));
    (start + MIN_REPORTED_SPAN < end).then_some((start, end))
}

fn secs(t: Duration) -> String {
    format!("{:.3}s", t.as_secs_f64())
}

fn note(kind: &'static str, message: String) -> Warning {
    Warning {
        kind,
        message,
        severity: Severity::Note,
    }
}

/// The effect's groups, for naming it in a finding.
fn groups_of(item: &Item) -> String {
    item.effect
        .groups
        .iter()
        .map(|g| format!("`{g}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A few fixture names, then a count.
fn fixture_list(names: &BTreeSet<&str>) -> String {
    const SHOWN: usize = 3;
    let shown: Vec<&str> = names.iter().copied().take(SHOWN).collect();
    if names.len() > SHOWN {
        format!("{} and {} more", shown.join(", "), names.len() - SHOWN)
    } else {
        shown.join(", ")
    }
}

/// Cases 1 and 2: an effect on a layer above one that dims the same
/// fixtures at the same time.
fn upper_colour_and_dimmer_replace(items: &[Item], out: &mut Vec<Warning>) {
    // One finding per upper effect and lower layer.
    let mut reported: BTreeSet<(usize, EffectLayer)> = BTreeSet::new();
    for (ui, upper) in items.iter().enumerate() {
        let upper_colour = upper.colour() && upper.blend != BlendMode::Multiply;
        let upper_replace_level = upper.sets_level() && upper.blend == BlendMode::Replace;
        if !upper_colour && !upper_replace_level {
            continue;
        }
        for lower in items {
            if lower.layer >= upper.layer || reported.contains(&(ui, lower.layer)) {
                continue;
            }
            let Some((from, _)) = overlap(upper, lower) else {
                continue;
            };
            let shared = |only: &dyn Fn(&FixtureTraits) -> bool| -> BTreeSet<&str> {
                upper
                    .fixtures
                    .iter()
                    .filter(|f| only(f) && lower.dims_on(f))
                    .filter(|f| lower.fixtures.iter().any(|l| l.name == f.name))
                    .map(|f| f.name.as_str())
                    .collect()
            };
            if upper_colour {
                let fixtures = shared(&|_| true);
                if !fixtures.is_empty() {
                    reported.insert((ui, lower.layer));
                    out.push(note(
                        "brightness_change_upper_colour",
                        format!(
                            "show \"{}\" at {}: colour on {} in the {} layer runs over dimming \
                             in the {} layer ({}). It used to be dimmed on some fixtures and now \
                             never is, so it will look brighter. Put the level on the colour's \
                             layer (`dimmer:` beside the colour), or blend with `multiply`.",
                            upper.show,
                            secs(from),
                            groups_of(upper),
                            upper.layer.keyword(),
                            lower.layer.keyword(),
                            fixture_list(&fixtures),
                        ),
                    ));
                    continue;
                }
            }
            if upper_replace_level {
                // RGB-only fixtures: a dimmer channel already replaced.
                let fixtures = shared(&|f| f.rgb && !f.dimmer);
                if !fixtures.is_empty() {
                    reported.insert((ui, lower.layer));
                    out.push(note(
                        "brightness_change_dimmer_replace",
                        format!(
                            "show \"{}\" at {}: a dimmer `replace` on {} in the {} layer runs \
                             over a dimmer in the {} layer ({}). On these RGB-only fixtures it \
                             used to multiply with the lower level; it now sets the level. Use \
                             `blend: multiply` to keep the old result.",
                            upper.show,
                            secs(from),
                            groups_of(upper),
                            upper.layer.keyword(),
                            lower.layer.keyword(),
                            fixture_list(&fixtures),
                        ),
                    ));
                }
            }
        }
    }
}

/// Merges intervals into disjoint ones, in order.
fn merged(mut spans: Vec<(Duration, Duration)>) -> Vec<(Duration, Duration)> {
    spans.sort();
    let mut out: Vec<(Duration, Duration)> = Vec::new();
    for (start, end) in spans {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// Case 3: on a dimmer-channel fixture, a colour that runs on after the
/// intensity effects covering it end.
fn colour_runs_on(items: &[Item], out: &mut Vec<Warning>) {
    for colour in items.iter().filter(|i| i.colour()) {
        // Where the cover ends -> the fixtures that go to full from there.
        let mut gaps: BTreeMap<Duration, BTreeSet<&str>> = BTreeMap::new();
        for fixture in colour.fixtures.iter().filter(|f| f.dimmer && f.rgb) {
            let covers: Vec<(Duration, Duration)> = items
                .iter()
                .filter(|i| i.intensity_on(fixture))
                .filter(|i| i.fixtures.iter().any(|f| f.name == fixture.name))
                .filter_map(|i| overlap(colour, i))
                .collect();
            let covers = merged(covers);
            // Never covered is not "after one has ended".
            let Some(&(_, first_end)) = covers.first() else {
                continue;
            };
            let next_start = covers.get(1).map_or(colour.end, |c| c.0);
            if first_end + MIN_REPORTED_SPAN < next_start {
                gaps.entry(first_end).or_default().insert(&fixture.name);
            }
        }
        for (at, fixtures) in gaps {
            out.push(note(
                "brightness_change_colour_runs_on",
                format!(
                    "show \"{}\" at {}: colour on {} keeps running after the last dimmer \
                     covering it ends ({}). The fixture used to hold its last dimmer level; it \
                     now goes to full. End the colour with the dimmer, or add a dimmer to cover \
                     it.",
                    colour.show,
                    secs(at),
                    groups_of(colour),
                    fixture_list(&fixtures),
                ),
            ));
        }
    }
}

/// Case 4: an intensity effect on colour fixtures with no colour effect
/// beneath it.
fn level_without_colour(items: &[Item], out: &mut Vec<Warning>) {
    for level in items {
        // The first moment with no colour -> the fixtures it happens on.
        let mut gaps: BTreeMap<Duration, BTreeSet<&str>> = BTreeMap::new();
        for fixture in level.fixtures.iter().filter(|f| f.rgb) {
            if !level.intensity_on(fixture) {
                continue;
            }
            let beneath: Vec<(Duration, Duration)> = items
                .iter()
                .filter(|i| i.colour() && i.layer <= level.layer)
                .filter(|i| i.fixtures.iter().any(|f| f.name == fixture.name))
                .filter_map(|i| overlap(level, i))
                .collect();
            let mut cursor = level.start;
            for (start, end) in merged(beneath) {
                if cursor + MIN_REPORTED_SPAN < start {
                    break;
                }
                cursor = cursor.max(end);
            }
            if cursor + MIN_REPORTED_SPAN < level.end {
                gaps.entry(cursor).or_default().insert(&fixture.name);
            }
        }
        for (at, fixtures) in gaps {
            out.push(note(
                "brightness_change_level_without_colour",
                format!(
                    "show \"{}\" at {}: a {} on {} has no colour beneath it ({}). It now shows \
                     white at its level; it used to show the last colour sent, or nothing. Add \
                     a colour on the same or a lower layer.",
                    level.show,
                    secs(at),
                    level.effect.effect_type.name().to_lowercase(),
                    groups_of(level),
                    fixture_list(&fixtures),
                ),
            ));
        }
    }
}
