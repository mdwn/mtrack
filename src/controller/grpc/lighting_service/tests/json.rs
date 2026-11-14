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

//! The answers of `LightingService` read back as the JSON the REST routes
//! sent (the web UI's wire types), so the tests of what the routes said
//! assert on exactly that, through the RPC: a field the proto drops fails
//! them.

use serde_json::{json, Value};

use crate::proto::player::v1 as proto;

fn severity(value: i32) -> &'static str {
    match proto::LightingSeverity::try_from(value).unwrap() {
        proto::LightingSeverity::Blocked => "blocked",
        proto::LightingSeverity::Attention => "attention",
        proto::LightingSeverity::Unknown => "unknown",
        proto::LightingSeverity::Note => "note",
        proto::LightingSeverity::Unspecified => panic!("severity unspecified"),
    }
}

fn want(value: &i32) -> &'static str {
    match proto::LightingWant::try_from(*value).unwrap() {
        proto::LightingWant::Move => "move",
        proto::LightingWant::Color => "color",
        proto::LightingWant::Strobe => "strobe",
        proto::LightingWant::Cells => "cells",
        proto::LightingWant::Dimmer => "dimmer",
        proto::LightingWant::Unspecified => panic!("want unspecified"),
    }
}

fn warning(w: &proto::LightingWarning) -> Value {
    json!({"kind": w.kind, "message": w.message, "severity": severity(w.severity)})
}

fn venue_error(e: &proto::LightingVenueError) -> Value {
    json!({"venue": e.venue, "fixture": e.fixture, "reason": e.reason})
}

