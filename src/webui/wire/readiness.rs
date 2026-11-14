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

//! `GetLightingReadiness`: the facts behind the Lighting overview's
//! checks, and the checks' verdicts on them ([`Readiness::verdicts`]). The
//! page words them and links each finding to where it is fixed.

use serde::Serialize;

use crate::lighting::findings::{LibraryFinding, Severity, VenueError, Warning};

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Readiness {
    /// Whether the player has a DMX engine.
    pub dmx: bool,
    /// The current venue; `null` without one.
    pub venue: Option<ReadinessVenue>,
    /// The current venue does not load: one fixture that cannot be driven
    /// fails the whole venue, and then no fixture lights.
    pub venue_error: Option<VenueError>,
    /// Fixtures patched over part of each other's addresses
    /// (`patch-overlap`) or past address 512 (`patch-overrun`).
    pub patch_warnings: Vec<Warning>,
    pub fixture_types: ReadinessTypes,
    /// The groups the shows target, with what each finds.
    pub groups: Vec<ReadinessGroup>,
    /// Every song with lighting, its files and what lint says of them, by
    /// song.
    pub shows: Vec<ReadinessShow>,
    pub output: ReadinessOutput,
    /// The five checks, in the order a show needs them.
    pub checks: Vec<Check>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessVenue {
    pub name: String,
    pub fixtures: usize,
    /// How many of them the venue places on the stage.
    pub placed: usize,
    pub focus_points: Vec<String>,
}

/// The fixture types the venue uses, those it cannot, and the library's
/// own findings.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessTypes {
    pub in_use: Vec<String>,
    pub unresolved: Vec<UnresolvedFixture>,
    pub library: Vec<LibraryFinding>,
}

/// A fixture whose type or mode does not resolve, and why.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UnresolvedFixture {
    pub fixture: String,
    #[serde(rename = "type")]
    pub fixture_type: String,
    pub reason: String,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessGroup {
    pub name: String,
    /// How many fixtures it finds in the current venue.
    pub fixtures: usize,
    pub songs: Vec<String>,
}

/// A song's lighting.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessShow {
    pub song: String,
    pub files: Vec<String>,
    /// Why the song's show does not load (the song does not, then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub warnings: Vec<Warning>,
}

/// The DMX universes the venue uses, those with no output, and what olad
/// says of them.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReadinessOutput {
    pub universes: Vec<u16>,
    pub unconfigured: Vec<u16>,
    /// `null` when nothing was asked.
    pub olad: Option<OladPatch>,
}

/// Whether olad answered, and the universes it has no output patched for.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OladPatch {
    pub reachable: bool,
    pub unpatched: Vec<u16>,
}

/// One of the overview's checks.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckKey {
    Fixtures,
    Venue,
    Groups,
    Shows,
    Output,
}

/// A check's verdict: its worst finding's severity, or `ready`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Ready,
    Attention,
    Blocked,
    Unknown,
}

/// A check, its verdict, what its card says and what it found.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Check {
    pub key: CheckKey,
    pub state: CheckState,
    /// The card's one-line summary, as parts shown side by side: a check
    /// that mixes severities says how many of each.
    pub summary: Vec<Summary>,
    pub findings: Vec<Finding>,
}

/// A part of a check's summary (the page's `lighting.hub.summary.<kind>`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Summary {
    Blocked {
        count: usize,
    },
    ShowsBlocked {
        count: usize,
    },
    Attention {
        count: usize,
    },
    Unknown,
    NoDmx,
    NeedsVenue,
    ShowsNone,
    ShowsReady {
        count: usize,
    },
    TypesReady {
        count: usize,
    },
    VenueAllPlaced {
        venue: String,
        fixtures: usize,
        unplaced: usize,
    },
    VenueSomeUnplaced {
        venue: String,
        fixtures: usize,
        unplaced: usize,
    },
    GroupsNone,
    GroupsReady {
        count: usize,
    },
    OutputReady {
        count: usize,
    },
}

/// Something a check found, and where it is fixed.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub what: FindingWhat,
    pub fix: Fix,
    /// A song, when the finding is listed under one.
    pub song: Option<String>,
}

