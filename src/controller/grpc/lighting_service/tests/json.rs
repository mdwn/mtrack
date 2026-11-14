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

// ---- Fixture types ----

fn optional<T: serde::Serialize>(value: &Option<T>) -> Value {
    serde_json::to_value(value).unwrap()
}

fn type_channels(t: &proto::FixtureTypeChannels) -> Value {
    json!({
        "name": t.name,
        "channels": t.channels,
        "max_strobe_frequency": t.max_strobe_frequency,
        "min_strobe_frequency": t.min_strobe_frequency,
        "strobe_dmx_offset": t.strobe_dmx_offset,
    })
}

/// A fixture type as the REST routes sent it. A listing's entry carries
/// `footprint` (null for a GDTF type's) and, for a GDTF type, `gdtf` (null
/// when the archive did not read); a single type's carries `dsl` (null with
/// no file) instead. An unset field is a null, never a missing key, except
/// the ones the routes left out.
pub(in super::super) fn fixture_type_info(i: &proto::FixtureTypeInfo, listing: bool) -> Value {
    let mut entry = json!({
        "referential": i.referential,
        "rich": i.rich,
        "fixture_type": type_channels(i.fixture_type.as_ref().unwrap()),
        "file": i.file,
        "extension": i.extension,
        "owner": i.owner.as_ref().map(|o| json!({
            "file": o.file, "editable": o.editable, "reason": o.reason,
        })),
        "form_editable": i.form_editable,
    });
    if listing {
        entry["footprint"] = optional(&i.footprint);
        if let Some(card) = &i.gdtf {
            entry["gdtf"] = json!({
                "fixture": card.fixture,
                "manufacturer": card.manufacturer,
                "modes": card.modes,
                "beam": card.beam.as_ref().map(|b| json!({
                    "type": b.beam_type, "angle": b.angle,
                })),
                "thumbnail": card.thumbnail,
                "used_by": card.used_by,
                "in_use": card.in_use.iter().map(|m| json!({
                    "mode": m.mode, "count": m.count,
                })).collect::<Vec<_>>(),
            });
        } else if i.referential {
            entry["gdtf"] = Value::Null;
        }
    } else {
        entry["dsl"] = optional(&i.dsl);
    }
    entry
}

pub(in super::super) fn fixture_type_list(l: &proto::ListFixtureTypesResponse) -> Value {
    json!({
        "fixture_types": l.fixture_types.iter()
            .map(|(name, info)| (name.clone(), fixture_type_info(info, true)))
            .collect::<serde_json::Map<_, _>>(),
        "errors": l.errors.iter()
            .map(|e| json!({"file": e.file, "error": e.error}))
            .collect::<Vec<_>>(),
    })
}

pub(in super::super) fn rename(r: &proto::FixtureTypeRename) -> Value {
    json!({
        "from": r.from,
        "to": r.to,
        "venues": r.venues.iter().map(|v| json!({
            "venue": v.venue, "file": v.file, "lines": v.lines,
        })).collect::<Vec<_>>(),
        "lines": r.lines,
    })
}

pub(in super::super) fn fixture_type_saved(s: &proto::SaveFixtureTypeResponse) -> Value {
    json!({
        "status": "saved",
        "name": s.name,
        "rename": s.rename.as_ref().map(rename),
        "venue_error": s.venue_error.as_ref().map(venue_error),
    })
}

pub(in super::super) fn fixture_type_deleted(d: &proto::DeleteFixtureTypeResponse) -> Value {
    json!({
        "status": "deleted",
        "name": d.name,
        "archive_removed": d.archive_removed,
        "archive_kept": d.archive_kept,
    })
}

fn gdtf_mode(m: &proto::GdtfModeInfo) -> Value {
    use proto::gdtf_mode_info::Outcome;
    let mut mode = json!({"name": m.name, "footprint": m.footprint});
    match m.outcome.as_ref().unwrap() {
        Outcome::Refused(reason) => mode["refused"] = json!(reason),
        Outcome::Facts(f) => {
            mode["capabilities"] = json!(f.capabilities.iter().map(capability).collect::<Vec<_>>());
            mode["wants"] = json!(f.wants.iter().map(want).collect::<Vec<_>>());
            mode["cells"] = json!(f.cells);
            if let Some(range) = &f.strobe_range {
                mode["strobe_range"] = json!({"min_hz": range.min_hz, "max_hz": range.max_hz});
            }
            mode["channels"] = json!(f
                .channels
                .iter()
                .map(|c| json!([c.offset, c.name]))
                .collect::<Vec<_>>());
        }
    }
    mode
}

