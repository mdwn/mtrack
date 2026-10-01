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

//! The venue-side facts lint needs, gathered in one place.
//!
//! MCP's `validate_lighting` and the web UI's readiness hub both ask "what does
//! this show do against the rig that is loaded?". They must never disagree, so
//! both build their [`LintContext`] here rather than each resolving groups and
//! universes on its own.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::dmx::engine::Engine;
use crate::lighting::lint::{
    lint_shows, universe_coverage, GroupCapabilities, LintContext, Warning,
};
use crate::lighting::parser::LightShow;
use crate::lighting::system::LightingSystem;
use crate::songs::Song;

/// Every distinct group name a set of shows targets, sorted and deduplicated.
pub fn group_names(shows: &[LightShow]) -> Vec<String> {
    let mut names: Vec<String> = shows
        .iter()
        .flat_map(|show| show.cues.iter())
        .flat_map(|cue| cue.effects.iter())
        .flat_map(|effect| effect.groups.iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Every song with lighting and the shows its files hold (sorted by name), in
/// the order the song list shows them.
///
/// These were parsed at song load with the song's own tempo map, the same parse
/// the player plays, so nothing here re-reads a file.
pub fn songs_with_lighting(songs: &crate::songs::Songs) -> Vec<(Arc<Song>, Vec<LightShow>)> {
    let mut out = Vec::new();
    for song in songs.sorted_list() {
        if song.dsl_lighting_shows().is_empty() {
            continue;
        }
        let mut shows: Vec<LightShow> = song
            .dsl_lighting_shows()
            .iter()
            .flat_map(|dsl| dsl.shows().values().cloned())
            .collect();
        shows.sort_by(|a, b| a.name.cmp(&b.name));
        out.push((song, shows));
    }
    out
}

/// What the loaded venue says about the groups a set of shows targets.
///
/// Everything here is empty when no venue is loaded, which makes the checks
/// that depend on it skip rather than report every group as empty.
#[derive(Default)]
pub struct VenueFacts {
    /// How many fixtures each group resolves to in the current venue.
    pub group_fixture_counts: HashMap<String, usize>,
    /// What each group's fixtures can do.
    pub group_capabilities: HashMap<String, GroupCapabilities>,
    /// The current venue's focus points; `None` without a venue.
    pub focus_points: Option<HashSet<String>>,
    /// Fixtures patched on a universe with no configured output.
    pub universe_warnings: Vec<Warning>,
    /// Fixtures of the venue patched over part of each other's addresses
    /// (kind `patch-overlap`, each pair of gangs once — fixtures at an
    /// identical span are ganged, not overlapping) and fixtures that run
    /// past address 512 (kind `patch-overrun`). A fixture whose type or mode
    /// did not load has no known footprint and is not checked.
    pub patch_warnings: Vec<Warning>,
    /// Why the current venue will not register — the first fixture that
    /// cannot be driven. The whole venue fails then (no fixture lights), so
    /// the group counts above, read from what did resolve, are empty.
    pub venue_error: Option<VenueError>,
}

/// A venue that does not register: the fixture that fails it and why.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct VenueError {
    pub venue: String,
    pub fixture: String,
    pub reason: String,
}

impl VenueFacts {
    /// Resolves `names` against the current venue of `system`.
    ///
    /// `configured_universes` is the running profile's `dmx.universes`; `None`
    /// skips the universe check.
    pub fn collect(
        system: &mut LightingSystem,
        names: &[String],
        configured_universes: Option<&[u16]>,
    ) -> VenueFacts {
        let mut facts = VenueFacts {
            focus_points: system
                .get_current_venue()
                .map(|venue| venue.focus_points().keys().cloned().collect()),
            ..Default::default()
        };
        // Only when a venue is actually loaded. Without one every group
        // resolves to nothing, and reporting them all as empty would be noise
        // rather than a finding.
        if system.get_current_venue().is_none() {
            return facts;
        }
        // A venue that does not register lights nothing; say why rather than
        // letting it read as an empty rig.
        let fixtures = match system.get_current_venue_fixtures() {
            Ok(fixtures) => fixtures,
            Err(e) => {
                let venue = system.current_venue().unwrap_or_default().to_string();
                facts.venue_error = Some(match system.venue_problem(&venue) {
                    Some((fixture, reason)) => VenueError {
                        venue,
                        fixture,
                        reason,
                    },
                    None => VenueError {
                        venue,
                        fixture: String::new(),
                        reason: e.to_string(),
                    },
                });
                Vec::new()
            }
        };
        for name in names {
            let members = system.resolve_logical_group_graceful(name);
            facts
                .group_fixture_counts
                .insert(name.clone(), members.len());
            facts.group_capabilities.insert(
                name.clone(),
                GroupCapabilities::from_fixtures(
                    fixtures.iter().filter(|f| members.contains(&f.name)),
                ),
            );
        }
        if let Some(configured) = configured_universes {
            facts.universe_warnings = universe_coverage(&fixtures, configured);
        }
        facts.patch_warnings = system
            .current_venue_overlaps()
            .into_iter()
            .map(|overlap| Warning {
                kind: "patch-overlap",
                message: overlap.to_string(),
            })
            .chain(
                system
                    .current_venue_overruns()
                    .into_iter()
                    .map(|overrun| Warning {
                        kind: "patch-overrun",
                        message: overrun.to_string(),
                    }),
            )
            .collect();
        facts
    }

    /// [`VenueFacts::collect`] for the engine's lighting system, on the
    /// blocking pool.
    ///
    /// The lighting-system mutex is shared with the effects loop thread, so
    /// taking it belongs off the async worker. An engine with no lighting
    /// system (or no engine at all) yields empty facts.
    pub async fn from_engine(
        dmx: Option<Arc<Engine>>,
        names: Vec<String>,
    ) -> Result<VenueFacts, tokio::task::JoinError> {
        let configured = dmx.as_ref().map(|d| d.configured_universes());
        match dmx.and_then(|dmx| dmx.broadcast_handles().lighting_system) {
            Some(system) => {
                tokio::task::spawn_blocking(move || {
                    let mut guard = system.lock();
                    VenueFacts::collect(&mut guard, &names, configured.as_deref())
                })
                .await
            }
            None => Ok(VenueFacts::default()),
        }
    }

    /// The lint context for shows accompanying `song` (or a bare source when
    /// `None`). Borrows the song, so it must not be held across an await.
    pub fn context<'a>(&self, song: Option<&'a Song>) -> LintContext<'a> {
        LintContext {
            song_duration: song.map(|s| s.duration()),
            beat_grid: song.and_then(|s| s.beat_grid()),
            group_fixture_counts: self.group_fixture_counts.clone(),
            group_capabilities: self.group_capabilities.clone(),
            focus_points: self.focus_points.clone(),
        }
    }

    /// Runs every lint check over `shows`, plus the venue-level findings:
    /// universe coverage and patch overlaps.
    pub fn lint(&self, shows: &[LightShow], song: Option<&Song>) -> Vec<Warning> {
        let mut warnings = lint_shows(shows, &self.context(song));
        warnings.extend(self.universe_warnings.iter().cloned());
        warnings.extend(self.patch_warnings.iter().cloned());
        warnings
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::Lighting;

    /// A system with a two-fixture venue: one tagged `wash` on universe 1,
    /// one untagged on universe 4.
    fn system(dir: &std::path::Path) -> LightingSystem {
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::create_dir_all(dir.join("venues")).unwrap();
        std::fs::write(
            dir.join("types/par.light"),
            "fixture_type \"Par\" {\n  channels: 2\n  channel_map: { \"dimmer\": 1, \"red\": 2 }\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("venues/v.light"),
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  fixture \"B\" Par @ 4:1\n}\n",
        )
        .unwrap();
        let yaml = "directories:\n  fixture_types: types\n  venues: venues\ncurrent_venue: v\n\
                    groups:\n  washes:\n    name: washes\n    constraints:\n      - AllOf: [\"wash\"]\n";
        let config: Lighting = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let mut system = LightingSystem::new();
        system.load(&config, dir).unwrap();
        system
    }

    #[test]
    fn no_venue_means_no_facts() {
        let mut system = LightingSystem::new();
        let facts = VenueFacts::collect(&mut system, &["washes".to_string()], Some(&[1]));
        assert!(facts.group_fixture_counts.is_empty());
        assert!(facts.focus_points.is_none());
        assert!(facts.universe_warnings.is_empty());
    }

    #[test]
    fn groups_resolve_against_the_current_venue() {
        let dir = tempfile::tempdir().unwrap();
        let mut system = system(dir.path());
        let names = vec!["washes".to_string(), "nobody".to_string()];
        let facts = VenueFacts::collect(&mut system, &names, Some(&[1]));
        assert_eq!(facts.group_fixture_counts["washes"], 1);
        assert_eq!(facts.group_fixture_counts["nobody"], 0);
        assert_eq!(facts.group_capabilities["washes"].fixtures, 1);
        assert_eq!(facts.focus_points, Some(HashSet::new()));
        // Universe 4 has no configured output.
        assert_eq!(facts.universe_warnings.len(), 1);
        assert_eq!(facts.universe_warnings[0].kind, "unconfigured-universe");
    }

    #[test]
    fn the_universe_check_is_skipped_without_a_configured_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut system = system(dir.path());
        let facts = VenueFacts::collect(&mut system, &[], None);
        assert!(facts.universe_warnings.is_empty());
    }

    #[test]
    fn fixtures_patched_over_each_other_are_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let mut system = system(dir.path());
        let facts = VenueFacts::collect(&mut system, &[], Some(&[1, 4]));
        assert!(facts.patch_warnings.is_empty(), "A and B do not overlap");

        // C starts on A's second address.
        std::fs::write(
            dir.path().join("venues/v.light"),
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  fixture \"B\" Par @ 4:1\n  \
             fixture \"C\" Par @ 1:2\n}\n",
        )
        .unwrap();
        system.reload_venues().unwrap();
        let facts = VenueFacts::collect(&mut system, &[], Some(&[1, 4]));
        assert_eq!(facts.patch_warnings.len(), 1, "{:?}", facts.patch_warnings);
        assert_eq!(facts.patch_warnings[0].kind, "patch-overlap");
        let message = &facts.patch_warnings[0].message;
        assert!(message.contains("\"A\" and \"C\""), "{message}");
        assert!(message.contains("universe 1 at address 2"), "{message}");
        let kinds: Vec<&str> = facts.lint(&[], None).iter().map(|w| w.kind).collect();
        assert_eq!(kinds, ["patch-overlap"]);
    }

    #[test]
    fn ganged_fixtures_are_quiet_and_an_overrun_is_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let mut system = system(dir.path());
        // Three pars ganged at 1:1: deliberate, not an overlap. D runs
        // from 512 past the end of the universe.
        std::fs::write(
            dir.path().join("venues/v.light"),
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"A2\" Par @ 1:1\n  \
             fixture \"A3\" Par @ 1:1\n  fixture \"D\" Par @ 1:512\n}\n",
        )
        .unwrap();
        system.reload_venues().unwrap();
        let facts = VenueFacts::collect(&mut system, &[], Some(&[1]));
        assert_eq!(facts.patch_warnings.len(), 1, "{:?}", facts.patch_warnings);
        assert_eq!(facts.patch_warnings[0].kind, "patch-overrun");
        assert!(facts.patch_warnings[0].message.contains("\"D\""));
        assert!(facts.venue_error.is_none());
    }

    #[test]
    fn a_venue_that_will_not_register_is_an_error_not_an_empty_rig() {
        let dir = tempfile::tempdir().unwrap();
        let mut system = system(dir.path());
        std::fs::write(
            dir.path().join("venues/v.venue"),
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  \
             fixture \"B\" Par mode \"8: RGBS\" @ 1:10\n}\n",
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("venues/v.light")).unwrap();
        system.reload_venues().unwrap();
        let facts = VenueFacts::collect(&mut system, &["washes".to_string()], Some(&[1]));
        let error = facts.venue_error.expect("the venue does not register");
        assert_eq!(error.venue, "v");
        assert_eq!(error.fixture, "B");
        assert!(
            error.reason.contains("is not GDTF-sourced"),
            "{}",
            error.reason
        );
    }

    #[test]
    fn lint_reports_an_empty_group_and_the_universe_finding() {
        let dir = tempfile::tempdir().unwrap();
        let mut system = system(dir.path());
        let shows: Vec<LightShow> = crate::lighting::parser::parse_light_shows_with_tempo(
            "show \"S\" {\n    @00:00.000\n    nobody: static color: \"red\", duration: 2s\n}\n",
            None,
        )
        .unwrap()
        .into_values()
        .collect();
        let names = group_names(&shows);
        assert_eq!(names, vec!["nobody".to_string()]);
        let facts = VenueFacts::collect(&mut system, &names, Some(&[1]));
        let kinds: Vec<&str> = facts.lint(&shows, None).iter().map(|w| w.kind).collect();
        assert!(kinds.contains(&"empty-group"), "{kinds:?}");
        assert!(kinds.contains(&"unconfigured-universe"), "{kinds:?}");
    }
}