/// What a finding is (the page's `lighting.hub.finding.<kind>`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FindingWhat {
    ShowError {
        song: String,
        detail: String,
    },
    Lint {
        detail: String,
    },
    NoDmx,
    TypeUnresolved {
        fixture: String,
        #[serde(rename = "type")]
        fixture_type: String,
        reason: String,
    },
    NoVenue,
    VenueFailed {
        venue: String,
        fixture: String,
        reason: String,
    },
    EmptyVenue {
        venue: String,
    },
    Patch {
        detail: String,
    },
    GroupEmpty {
        group: String,
        venue: String,
        songs: String,
    },
    UniverseUnconfigured {
        universe: u16,
    },
    OladUnreachable,
    UniverseUnpatched {
        universe: u16,
    },
}

/// Where a finding is fixed (the page's `lighting.hub.fix.<kind>`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Fix {
    /// The song's lighting.
    Song {
        song: String,
    },
    /// The Fit shows page, with a group selected when there is one.
    Fit {
        group: Option<String>,
    },
    Fixtures,
    Venues,
    /// The running profile's DMX settings.
    Config,
    /// The groups page, where a venue is chosen.
    ChooseVenue,
}

impl Finding {
    fn new(severity: Severity, what: FindingWhat, fix: Fix) -> Finding {
        Finding {
            severity,
            what,
            fix,
            song: None,
        }
    }
}

impl Check {
    /// The check of `findings`: blocked by any blocking one, else needing
    /// attention for any that does, else unknown for any unknown, else
    /// `ready` (which a note leaves alone). Its summary counts the blocking
    /// (in `blocked`'s words) and the ones needing attention, or says the
    /// check is unknown, or `ready`.
    fn of(
        key: CheckKey,
        findings: Vec<Finding>,
        ready: Summary,
        blocked: fn(usize) -> Summary,
    ) -> Check {
        let count = |severity| findings.iter().filter(|f| f.severity == severity).count();
        let (blocking, attention, unknown) = (
            count(Severity::Blocked),
            count(Severity::Attention),
            count(Severity::Unknown),
        );
        let state = if blocking > 0 {
            CheckState::Blocked
        } else if attention > 0 {
            CheckState::Attention
        } else if unknown > 0 {
            CheckState::Unknown
        } else {
            CheckState::Ready
        };
        let mut summary = Vec::new();
        if blocking > 0 {
            summary.push(blocked(blocking));
        }
        if attention > 0 {
            summary.push(Summary::Attention { count: attention });
        }
        if summary.is_empty() {
            summary.push(if unknown > 0 { Summary::Unknown } else { ready });
        }
        Check {
            key,
            state,
            summary,
            findings,
        }
    }

    /// A check that cannot be evaluated, and why.
    fn cannot(key: CheckKey, why: Summary) -> Check {
        Check {
            key,
            state: CheckState::Unknown,
            summary: vec![why],
            findings: Vec::new(),
        }
    }
}