/// What `GET /api/lighting/fixture-types/{name}/gdtf` sent.
pub(in super::super) fn gdtf_details(g: &proto::FixtureTypeGdtfDetails) -> Value {
    json!({
        "archive": g.archive,
        "rig": g.rig,
        "thumbnail": g.thumbnail,
        "beam": g.beam.as_ref().map(|b| json!({
            "type": b.beam_type,
            "beam_angle": b.beam_angle,
            "field_angle": b.field_angle,
            "luminous_flux": b.luminous_flux,
            "color_temperature": b.color_temperature,
            "power": b.power,
        })),
        "about": g.about,
        "venues": g.venues.iter().map(|v| json!({
            "name": v.name,
            "fixtures": v.fixtures.iter()
                .map(|f| json!({"name": f.name, "mode": f.mode}))
                .collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "inspection": {"modes": g.modes.iter().map(gdtf_mode).collect::<Vec<_>>()},
    })
}

fn strobe_curve(value: i32) -> Value {
    match proto::FixtureStrobeCurve::try_from(value).unwrap() {
        proto::FixtureStrobeCurve::Unspecified => Value::Null,
        proto::FixtureStrobeCurve::Period => json!("period"),
        proto::FixtureStrobeCurve::Linear => json!("linear"),
        proto::FixtureStrobeCurve::Declared => json!("declared"),
    }
}

pub(in super::super) fn fixture_settings(s: &proto::FixtureTypeSettings) -> Value {
    let movement = s.movement.as_ref().unwrap();
    json!({
        "name": s.name,
        "movement": {
            "max_pan_speed": movement.max_pan_speed,
            "max_tilt_speed": movement.max_tilt_speed,
        },
        "strobe_curve": strobe_curve(s.strobe_curve),
        "strobe": s.strobe.as_ref().map(|t| json!({
            "steps": t.steps, "automatic": strobe_curve(t.automatic),
        })),
        "version": s.revision,
    })
}

pub(in super::super) fn settings_saved(s: &proto::FixtureTypeSettingsSaved) -> Value {
    json!({
        "version": s.revision,
        "rename": s.rename.as_ref().map(rename),
        "venue_versions": s.venue_revisions.iter().collect::<std::collections::BTreeMap<_, _>>(),
        "venue_error": s.venue_error.as_ref().map(venue_error),
    })
}

// ---- Venues ----

fn point(v: &proto::LightingVec3) -> Value {
    json!([v.x, v.y, v.z])
}

fn venue_fixture(f: &proto::LightingFixture) -> Value {
    let mut fixture = json!({
        "name": f.name,
        "fixture_type": f.fixture_type,
        "universe": f.universe,
        "start_channel": f.start_channel,
        "tags": f.tags,
        "position": f.position.as_ref().map(point),
        "rotation": f.rotation.as_ref().map(point),
    });
    // Serialized only when present, as the venue always was.
    if let Some(mode) = &f.mode {
        fixture["mode"] = json!(mode);
    }
    if let Some(angle) = f.beam_angle {
        fixture["beam_angle"] = json!(angle);
    }
    fixture
}

/// A venue as the web UI's `Venue` type had it.
pub(in super::super) fn venue(v: &proto::LightingVenue) -> Value {
    json!({
        "name": v.name,
        "fixtures": v.fixtures.iter()
            .map(|(name, f)| (name.clone(), venue_fixture(f)))
            .collect::<serde_json::Map<_, _>>(),
        "focus_points": v.focus_points.iter()
            .map(|(name, p)| (name.clone(), point(p)))
            .collect::<serde_json::Map<_, _>>(),
        "source": v.source.as_ref().map(|s| json!({
            "mvr": s.mvr,
            "origin": s.origin.as_ref().map(point),
        })),
    })
}

/// What `GET /api/lighting/venues` sent.
pub(in super::super) fn venue_list(l: &proto::ListVenuesResponse) -> Value {
    json!({
        "venues": l.venues.iter()
            .map(|(name, v)| (name.clone(), venue(v)))
            .collect::<serde_json::Map<_, _>>(),
        "owners": l.owners.iter()
            .map(|(name, o)| (name.clone(), json!({
                "file": o.file, "editable": o.editable, "reason": o.reason,
            })))
            .collect::<serde_json::Map<_, _>>(),
        "errors": l.errors.iter()
            .map(|e| json!({"file": e.file, "error": e.error}))
            .collect::<Vec<_>>(),
        "versions": l.revisions.iter()
            .map(|(name, v)| (name.clone(), json!(v)))
            .collect::<serde_json::Map<_, _>>(),
    })
}

/// What `GET /api/lighting/venues/{name}` sent.
pub(in super::super) fn venue_detail(d: &proto::VenueDetail) -> Value {
    json!({
        "venue": venue(d.venue.as_ref().unwrap()),
        "dsl": d.dsl,
        "version": d.revision,
    })
}

/// What `PUT /api/lighting/venues/{name}` sent.
pub(in super::super) fn venue_saved(s: &proto::SaveVenueResponse) -> Value {
    json!({
        "status": "saved",
        "name": s.name,
        "reloaded": s.reloaded,
        "version": s.revision,
        "venue_error": s.venue_error.as_ref().map(venue_error),
    })
}

/// What `GET /api/lighting/venues/{name}/patch` sent.
pub(in super::super) fn venue_patch(p: &proto::VenuePatch) -> Value {
    json!({
        "spans": p.spans.iter().map(|s| json!({
            "fixture": s.fixture,
            "universe": s.universe,
            "address": s.address,
            "footprint": s.footprint,
            "type": s.fixture_type,
            "mode": s.mode,
        })).collect::<Vec<_>>(),
        "overlaps": p.overlaps.iter().map(|o| json!({
            "first": o.first,
            "second": o.second,
            "first_gang": o.first_gang,
            "second_gang": o.second_gang,
            "universe": o.universe,
            "from": o.from,
            "to": o.to,
        })).collect::<Vec<_>>(),
        "overruns": p.overruns.iter().map(|o| json!({
            "fixture": o.fixture,
            "universe": o.universe,
            "address": o.address,
            "footprint": o.footprint,
            "last": o.last,
        })).collect::<Vec<_>>(),
    })
}

/// What `GET /api/lighting/venues/{name}/scene` sent.
pub(in super::super) fn venue_scene(s: &proto::VenueScene) -> Value {
    let stage = s.venue.as_ref().unwrap();
    json!({
        "fixtures": s.fixtures.iter().map(|(name, f)| (name.clone(), json!({
            "tags": f.tags,
            "type": f.fixture_type,
            "position": f.position.as_ref().map(point),
            "rotation": f.rotation.as_ref().map(point),
            "beam_angle": f.beam_angle,
            "rig": f.rig,
            "mode": f.mode,
        }))).collect::<serde_json::Map<_, _>>(),
        "venue": {
            "name": stage.name,
            "focus_points": stage.focus_points.iter()
                .map(|(name, p)| (name.clone(), point(p)))
                .collect::<serde_json::Map<_, _>>(),
            "scenery": stage.scenery,
            "scenery_error": stage.scenery_error,
        },
    })
}

/// What `POST /api/lighting/venues/{name}/aim-points` sent.
pub(in super::super) fn aim_points(a: &proto::AddVenueAimPointsResponse) -> Value {
    json!({
        "created": a.created.iter().map(|c| json!({
            "name": c.name,
            "fixture": c.fixture,
            "point": c.point.as_ref().map(point),
        })).collect::<Vec<_>>(),
        "reloaded": a.reloaded,
        "version": a.revision,
        "venue_error": a.venue_error.as_ref().map(venue_error),
    })
}

/// What `GET /api/lighting/mvr/export/summary` sent.
pub(in super::super) fn export_summary(s: &proto::MvrExportSummary) -> Value {
    json!({
        "file_name": s.file_name,
        "venue": s.venue,
        "output": s.output,
        "fixtures": s.fixtures,
        "focus_points": s.focus_points,
        "embedded_gdtfs": s.embedded_gdtfs,
        "generated_gdtfs": s.generated_gdtfs.iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect::<serde_json::Map<_, _>>(),
        "positioned_fixtures": s.positioned_fixtures,
        "fixed_fixtures": s.fixed_fixtures.iter()
            .map(|f| json!({"name": f.name, "focus": f.focus}))
            .collect::<Vec<_>>(),
        "warnings": s.warnings,
    })
}

/// What `POST /api/lighting/gdtf/import` sent.
pub(in super::super) fn gdtf_import(g: &proto::GdtfImport) -> Value {
    json!({
        "type_name": g.type_name,
        "fixture": g.fixture,
        "manufacturer": g.manufacturer,
        "modes": g.modes,
        "archive": g.archive,
        "already_imported": g.already_imported,
        "renamed_from": g.renamed_from,
        "refused_modes": g.refused_modes.iter()
            .map(|m| json!({"mode": m.mode, "reason": m.reason}))
            .collect::<Vec<_>>(),
        "fixture_file": g.fixture_file,
    })
}

fn hand_edits(edits: &[proto::MvrHandEdit]) -> Vec<Value> {
    edits
        .iter()
        .map(|e| json!({"field": e.field, "mine": e.mine, "mvr": e.mvr}))
        .collect()
}

fn mvr_plan(p: &proto::MvrPlan) -> Value {
    json!({
        "venue_name": p.venue_name,
        "venue_file": p.venue_file,
        "archive": p.archive,
        "merge": p.merge,
        "origin": p.origin.as_ref().map(point),
        "fixture_types": p.fixture_types.iter().map(|t| json!({
            "name": t.name,
            "archive": t.archive,
            "modes": t.modes,
            "existing": t.existing,
            "fixture_file": t.fixture_file,
        })).collect::<Vec<_>>(),
        "fixtures": p.fixtures.iter().map(|f| json!({
            "name": f.name,
            "layer": f.layer,
            "fixture_type": f.fixture_type,
            "mode": f.mode,
            "patch": f.patch.as_ref().map(|p| json!([p.universe, p.address])),
            "position": f.position.as_ref().map(point),
            "rotation": f.rotation.as_ref().map(point),
            "tags": f.tags,
            "todo": f.todo,
            "change": f.change,
            "overwrites": hand_edits(&f.overwrites),
            "kept_edits": f.kept_edits,
        })).collect::<Vec<_>>(),
        "removed_fixtures": p.removed_fixtures.iter()
            .map(|f| json!({"name": f.name, "tags": f.tags}))
            .collect::<Vec<_>>(),
        "kept_fixtures": p.kept_fixtures,
        "focus_points": p.focus_points.iter().map(|f| json!({
            "name": f.name,
            "point": f.point.as_ref().map(point),
            "change": f.change,
            "overwrites": hand_edits(&f.overwrites),
            "kept_edit": f.kept_edit,
        })).collect::<Vec<_>>(),
        "removed_focus_points": p.removed_focus_points,
        "kept_focus_points": p.kept_focus_points,
        "scenery_objects": p.scenery_objects,
        "scenery_meshes_undrawn": p.scenery_meshes_undrawn,
        "warnings": p.warnings,
    })
}

fn bounds(b: &proto::MvrBounds) -> Value {
    json!({
        "min": b.min.as_ref().map(point),
        "max": b.max.as_ref().map(point),
    })
}

/// What `POST /api/lighting/mvr/inspect` sent.
pub(in super::super) fn mvr_inspection(i: &proto::MvrInspection) -> Value {
    let scene = i.scene.as_ref().unwrap();
    json!({
        "file_name": i.file_name,
        "report": mvr_plan(i.report.as_ref().unwrap()),
        "scene": {
            "fixtures": scene.fixtures.iter().map(|f| json!({
                "name": f.name,
                "layer": f.layer,
                "position_mm": f.position_mm.as_ref().map(point),
            })).collect::<Vec<_>>(),
            "focus_points": scene.focus_points.iter().map(|f| json!({
                "name": f.name,
                "position_mm": f.position_mm.as_ref().map(point),
            })).collect::<Vec<_>>(),
            "scenery": scene.scenery.iter().map(|o| json!({
                "name": o.name,
                "kind": o.kind,
                "deck": o.deck,
                "bounds_mm": o.bounds_mm.as_ref().map(bounds),
            })).collect::<Vec<_>>(),
            "deck_mm": scene.deck.as_ref().map(bounds),
        },
    })
}

/// What `POST /api/lighting/mvr/import` sent: the plan, or the report (the
/// plan with what was written, flattened), and for a write whether the
/// engine reloaded and the venue's failure.
pub(in super::super) fn mvr_import_answer(a: &proto::ImportMvrResponse) -> Value {
    let mut answer = json!({"write": a.write});
    if let Some(plan) = &a.plan {
        answer["plan"] = mvr_plan(plan);
    }
    if let Some(report) = &a.report {
        let mut flat = mvr_plan(report.plan.as_ref().unwrap());
        flat["written"] = json!(report.written);
        flat["distillation_warnings"] = report
            .distillation_warnings
            .iter()
            .map(|(k, v)| (k.clone(), json!(v.values)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        answer["report"] = flat;
    }
    if a.write {
        answer["reloaded"] = json!(a.reloaded);
        answer["venue_error"] = json!(a.venue_error.as_ref().map(venue_error));
    }
    answer
}