fn unresolved(u: &proto::ReadinessUnresolvedFixture) -> Value {
    json!({"fixture": u.fixture, "type": u.r#type, "reason": u.reason})
}

pub(in super::super) fn readiness(r: &proto::LightingReadiness) -> Value {
    let types = r.fixture_types.as_ref().unwrap();
    let output = r.output.as_ref().unwrap();
    json!({
        "dmx": r.dmx,
        "venue": r.venue.as_ref().map(|v| json!({
            "name": v.name, "fixtures": v.fixtures, "placed": v.placed,
            "focus_points": v.focus_points,
        })),
        "venue_error": r.venue_error.as_ref().map(venue_error),
        "patch_warnings": r.patch_warnings.iter().map(warning).collect::<Vec<_>>(),
        "fixture_types": {
            "in_use": types.in_use,
            "unresolved": types.unresolved.iter().map(unresolved).collect::<Vec<_>>(),
            "library": types.library.iter().map(|f| json!({
                "kind": f.kind, "file": f.file, "message": f.message,
            })).collect::<Vec<_>>(),
        },
        "groups": r.groups.iter().map(|g| json!({
            "name": g.name, "fixtures": g.fixtures, "songs": g.songs,
        })).collect::<Vec<_>>(),
        "shows": r.shows.iter().map(|s| {
            let mut show = json!({
                "song": s.song, "files": s.files,
                "warnings": s.warnings.iter().map(warning).collect::<Vec<_>>(),
            });
            if let Some(error) = &s.error {
                show["error"] = json!(error);
            }
            show
        }).collect::<Vec<_>>(),
        "output": {
            "universes": output.universes,
            "unconfigured": output.unconfigured,
            "olad": output.olad.as_ref().map(|o| json!({
                "reachable": o.reachable, "unpatched": o.unpatched,
            })),
        },
        "checks": r.checks.iter().map(check).collect::<Vec<_>>(),
    })
}

fn check(c: &proto::ReadinessCheck) -> Value {
    let key = match proto::ReadinessCheckKey::try_from(c.key).unwrap() {
        proto::ReadinessCheckKey::Fixtures => "fixtures",
        proto::ReadinessCheckKey::Venue => "venue",
        proto::ReadinessCheckKey::Groups => "groups",
        proto::ReadinessCheckKey::Shows => "shows",
        proto::ReadinessCheckKey::Output => "output",
        proto::ReadinessCheckKey::Unspecified => panic!("key unspecified"),
    };
    let state = match proto::ReadinessCheckState::try_from(c.state).unwrap() {
        proto::ReadinessCheckState::Ready => "ready",
        proto::ReadinessCheckState::Attention => "attention",
        proto::ReadinessCheckState::Blocked => "blocked",
        proto::ReadinessCheckState::Unknown => "unknown",
        proto::ReadinessCheckState::Unspecified => panic!("state unspecified"),
    };
    json!({
        "key": key,
        "state": state,
        "summary": c.summary.iter().map(summary).collect::<Vec<_>>(),
        "findings": c.findings.iter().map(finding).collect::<Vec<_>>(),
    })
}

fn summary(s: &proto::ReadinessSummary) -> Value {
    use proto::readiness_summary::Kind;
    let count = |kind: &str, c: &proto::ReadinessCount| json!({"kind": kind, "count": c.count});
    let bare = |kind: &str| json!({"kind": kind});
    let venue = |kind: &str, v: &proto::ReadinessVenueSummary| json!({"kind": kind, "venue": v.venue, "fixtures": v.fixtures, "unplaced": v.unplaced});
    match s.kind.as_ref().unwrap() {
        Kind::Blocked(c) => count("blocked", c),
        Kind::ShowsBlocked(c) => count("showsBlocked", c),
        Kind::Attention(c) => count("attention", c),
        Kind::Unknown(_) => bare("unknown"),
        Kind::NoDmx(_) => bare("noDmx"),
        Kind::NeedsVenue(_) => bare("needsVenue"),
        Kind::ShowsNone(_) => bare("showsNone"),
        Kind::ShowsReady(c) => count("showsReady", c),
        Kind::TypesReady(c) => count("typesReady", c),
        Kind::VenueAllPlaced(v) => venue("venueAllPlaced", v),
        Kind::VenueSomeUnplaced(v) => venue("venueSomeUnplaced", v),
        Kind::GroupsNone(_) => bare("groupsNone"),
        Kind::GroupsReady(c) => count("groupsReady", c),
        Kind::OutputReady(c) => count("outputReady", c),
    }
}

fn finding(f: &proto::ReadinessFinding) -> Value {
    json!({
        "severity": severity(f.severity),
        "what": what(f.what.as_ref().unwrap()),
        "fix": fix(f.fix.as_ref().unwrap()),
        "song": f.song,
    })
}

fn what(w: &proto::ReadinessWhat) -> Value {
    use proto::readiness_what::Kind;
    match w.kind.as_ref().unwrap() {
        Kind::ShowError(e) => json!({"kind": "showError", "song": e.song, "detail": e.detail}),
        Kind::Lint(d) => json!({"kind": "lint", "detail": d.detail}),
        Kind::NoDmx(_) => json!({"kind": "noDmx"}),
        Kind::TypeUnresolved(u) => {
            let mut value = unresolved(u);
            value["kind"] = json!("typeUnresolved");
            value
        }
        Kind::NoVenue(_) => json!({"kind": "noVenue"}),
        Kind::VenueFailed(e) => {
            let mut value = venue_error(e);
            value["kind"] = json!("venueFailed");
            value
        }
        Kind::EmptyVenue(v) => json!({"kind": "emptyVenue", "venue": v.venue}),
        Kind::Patch(d) => json!({"kind": "patch", "detail": d.detail}),
        Kind::GroupEmpty(g) => json!({
            "kind": "groupEmpty", "group": g.group, "venue": g.venue, "songs": g.songs,
        }),
        Kind::UniverseUnconfigured(u) => {
            json!({"kind": "universeUnconfigured", "universe": u.universe})
        }
        Kind::OladUnreachable(_) => json!({"kind": "oladUnreachable"}),
        Kind::UniverseUnpatched(u) => json!({"kind": "universeUnpatched", "universe": u.universe}),
    }
}

fn fix(f: &proto::ReadinessFix) -> Value {
    use proto::readiness_fix::Kind;
    match f.kind.as_ref().unwrap() {
        Kind::Song(s) => json!({"kind": "song", "song": s.song}),
        Kind::Fit(g) => json!({"kind": "fit", "group": g.group}),
        Kind::Fixtures(_) => json!({"kind": "fixtures"}),
        Kind::Venues(_) => json!({"kind": "venues"}),
        Kind::Config(_) => json!({"kind": "config"}),
        Kind::ChooseVenue(_) => json!({"kind": "chooseVenue"}),
    }
}

fn height(value: Option<i32>) -> Value {
    json!(value.map(|v| match proto::FitHeight::try_from(v).unwrap() {
        proto::FitHeight::Deck => "deck",
        proto::FitHeight::Low => "low",
        proto::FitHeight::Truss => "truss",
        proto::FitHeight::Unspecified => panic!("height unspecified"),
    }))
}

fn depth(value: Option<i32>) -> Value {
    json!(value.map(|v| match proto::FitDepth::try_from(v).unwrap() {
        proto::FitDepth::Downstage => "downstage",
        proto::FitDepth::Mid => "mid",
        proto::FitDepth::Upstage => "upstage",
        proto::FitDepth::Unspecified => panic!("depth unspecified"),
    }))
}

fn capability(value: &i32) -> &'static str {
    match proto::FitCapability::try_from(*value).unwrap() {
        proto::FitCapability::Color => "color",
        proto::FitCapability::PanTilt => "pan_tilt",
        proto::FitCapability::Strobe => "strobe",
        proto::FitCapability::Cells => "cells",
        proto::FitCapability::Dimmer => "dimmer",
        proto::FitCapability::White => "white",
        proto::FitCapability::Zoom => "zoom",
        proto::FitCapability::Focus => "focus",
        proto::FitCapability::Gobo => "gobo",
        proto::FitCapability::ColorWheel => "color_wheel",
        proto::FitCapability::Unspecified => panic!("capability unspecified"),
    }
}

fn vec3(v: &proto::LightingVec3) -> Value {
    json!([v.x, v.y, v.z])
}

pub(in super::super) fn fit(r: &proto::LightingFit) -> Value {
    let output = r.output.as_ref().unwrap();
    json!({
        "venue": r.venue.as_ref().map(|v| json!({
            "name": v.name,
            "fixtures": v.fixtures.iter().map(|f| json!({
                "name": f.name, "type": f.r#type, "tags": f.tags,
                "position": f.position.as_ref().map(vec3),
                "capabilities": f.capabilities.iter().map(capability).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "focus_points": v.focus_points,
            "error": v.error.as_ref().map(|e| json!({"fixture": e.fixture, "reason": e.reason})),
        })),
        "groups": r.groups.iter().map(|g| json!({
            "name": g.name,
            "defined": g.defined,
            "needs": {
                "all_of": g.needs.as_ref().unwrap().all_of,
                "any_of": g.needs.as_ref().unwrap().any_of,
                "prefer": g.needs.as_ref().unwrap().prefer,
            },
            "tags": g.tags,
            "fixtures": g.fixtures,
            "songs": g.songs,
            "wants": g.wants.iter().map(want).collect::<Vec<_>>(),
            "suggestion": g.suggestion.as_ref().map(|s| {
                let reason = s.reason.as_ref().unwrap();
                json!({
                    "fixtures": s.fixtures,
                    "tags": s.tags,
                    "reason": {
                        "count": reason.count,
                        "type": reason.r#type,
                        "where": reason.r#where,
                        "height": height(reason.height),
                        "depth": depth(reason.depth),
                        "can": reason.can.iter().map(want).collect::<Vec<_>>(),
                    },
                })
            }),
            "others": g.others.iter().map(|c| json!({
                "fixtures": c.fixtures, "type": c.r#type, "where": c.r#where,
                "height": height(c.height), "depth": depth(c.depth),
            })).collect::<Vec<_>>(),
            "unmet": g.unmet.iter().map(want).collect::<Vec<_>>(),
            "unmet_together": g.unmet_together,
        })).collect::<Vec<_>>(),
        "focus_points_wanted": r.focus_points_wanted.iter().map(|f| json!({
            "name": f.name, "songs": f.songs,
        })).collect::<Vec<_>>(),
        "output": {
            "unconfigured": output.unconfigured,
            "unpatched": output.unpatched,
            "reachable": output.reachable,
            "ola_http_port": output.ola_http_port,
        },
    })
}

fn channels(c: &proto::PreviewChannels) -> Value {
    json!(c.values)
}

/// The preview as the REST route sent it: each evaluation's stage values
/// flattened beside its time and its effects.
pub(in super::super) fn preview(p: &proto::LightingPreview) -> Value {
    json!({
        "song": p.song,
        "evaluations": p.evaluations.iter().map(|e| {
            let frame = e.frame.as_ref().unwrap();
            json!({
                "time": e.time,
                "fixtures": frame.fixtures.iter().map(|(k, v)| (k.clone(), channels(v))).collect::<serde_json::Map<_, _>>(),
                "poses": frame.poses.iter().map(|(k, pose)| (k.clone(), json!({
                    "pan": pose.pan,
                    "tilt": pose.tilt,
                    "aim": vec3(pose.aim.as_ref().unwrap()),
                    "floor": pose.floor.as_ref().map(|f| json!([f.x, f.y])),
                }))).collect::<serde_json::Map<_, _>>(),
                "cells": frame.cells.iter().map(|(k, cells)| (k.clone(), json!(
                    cells.cells.iter().map(|(cell, v)| (cell.clone(), channels(v))).collect::<serde_json::Map<_, _>>()
                ))).collect::<serde_json::Map<_, _>>(),
                "strobe_rates": frame.strobe_rates,
                "active_effects": e.active_effects.iter().map(|effect| json!({
                    "id": effect.id,
                    "groups": effect.groups.as_ref().map(|g| &g.names),
                    "kind": effect.kind,
                    "layer": effect.layer,
                    "elapsed": effect.elapsed,
                    "duration": effect.duration,
                    "fixtures": effect.fixtures,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "untouched": p.untouched,
        "venue_error": p.venue_error,
    })
}

pub(in super::super) fn groups(g: &proto::ListLightingGroupsResponse) -> Value {
    let mut value = json!({
        "groups": g.groups.iter().map(|g| json!({"name": g.name, "fixtures": g.fixtures})).collect::<Vec<_>>(),
    });
    if let Some(resolved) = g.resolved {
        value["resolved"] = json!(resolved);
    }
    value
}

fn hz_range(r: &proto::FixtureTestHzRange) -> Value {
    json!({"min_hz": r.min_hz, "max_hz": r.max_hz})
}

fn degree_range(r: &proto::FixtureTestDegreeRange) -> Value {
    json!({"min": r.min, "max": r.max})
}

pub(in super::super) fn test_options(o: &proto::FixtureTestOptions) -> Value {
    json!({
        "fixture_type": o.fixture_type,
        "gdtf": o.gdtf,
        "modes": o.modes.iter().map(|m| json!({
            "name": m.name,
            "drivable": m.drivable,
            "error": m.error,
            "footprint": m.footprint,
            "controls": m.controls.as_ref().map(|c| json!({
                "color": c.color,
                "dimmer": c.dimmer.map(|d| match proto::FixtureTestDimmer::try_from(d).unwrap() {
                    proto::FixtureTestDimmer::Channel => "channel",
                    proto::FixtureTestDimmer::Folded => "folded",
                    proto::FixtureTestDimmer::Unspecified => panic!("dimmer unspecified"),
                }),
                "strobe": c.strobe.as_ref().map(hz_range),
                "pan": c.pan.as_ref().map(degree_range),
                "tilt": c.tilt.as_ref().map(degree_range),
                "levels": c.levels,
            })),
            "channels": m.channels.iter().map(|c| json!({"offset": c.offset, "name": c.name})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "default_mode": o.default_mode,
        "universes": o.universes.iter().map(|u| json!({
            "universe": u.universe, "name": u.name, "patched": u.patched,
        })).collect::<Vec<_>>(),
        "olad": {
            "reachable": o.olad.as_ref().unwrap().reachable,
            "port": o.olad.as_ref().unwrap().port,
        },
        "available": o.available,
        "unavailable": o.unavailable.map(|u| match proto::FixtureTestUnavailable::try_from(u).unwrap() {
            proto::FixtureTestUnavailable::Locked => "locked",
            proto::FixtureTestUnavailable::Playing => "playing",
            proto::FixtureTestUnavailable::NoDmx => "no_dmx",
            proto::FixtureTestUnavailable::NoUniverses => "no_universes",
            proto::FixtureTestUnavailable::Unspecified => panic!("unavailable unspecified"),
        }),
        "unavailable_message": o.unavailable_message,
    })
}

pub(in super::super) fn test_answer(a: &proto::FixtureTestAnswer) -> Value {
    use proto::fixture_test_warning::Kind;
    json!({
        "active": a.active,
        "universe": a.universe,
        "address": a.address,
        "footprint": a.footprint,
        "fixture_type": a.fixture_type,
        "mode": a.mode,
        "expires_in_secs": a.expires_in_secs,
        "frame": a.frame.iter().map(|c| json!({
            "offset": c.offset, "address": c.address, "name": c.name,
            "value": c.value, "raw": c.raw,
        })).collect::<Vec<_>>(),
        "warnings": a.warnings.iter().map(|w| match w.kind.as_ref().unwrap() {
            Kind::VenueOverlap(o) => json!({
                "kind": "venue_overlap", "fixtures": o.fixtures, "message": o.message,
            }),
            Kind::PastUniverseEnd(m) => json!({"kind": "past_universe_end", "message": m.message}),
            Kind::OladUnreachable(m) => json!({"kind": "olad_unreachable", "message": m.message}),
            Kind::UniverseUnpatched(m) => json!({"kind": "universe_unpatched", "message": m.message}),
            Kind::UnsupportedControl(u) => json!({
                "kind": "unsupported_control", "control": u.control, "message": u.message,
            }),
        }).collect::<Vec<_>>(),
    })
}