impl Readiness {
    /// The five checks of these facts, in the order a show needs them.
    pub fn verdicts(&self) -> Vec<Check> {
        let blocked = |count| Summary::Blocked { count };
        let shows = self.shows_check();
        if !self.dmx {
            // No DMX on this profile: no lighting system exists to ask
            // about, so the venue-shaped checks cannot run and the output
            // is known to be missing. The shows still can be.
            let no_dmx = Finding::new(Severity::Blocked, FindingWhat::NoDmx, Fix::Config);
            return vec![
                Check::cannot(CheckKey::Fixtures, Summary::NoDmx),
                Check::cannot(CheckKey::Venue, Summary::NoDmx),
                Check::cannot(CheckKey::Groups, Summary::NoDmx),
                shows,
                Check::of(CheckKey::Output, vec![no_dmx], Summary::NoDmx, blocked),
            ];
        }
        let Some(venue) = &self.venue else {
            let no_venue = Finding::new(Severity::Blocked, FindingWhat::NoVenue, Fix::ChooseVenue);
            return vec![
                Check::cannot(CheckKey::Fixtures, Summary::NeedsVenue),
                Check::of(
                    CheckKey::Venue,
                    vec![no_venue],
                    Summary::NeedsVenue,
                    blocked,
                ),
                Check::cannot(CheckKey::Groups, Summary::NeedsVenue),
                shows,
                Check::cannot(CheckKey::Output, Summary::NeedsVenue),
            ];
        };

        let fixtures = self
            .fixture_types
            .unresolved
            .iter()
            .map(|u| {
                let what = FindingWhat::TypeUnresolved {
                    fixture: u.fixture.clone(),
                    fixture_type: u.fixture_type.clone(),
                    reason: u.reason.clone(),
                };
                Finding::new(Severity::Blocked, what, Fix::Fixtures)
            })
            .collect();
        let fixtures = Check::of(
            CheckKey::Fixtures,
            fixtures,
            Summary::TypesReady {
                count: self.fixture_types.in_use.len(),
            },
            blocked,
        );

        let mut found = Vec::new();
        if let Some(e) = &self.venue_error {
            let what = FindingWhat::VenueFailed {
                venue: e.venue.clone(),
                fixture: e.fixture.clone(),
                reason: e.reason.clone(),
            };
            found.push(Finding::new(Severity::Blocked, what, Fix::Venues));
        } else if venue.fixtures == 0 {
            let what = FindingWhat::EmptyVenue {
                venue: venue.name.clone(),
            };
            found.push(Finding::new(Severity::Attention, what, Fix::Venues));
        }
        for w in &self.patch_warnings {
            let what = FindingWhat::Patch {
                detail: w.message.clone(),
            };
            found.push(Finding::new(w.severity, what, Fix::Venues));
        }
        let (name, fixtures_in, unplaced) = (
            venue.name.clone(),
            venue.fixtures,
            venue.fixtures - venue.placed,
        );
        let placed = if unplaced == 0 {
            Summary::VenueAllPlaced {
                venue: name,
                fixtures: fixtures_in,
                unplaced,
            }
        } else {
            Summary::VenueSomeUnplaced {
                venue: name,
                fixtures: fixtures_in,
                unplaced,
            }
        };
        let venue_check = Check::of(CheckKey::Venue, found, placed, blocked);

        let empty = self
            .groups
            .iter()
            .filter(|g| g.fixtures == 0)
            .map(|g| {
                let what = FindingWhat::GroupEmpty {
                    group: g.name.clone(),
                    venue: venue.name.clone(),
                    songs: g.songs.join(", "),
                };
                let fix = Fix::Fit {
                    group: Some(g.name.clone()),
                };
                Finding::new(Severity::Attention, what, fix)
            })
            .collect();
        let groups_ready = match self.groups.len() {
            0 => Summary::GroupsNone,
            count => Summary::GroupsReady { count },
        };
        let groups = Check::of(CheckKey::Groups, empty, groups_ready, blocked);

        let fit = || Fix::Fit { group: None };
        let mut found: Vec<Finding> = self
            .output
            .unconfigured
            .iter()
            .map(|&universe| {
                let what = FindingWhat::UniverseUnconfigured { universe };
                Finding::new(Severity::Blocked, what, fit())
            })
            .collect();
        match &self.output.olad {
            Some(olad) if !olad.reachable => found.push(Finding::new(
                Severity::Unknown,
                FindingWhat::OladUnreachable,
                Fix::Config,
            )),
            Some(olad) => found.extend(olad.unpatched.iter().map(|&universe| {
                let what = FindingWhat::UniverseUnpatched { universe };
                Finding::new(Severity::Blocked, what, fit())
            })),
            None => {}
        }
        let output = Check::of(
            CheckKey::Output,
            found,
            Summary::OutputReady {
                count: self.output.universes.len(),
            },
            blocked,
        );

        vec![fixtures, venue_check, groups, shows, output]
    }

    /// The Shows check. It needs no venue or output: a show that does not
    /// load is broken on a laptop with no DMX too, which is where shows are
    /// written.
    fn shows_check(&self) -> Check {
        let mut found = Vec::new();
        for show in &self.shows {
            let song = Some(show.song.clone());
            let in_song = || Fix::Song {
                song: show.song.clone(),
            };
            if let Some(error) = &show.error {
                let what = FindingWhat::ShowError {
                    song: show.song.clone(),
                    detail: error.clone(),
                };
                found.push(Finding {
                    song: song.clone(),
                    ..Finding::new(Severity::Blocked, what, in_song())
                });
            }
            // An empty group is the Groups check's, which already says it.
            for w in show.warnings.iter().filter(|w| w.kind != "empty-group") {
                // A focus point the venue does not bind is fitted on the
                // Fit shows page, where it is placed on the plan; every
                // other finding is fixed in the song.
                let fix = if w.kind == "unbound-focus-point" {
                    Fix::Fit { group: None }
                } else {
                    in_song()
                };
                let what = FindingWhat::Lint {
                    detail: w.message.clone(),
                };
                found.push(Finding {
                    song: song.clone(),
                    ..Finding::new(w.severity, what, fix)
                });
            }
        }
        let ready = match self.shows.len() {
            0 => Summary::ShowsNone,
            count => Summary::ShowsReady { count },
        };
        Check::of(CheckKey::Shows, found, ready, |count| {
            Summary::ShowsBlocked { count }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rig where everything lines up.
    fn ready() -> Readiness {
        Readiness {
            dmx: true,
            venue: Some(ReadinessVenue {
                name: "test-venue".to_string(),
                fixtures: 8,
                placed: 8,
                focus_points: Vec::new(),
            }),
            venue_error: None,
            patch_warnings: Vec::new(),
            fixture_types: ReadinessTypes {
                in_use: vec!["brick".to_string()],
                unresolved: Vec::new(),
                library: Vec::new(),
            },
            groups: vec![ReadinessGroup {
                name: "front_wash".to_string(),
                fixtures: 4,
                songs: vec!["Alpha".to_string()],
            }],
            shows: vec![ReadinessShow {
                song: "Alpha".to_string(),
                files: vec!["show.light".to_string()],
                error: None,
                warnings: Vec::new(),
            }],
            output: ReadinessOutput {
                universes: vec![1],
                unconfigured: Vec::new(),
                olad: None,
            },
            checks: Vec::new(),
        }
    }

    fn states(r: &Readiness) -> Vec<CheckState> {
        r.verdicts().iter().map(|c| c.state).collect()
    }

    fn state(r: &Readiness, key: CheckKey) -> CheckState {
        r.verdicts().iter().find(|c| c.key == key).unwrap().state
    }

    fn warning(kind: &'static str, severity: Severity) -> Warning {
        Warning {
            kind,
            message: kind.to_string(),
            severity,
        }
    }

    fn broken_show(song: &str) -> ReadinessShow {
        ReadinessShow {
            song: song.to_string(),
            files: Vec::new(),
            error: Some("Effect 'static' requires a 'duration'".to_string()),
            warnings: Vec::new(),
        }
    }

    use CheckState::*;

    #[test]
    fn a_rig_where_everything_lines_up_is_ready_in_all_five() {
        let checks = ready().verdicts();
        let keys: Vec<CheckKey> = checks.iter().map(|c| c.key).collect();
        use CheckKey::*;
        assert_eq!(keys, vec![Fixtures, Venue, Groups, Shows, Output]);
        assert!(checks.iter().all(|c| c.state == Ready));
        assert_eq!(
            checks[1].summary,
            vec![Summary::VenueAllPlaced {
                venue: "test-venue".to_string(),
                fixtures: 8,
                unplaced: 0
            }]
        );
    }

    #[test]
    fn a_venue_that_does_not_load_blocks_the_venue() {
        let mut r = ready();
        r.venue_error = Some(VenueError {
            venue: "test-venue".to_string(),
            fixture: "Brick3".to_string(),
            reason: "did not load".to_string(),
        });
        assert_eq!(state(&r, CheckKey::Venue), Blocked);
    }

    #[test]
    fn patch_warnings_need_attention_on_the_venue() {
        let mut r = ready();
        r.patch_warnings = vec![warning("patch-overlap", Severity::Attention)];
        assert_eq!(state(&r, CheckKey::Venue), Attention);
    }

    #[test]
    fn a_fixture_type_that_did_not_load_blocks() {
        let mut r = ready();
        r.fixture_types.unresolved = vec![UnresolvedFixture {
            fixture: "Brick9".to_string(),
            fixture_type: "Nope".to_string(),
            reason: "no such type".to_string(),
        }];
        let checks = r.verdicts();
        assert_eq!(checks[0].state, Blocked);
        assert_eq!(checks[0].findings[0].fix, Fix::Fixtures);
    }

    #[test]
    fn no_venue_blocks_the_venue_and_leaves_what_depends_on_it_unknown() {
        let mut r = ready();
        r.venue = None;
        r.groups.clear();
        r.shows.clear();
        assert_eq!(states(&r), vec![Unknown, Blocked, Unknown, Ready, Unknown]);
        assert_eq!(r.verdicts()[1].findings[0].fix, Fix::ChooseVenue);
    }

    #[test]
    fn a_group_that_finds_no_fixtures_needs_attention() {
        let mut r = ready();
        r.groups[0].fixtures = 0;
        let checks = r.verdicts();
        assert_eq!(checks[2].state, Attention);
        assert_eq!(
            checks[2].findings[0].fix,
            Fix::Fit {
                group: Some("front_wash".to_string())
            }
        );
    }

    #[test]
    fn advisory_lint_is_a_note_and_a_gap_needs_attention() {
        let mut r = ready();
        r.shows[0].warnings = vec![warning("past-end-of-song", Severity::Note)];
        let checks = r.verdicts();
        assert_eq!(checks[3].state, Ready);
        assert_eq!(checks[3].findings[0].severity, Severity::Note);
        r.shows[0].warnings = vec![warning("capability-gap", Severity::Attention)];
        assert_eq!(state(&r, CheckKey::Shows), Attention);
        r.shows[0].warnings = vec![warning("empty-group", Severity::Note)];
        assert!(r.verdicts()[3].findings.is_empty());
    }

    #[test]
    fn output_blocks_on_a_universe_without_output_and_is_unknown_without_olad() {
        let mut r = ready();
        r.output.unconfigured = vec![4];
        assert_eq!(state(&r, CheckKey::Output), Blocked);
        let mut r = ready();
        r.output.olad = Some(OladPatch {
            reachable: true,
            unpatched: vec![1],
        });
        assert_eq!(state(&r, CheckKey::Output), Blocked);
        r.output.olad = Some(OladPatch {
            reachable: false,
            unpatched: Vec::new(),
        });
        assert_eq!(states(&r), vec![Ready, Ready, Ready, Ready, Unknown]);
    }

    #[test]
    fn a_profile_without_dmx_still_checks_its_shows() {
        let mut r = ready();
        r.dmx = false;
        r.venue = None;
        r.shows = vec![broken_show("Esaweg")];
        assert_eq!(
            states(&r),
            vec![Unknown, Unknown, Unknown, Blocked, Blocked]
        );
        assert_eq!(r.verdicts()[4].findings[0].fix, Fix::Config);
    }

    #[test]
    fn the_shows_summary_counts_what_wont_load_apart_from_what_to_check() {
        let mut r = ready();
        let mut unbound = broken_show("Decay");
        unbound.error = None;
        unbound.warnings = vec![warning("unbound-focus-point", Severity::Attention)];
        r.shows = vec![broken_show("Esaweg"), unbound];
        let shows = &r.verdicts()[3];
        assert_eq!(shows.state, Blocked);
        assert_eq!(
            shows.summary,
            vec![
                Summary::ShowsBlocked { count: 1 },
                Summary::Attention { count: 1 }
            ]
        );
        assert_eq!(shows.findings[1].fix, Fix::Fit { group: None });
        assert_eq!(shows.findings[1].song.as_deref(), Some("Decay"));
    }
}
