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

//! `LightingService`'s answers: the lighting pages' facts (the plain
//! functions of `webui::api::lighting_api` and `fixture_test`) as proto
//! messages. Each is the same facts the REST routes sent as JSON.

use std::collections::HashMap;

use crate::lighting::findings::{LibraryFinding, Severity, VenueError, Warning};
use crate::lighting::fit::{self, Capability, Depth, Height, Want};
use crate::lighting::fixture_test::{
    ControlSupport, DegreeRange, DimmerControl, FrameChannel, HzRange,
};
use crate::lighting::types::{FixtureType, MovementLimits, StrobeCurve};
use crate::proto::player::v1 as proto;
use crate::webui::wire::fixture_test as test_wire;
use crate::webui::wire::fixture_types as types_wire;
use crate::webui::wire::preview as preview_wire;
use crate::webui::wire::readiness as ready;

fn vec3(v: [f64; 3]) -> proto::LightingVec3 {
    proto::LightingVec3 {
        x: v[0],
        y: v[1],
        z: v[2],
    }
}

fn u32s(values: &[u16]) -> Vec<u32> {
    values.iter().map(|v| u32::from(*v)).collect()
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

// ---- Findings ----

fn severity(s: Severity) -> proto::LightingSeverity {
    match s {
        Severity::Blocked => proto::LightingSeverity::Blocked,
        Severity::Attention => proto::LightingSeverity::Attention,
        Severity::Unknown => proto::LightingSeverity::Unknown,
        Severity::Note => proto::LightingSeverity::Note,
    }
}

fn want(w: Want) -> proto::LightingWant {
    match w {
        Want::Move => proto::LightingWant::Move,
        Want::Color => proto::LightingWant::Color,
        Want::Strobe => proto::LightingWant::Strobe,
        Want::Cells => proto::LightingWant::Cells,
        Want::Dimmer => proto::LightingWant::Dimmer,
    }
}

fn wants(ws: &[Want]) -> Vec<i32> {
    ws.iter().map(|w| want(*w) as i32).collect()
}

fn warning(w: &Warning) -> proto::LightingWarning {
    proto::LightingWarning {
        kind: w.kind.to_string(),
        message: w.message.clone(),
        severity: severity(w.severity) as i32,
    }
}

fn library_finding(f: &LibraryFinding) -> proto::LightingLibraryFinding {
    proto::LightingLibraryFinding {
        kind: f.kind.to_string(),
        file: f.file.clone(),
        message: f.message.clone(),
    }
}

fn venue_error(e: &VenueError) -> proto::LightingVenueError {
    proto::LightingVenueError {
        venue: e.venue.clone(),
        fixture: e.fixture.clone(),
        reason: e.reason.clone(),
    }
}

// ---- Readiness ----

pub(super) fn readiness(r: &ready::Readiness) -> proto::LightingReadiness {
    proto::LightingReadiness {
        dmx: r.dmx,
        venue: r.venue.as_ref().map(|v| proto::ReadinessVenue {
            name: v.name.clone(),
            fixtures: count(v.fixtures),
            placed: count(v.placed),
            focus_points: v.focus_points.clone(),
        }),
        venue_error: r.venue_error.as_ref().map(venue_error),
        patch_warnings: r.patch_warnings.iter().map(warning).collect(),
        fixture_types: Some(proto::ReadinessTypes {
            in_use: r.fixture_types.in_use.clone(),
            unresolved: r.fixture_types.unresolved.iter().map(unresolved).collect(),
            library: r
                .fixture_types
                .library
                .iter()
                .map(library_finding)
                .collect(),
        }),
        groups: r
            .groups
            .iter()
            .map(|g| proto::ReadinessGroup {
                name: g.name.clone(),
                fixtures: count(g.fixtures),
                songs: g.songs.clone(),
            })
            .collect(),
        shows: r
            .shows
            .iter()
            .map(|s| proto::ReadinessShow {
                song: s.song.clone(),
                files: s.files.clone(),
                error: s.error.clone(),
                warnings: s.warnings.iter().map(warning).collect(),
            })
            .collect(),
        output: Some(proto::ReadinessOutput {
            universes: u32s(&r.output.universes),
            unconfigured: u32s(&r.output.unconfigured),
            olad: r.output.olad.as_ref().map(|o| proto::ReadinessOladPatch {
                reachable: o.reachable,
                unpatched: u32s(&o.unpatched),
            }),
        }),
        checks: r.checks.iter().map(check).collect(),
    }
}

fn unresolved(u: &ready::UnresolvedFixture) -> proto::ReadinessUnresolvedFixture {
    proto::ReadinessUnresolvedFixture {
        fixture: u.fixture.clone(),
        r#type: u.fixture_type.clone(),
        reason: u.reason.clone(),
    }
}

fn check(c: &ready::Check) -> proto::ReadinessCheck {
    use ready::{CheckKey, CheckState};
    proto::ReadinessCheck {
        key: match c.key {
            CheckKey::Fixtures => proto::ReadinessCheckKey::Fixtures,
            CheckKey::Venue => proto::ReadinessCheckKey::Venue,
            CheckKey::Groups => proto::ReadinessCheckKey::Groups,
            CheckKey::Shows => proto::ReadinessCheckKey::Shows,
            CheckKey::Output => proto::ReadinessCheckKey::Output,
        } as i32,
        state: match c.state {
            CheckState::Ready => proto::ReadinessCheckState::Ready,
            CheckState::Attention => proto::ReadinessCheckState::Attention,
            CheckState::Blocked => proto::ReadinessCheckState::Blocked,
            CheckState::Unknown => proto::ReadinessCheckState::Unknown,
        } as i32,
        summary: c.summary.iter().map(summary).collect(),
        findings: c.findings.iter().map(finding).collect(),
    }
}

fn counted(n: usize) -> proto::ReadinessCount {
    proto::ReadinessCount { count: count(n) }
}

fn nothing() -> proto::ReadinessNothing {
    proto::ReadinessNothing {}
}

fn venue_summary(venue: &str, fixtures: usize, unplaced: usize) -> proto::ReadinessVenueSummary {
    proto::ReadinessVenueSummary {
        venue: venue.to_string(),
        fixtures: count(fixtures),
        unplaced: count(unplaced),
    }
}

fn summary(s: &ready::Summary) -> proto::ReadinessSummary {
    use proto::readiness_summary::Kind;
    use ready::Summary;
    proto::ReadinessSummary {
        kind: Some(match s {
            Summary::Blocked { count } => Kind::Blocked(counted(*count)),
            Summary::ShowsBlocked { count } => Kind::ShowsBlocked(counted(*count)),
            Summary::Attention { count } => Kind::Attention(counted(*count)),
            Summary::Unknown => Kind::Unknown(nothing()),
            Summary::NoDmx => Kind::NoDmx(nothing()),
            Summary::NeedsVenue => Kind::NeedsVenue(nothing()),
            Summary::ShowsNone => Kind::ShowsNone(nothing()),
            Summary::ShowsReady { count } => Kind::ShowsReady(counted(*count)),
            Summary::TypesReady { count } => Kind::TypesReady(counted(*count)),
            Summary::VenueAllPlaced {
                venue,
                fixtures,
                unplaced,
            } => Kind::VenueAllPlaced(venue_summary(venue, *fixtures, *unplaced)),
            Summary::VenueSomeUnplaced {
                venue,
                fixtures,
                unplaced,
            } => Kind::VenueSomeUnplaced(venue_summary(venue, *fixtures, *unplaced)),
            Summary::GroupsNone => Kind::GroupsNone(nothing()),
            Summary::GroupsReady { count } => Kind::GroupsReady(counted(*count)),
            Summary::OutputReady { count } => Kind::OutputReady(counted(*count)),
        }),
    }
}

fn finding(f: &ready::Finding) -> proto::ReadinessFinding {
    proto::ReadinessFinding {
        severity: severity(f.severity) as i32,
        what: Some(what(&f.what)),
        fix: Some(fix(&f.fix)),
        song: f.song.clone(),
    }
}

fn what(w: &ready::FindingWhat) -> proto::ReadinessWhat {
    use proto::readiness_what::Kind;
    use ready::FindingWhat;
    let universe = |universe: &u16| proto::ReadinessUniverse {
        universe: u32::from(*universe),
    };
    let detail = |detail: &String| proto::ReadinessDetail {
        detail: detail.clone(),
    };
    proto::ReadinessWhat {
        kind: Some(match w {
            FindingWhat::ShowError { song, detail } => Kind::ShowError(proto::ReadinessShowError {
                song: song.clone(),
                detail: detail.clone(),
            }),
            FindingWhat::Lint { detail: d } => Kind::Lint(detail(d)),
            FindingWhat::NoDmx => Kind::NoDmx(nothing()),
            FindingWhat::TypeUnresolved {
                fixture,
                fixture_type,
                reason,
            } => Kind::TypeUnresolved(proto::ReadinessUnresolvedFixture {
                fixture: fixture.clone(),
                r#type: fixture_type.clone(),
                reason: reason.clone(),
            }),
            FindingWhat::NoVenue => Kind::NoVenue(nothing()),
            FindingWhat::VenueFailed {
                venue,
                fixture,
                reason,
            } => Kind::VenueFailed(proto::LightingVenueError {
                venue: venue.clone(),
                fixture: fixture.clone(),
                reason: reason.clone(),
            }),
            FindingWhat::EmptyVenue { venue } => Kind::EmptyVenue(proto::ReadinessEmptyVenue {
                venue: venue.clone(),
            }),
            FindingWhat::Patch { detail: d } => Kind::Patch(detail(d)),
            FindingWhat::GroupEmpty {
                group,
                venue,
                songs,
            } => Kind::GroupEmpty(proto::ReadinessGroupEmpty {
                group: group.clone(),
                venue: venue.clone(),
                songs: songs.clone(),
            }),
            FindingWhat::UniverseUnconfigured { universe: u } => {
                Kind::UniverseUnconfigured(universe(u))
            }
            FindingWhat::OladUnreachable => Kind::OladUnreachable(nothing()),
            FindingWhat::UniverseUnpatched { universe: u } => Kind::UniverseUnpatched(universe(u)),
        }),
    }
}

fn fix(f: &ready::Fix) -> proto::ReadinessFix {
    use proto::readiness_fix::Kind;
    use ready::Fix;
    proto::ReadinessFix {
        kind: Some(match f {
            Fix::Song { song } => Kind::Song(proto::ReadinessFixSong { song: song.clone() }),
            Fix::Fit { group } => Kind::Fit(proto::ReadinessFixFit {
                group: group.clone(),
            }),
            Fix::Fixtures => Kind::Fixtures(nothing()),
            Fix::Venues => Kind::Venues(nothing()),
            Fix::Config => Kind::Config(nothing()),
            Fix::ChooseVenue => Kind::ChooseVenue(nothing()),
        }),
    }
}

// ---- Fit ----

fn height(h: Height) -> i32 {
    (match h {
        Height::Deck => proto::FitHeight::Deck,
        Height::Low => proto::FitHeight::Low,
        Height::Truss => proto::FitHeight::Truss,
    }) as i32
}

fn depth(d: Depth) -> i32 {
    (match d {
        Depth::Downstage => proto::FitDepth::Downstage,
        Depth::Mid => proto::FitDepth::Mid,
        Depth::Upstage => proto::FitDepth::Upstage,
    }) as i32
}

fn capability(c: &Capability) -> i32 {
    (match c {
        Capability::Color => proto::FitCapability::Color,
        Capability::PanTilt => proto::FitCapability::PanTilt,
        Capability::Strobe => proto::FitCapability::Strobe,
        Capability::Cells => proto::FitCapability::Cells,
        Capability::Dimmer => proto::FitCapability::Dimmer,
        Capability::White => proto::FitCapability::White,
        Capability::Zoom => proto::FitCapability::Zoom,
        Capability::Focus => proto::FitCapability::Focus,
        Capability::Gobo => proto::FitCapability::Gobo,
        Capability::ColorWheel => proto::FitCapability::ColorWheel,
    }) as i32
}

fn cluster(c: &fit::Cluster) -> proto::FitCluster {
    proto::FitCluster {
        fixtures: c.fixtures.clone(),
        r#type: c.fixture_type.clone(),
        r#where: c.place.clone(),
        height: c.height.map(height),
        depth: c.depth.map(depth),
    }
}

pub(super) fn fit_report(r: &fit::FitReport) -> proto::LightingFit {
    proto::LightingFit {
        venue: r.venue.as_ref().map(|v| proto::FitVenue {
            name: v.name.clone(),
            fixtures: v
                .fixtures
                .iter()
                .map(|f| proto::FitFixture {
                    name: f.name.clone(),
                    r#type: f.fixture_type.clone(),
                    tags: f.tags.clone(),
                    position: f.position.map(vec3),
                    capabilities: f.capabilities.iter().map(capability).collect(),
                })
                .collect(),
            focus_points: v.focus_points.clone(),
            error: v.error.as_ref().map(|e| proto::FitVenueError {
                fixture: e.fixture.clone(),
                reason: e.reason.clone(),
            }),
        }),
        groups: r.groups.iter().map(fit_group).collect(),
        focus_points_wanted: r
            .focus_points_wanted
            .iter()
            .map(|f| proto::FitFocusWanted {
                name: f.name.clone(),
                songs: f.songs.clone(),
            })
            .collect(),
        output: Some(proto::FitOutput {
            unconfigured: u32s(&r.output.unconfigured),
            unpatched: u32s(&r.output.unpatched),
            reachable: r.output.reachable,
            ola_http_port: r.output.ola_http_port.map(u32::from),
        }),
    }
}

fn fit_group(g: &fit::FitGroup) -> proto::FitGroup {
    proto::FitGroup {
        name: g.name.clone(),
        defined: g.defined,
        needs: Some(proto::FitNeeds {
            all_of: g.needs.all_of.clone(),
            any_of: g.needs.any_of.clone(),
            prefer: g.needs.prefer.clone(),
        }),
        tags: g.tags.clone(),
        fixtures: g.fixtures.clone(),
        songs: g.songs.clone(),
        wants: wants(&g.wants),
        suggestion: g.fit.suggestion.as_ref().map(|s| proto::FitSuggestion {
            fixtures: s.fixtures.clone(),
            tags: s.tags.clone(),
            reason: Some(proto::FitReason {
                count: count(s.reason.count),
                r#type: s.reason.fixture_type.clone(),
                r#where: s.reason.place.clone(),
                height: s.reason.height.map(height),
                depth: s.reason.depth.map(depth),
                can: wants(&s.reason.can),
            }),
        }),
        others: g.fit.others.iter().map(cluster).collect(),
        unmet: wants(&g.fit.unmet),
        unmet_together: g.fit.unmet_together,
    }
}

// ---- Groups and the preview ----

pub(super) fn groups(g: &preview_wire::LightingGroups) -> proto::ListLightingGroupsResponse {
    proto::ListLightingGroupsResponse {
        groups: g
            .groups
            .iter()
            .map(|g| proto::LightingGroup {
                name: g.name.clone(),
                fixtures: g.fixtures.clone(),
            })
            .collect(),
        resolved: g.resolved,
    }
}

fn channel_values(values: &HashMap<String, u8>) -> proto::PreviewChannels {
    proto::PreviewChannels {
        values: values
            .iter()
            .map(|(name, value)| (name.clone(), u32::from(*value)))
            .collect(),
    }
}

fn frame(f: &crate::webui::wire::messages::StageValues) -> proto::PreviewFrame {
    proto::PreviewFrame {
        fixtures: f
            .fixtures
            .iter()
            .map(|(name, values)| (name.clone(), channel_values(values)))
            .collect(),
        poses: f
            .poses
            .iter()
            .map(|(name, pose)| {
                (
                    name.clone(),
                    proto::PreviewPose {
                        pan: pose.pan,
                        tilt: pose.tilt,
                        aim: Some(vec3(pose.aim)),
                        floor: pose.floor.map(|[x, y]| proto::LightingVec2 { x, y }),
                    },
                )
            })
            .collect(),
        cells: f
            .cells
            .iter()
            .map(|(fixture, cells)| {
                (
                    fixture.clone(),
                    proto::PreviewCells {
                        cells: cells
                            .iter()
                            .map(|(cell, values)| (cell.clone(), channel_values(values)))
                            .collect(),
                    },
                )
            })
            .collect(),
        strobe_rates: f
            .strobe_rates
            .iter()
            .map(|(name, hz)| (name.clone(), *hz))
            .collect(),
    }
}

pub(super) fn preview(p: &preview_wire::Preview) -> proto::LightingPreview {
    proto::LightingPreview {
        song: p.song.clone(),
        evaluations: p
            .evaluations
            .iter()
            .map(|e| proto::PreviewEvaluation {
                time: e.time,
                frame: Some(frame(&e.frame)),
                active_effects: e
                    .active_effects
                    .iter()
                    .map(|effect| proto::PreviewEffect {
                        id: effect.id.clone(),
                        groups: effect
                            .groups
                            .as_ref()
                            .map(|names| proto::PreviewEffectGroups {
                                names: names.clone(),
                            }),
                        kind: effect.kind.clone(),
                        layer: effect.layer.clone(),
                        elapsed: effect.elapsed,
                        duration: effect.duration,
                        fixtures: effect.fixtures.clone(),
                    })
                    .collect(),
            })
            .collect(),
        untouched: p.untouched.clone(),
        venue_error: p.venue_error.clone(),
    }
}

// ---- Fixture test ----

fn hz_range(r: &HzRange) -> proto::FixtureTestHzRange {
    proto::FixtureTestHzRange {
        min_hz: r.min_hz,
        max_hz: r.max_hz,
    }
}

fn degree_range(r: &DegreeRange) -> proto::FixtureTestDegreeRange {
    proto::FixtureTestDegreeRange {
        min: r.min,
        max: r.max,
    }
}

fn control_support(c: &ControlSupport) -> proto::FixtureTestControlSupport {
    proto::FixtureTestControlSupport {
        color: c.color,
        dimmer: c.dimmer.map(|d| {
            (match d {
                DimmerControl::Channel => proto::FixtureTestDimmer::Channel,
                DimmerControl::Folded => proto::FixtureTestDimmer::Folded,
            }) as i32
        }),
        strobe: c.strobe.as_ref().map(hz_range),
        pan: c.pan.as_ref().map(degree_range),
        tilt: c.tilt.as_ref().map(degree_range),
        levels: c.levels.clone(),
    }
}

pub(super) fn test_options(o: &test_wire::TestOptions) -> proto::FixtureTestOptions {
    proto::FixtureTestOptions {
        fixture_type: o.fixture_type.clone(),
        gdtf: o.gdtf,
        modes: o
            .modes
            .iter()
            .map(|m| proto::FixtureTestMode {
                name: m.name.clone(),
                drivable: m.drivable,
                error: m.error.clone(),
                footprint: m.footprint.map(u32::from),
                controls: m.controls.as_ref().map(control_support),
                channels: m
                    .channels
                    .iter()
                    .map(|c| proto::FixtureTestChannel {
                        offset: u32::from(c.offset),
                        name: c.name.clone(),
                    })
                    .collect(),
            })
            .collect(),
        default_mode: o.default_mode.clone(),
        universes: o
            .universes
            .iter()
            .map(|u| proto::FixtureTestUniverse {
                universe: u32::from(u.universe),
                name: u.name.clone(),
                patched: u.patched,
            })
            .collect(),
        olad: Some(proto::FixtureTestOlad {
            reachable: o.olad.reachable,
            port: o.olad.port.map(u32::from),
        }),
        available: o.available,
        unavailable: o.unavailable.map(|u| {
            (match u {
                test_wire::TestUnavailable::Locked => proto::FixtureTestUnavailable::Locked,
                test_wire::TestUnavailable::Playing => proto::FixtureTestUnavailable::Playing,
                test_wire::TestUnavailable::NoDmx => proto::FixtureTestUnavailable::NoDmx,
                test_wire::TestUnavailable::NoUniverses => {
                    proto::FixtureTestUnavailable::NoUniverses
                }
            }) as i32
        }),
        unavailable_message: o.unavailable_message.clone(),
    }
}

fn frame_channel(c: &FrameChannel) -> proto::FixtureTestFrameChannel {
    proto::FixtureTestFrameChannel {
        offset: u32::from(c.offset),
        address: u32::from(c.address),
        name: c.name.clone(),
        value: u32::from(c.value),
        raw: c.raw,
    }
}

fn test_warning(w: &test_wire::TestWarning) -> proto::FixtureTestWarning {
    use proto::fixture_test_warning::Kind;
    use test_wire::TestWarning;
    let message = |message: &String| proto::FixtureTestMessage {
        message: message.clone(),
    };
    proto::FixtureTestWarning {
        kind: Some(match w {
            TestWarning::VenueOverlap { fixtures, message } => {
                Kind::VenueOverlap(proto::FixtureTestOverlap {
                    fixtures: fixtures.clone(),
                    message: message.clone(),
                })
            }
            TestWarning::PastUniverseEnd { message: m } => Kind::PastUniverseEnd(message(m)),
            TestWarning::OladUnreachable { message: m } => Kind::OladUnreachable(message(m)),
            TestWarning::UniverseUnpatched { message: m } => Kind::UniverseUnpatched(message(m)),
            TestWarning::UnsupportedControl { control, message } => {
                Kind::UnsupportedControl(proto::FixtureTestUnsupportedControl {
                    control: control.clone(),
                    message: message.clone(),
                })
            }
        }),
    }
}

pub(super) fn test_answer(a: &test_wire::TestAnswer) -> proto::FixtureTestAnswer {
    proto::FixtureTestAnswer {
        active: a.active,
        universe: u32::from(a.universe),
        address: u32::from(a.address),
        footprint: u32::from(a.footprint),
        fixture_type: a.fixture_type.clone(),
        mode: a.mode.clone(),
        expires_in_secs: a.expires_in_secs,
        frame: a.frame.iter().map(frame_channel).collect(),
        warnings: a.warnings.iter().map(test_warning).collect(),
    }
}

// ---- Fixture types ----

fn fixture_type_channels(t: &FixtureType) -> proto::FixtureTypeChannels {
    proto::FixtureTypeChannels {
        name: t.name().to_string(),
        channels: t
            .channels()
            .iter()
            .map(|(name, offset)| (name.clone(), u32::from(*offset)))
            .collect(),
        max_strobe_frequency: t.max_strobe_frequency(),
        min_strobe_frequency: t.min_strobe_frequency(),
        strobe_dmx_offset: t.strobe_dmx_offset().map(u32::from),
    }
}

fn file_owner(o: &types_wire::LightingOwner) -> proto::FixtureFileOwner {
    proto::FixtureFileOwner {
        file: o.file.clone(),
        editable: o.editable,
        reason: o.reason.clone(),
    }
}

fn gdtf_card(g: &types_wire::GdtfSummary) -> proto::GdtfCard {
    proto::GdtfCard {
        fixture: g.fixture.clone(),
        manufacturer: g.manufacturer.clone(),
        modes: count(g.modes),
        beam: g.beam.as_ref().map(|b| proto::GdtfBeamBrief {
            beam_type: b.beam_type.clone(),
            angle: b.angle,
        }),
        thumbnail: g.thumbnail.clone(),
        used_by: count(g.used_by),
        in_use: g
            .in_use
            .iter()
            .map(|m| proto::GdtfModeUse {
                mode: m.mode.clone(),
                count: count(m.count),
            })
            .collect(),
    }
}

pub(super) fn fixture_type_info(e: &types_wire::FixtureTypeEntry) -> proto::FixtureTypeInfo {
    proto::FixtureTypeInfo {
        referential: e.referential,
        footprint: e.footprint.flatten().map(u32::from),
        rich: e.rich,
        fixture_type: Some(fixture_type_channels(&e.fixture_type)),
        file: e.file.clone(),
        extension: e.extension.clone(),
        owner: e.owner.as_ref().map(file_owner),
        form_editable: e.form_editable,
        gdtf: e.gdtf.as_ref().and_then(Option::as_ref).map(gdtf_card),
        dsl: e.dsl.clone().flatten(),
    }
}

pub(super) fn fixture_type_list(
    l: &types_wire::FixtureTypeList,
) -> proto::ListFixtureTypesResponse {
    proto::ListFixtureTypesResponse {
        fixture_types: l
            .fixture_types
            .iter()
            .map(|(name, entry)| (name.clone(), fixture_type_info(entry)))
            .collect(),
        errors: l
            .errors
            .iter()
            .map(|e| proto::LightingFileProblem {
                file: e.file.clone(),
                error: e.error.clone(),
            })
            .collect(),
    }
}

pub(super) fn rename(r: &types_wire::Rename) -> proto::FixtureTypeRename {
    proto::FixtureTypeRename {
        from: r.from.clone(),
        to: r.to.clone(),
        venues: r
            .venues
            .iter()
            .map(|v| proto::FixtureTypeRenamedLines {
                venue: v.venue.clone(),
                file: v.file.clone(),
                lines: count(v.lines),
            })
            .collect(),
        lines: count(r.lines),
    }
}

pub(super) fn fixture_type_saved(
    s: &types_wire::FixtureTypeSaved,
) -> proto::SaveFixtureTypeResponse {
    proto::SaveFixtureTypeResponse {
        name: s.name.clone(),
        rename: s.rename.as_ref().map(rename),
        venue_error: s.venue_error.as_ref().map(venue_error),
    }
}

pub(super) fn fixture_type_deleted(
    d: &types_wire::FixtureTypeDeleted,
) -> proto::DeleteFixtureTypeResponse {
    proto::DeleteFixtureTypeResponse {
        name: d.name.clone(),
        archive_removed: d.archive_removed.clone(),
        archive_kept: d.archive_kept,
    }
}

fn gdtf_beam(b: &types_wire::GdtfBeam) -> proto::GdtfBeam {
    proto::GdtfBeam {
        beam_type: b.beam_type.clone(),
        beam_angle: b.beam_angle,
        field_angle: b.field_angle,
        luminous_flux: b.luminous_flux,
        color_temperature: b.color_temperature,
        power: b.power,
    }
}

fn gdtf_mode(m: &types_wire::GdtfMode) -> proto::GdtfModeInfo {
    use proto::gdtf_mode_info::Outcome;
    let outcome = match &m.refused {
        Some(reason) => Outcome::Refused(reason.clone()),
        None => Outcome::Facts(proto::GdtfModeFacts {
            capabilities: m.capabilities.iter().flatten().map(capability).collect(),
            wants: wants(m.wants.as_deref().unwrap_or_default()),
            cells: count(m.cells.unwrap_or_default()),
            strobe_range: m.strobe_range.as_ref().map(|r| proto::GdtfStrobeRange {
                min_hz: r.min_hz,
                max_hz: r.max_hz,
            }),
            channels: m
                .channels
                .iter()
                .flatten()
                .map(|(offset, name)| proto::GdtfChannel {
                    offset: u32::from(*offset),
                    name: name.clone(),
                })
                .collect(),
        }),
    };
    proto::GdtfModeInfo {
        name: m.name.clone(),
        footprint: u32::from(m.footprint),
        outcome: Some(outcome),
    }
}

pub(super) fn fixture_type_gdtf(g: &types_wire::FixtureTypeGdtf) -> proto::FixtureTypeGdtfDetails {
    proto::FixtureTypeGdtfDetails {
        archive: g.archive.clone(),
        rig: g.rig.clone(),
        thumbnail: g.thumbnail.clone(),
        beam: g.beam.as_ref().map(gdtf_beam),
        about: g.about.clone(),
        venues: g
            .venues
            .iter()
            .map(|v| proto::GdtfTypeVenue {
                name: v.name.clone(),
                fixtures: v
                    .fixtures
                    .iter()
                    .map(|f| proto::GdtfTypeVenueFixture {
                        name: f.name.clone(),
                        mode: f.mode.clone(),
                    })
                    .collect(),
            })
            .collect(),
        modes: g.inspection.modes.iter().map(gdtf_mode).collect(),
    }
}

pub(super) fn strobe_curve(c: StrobeCurve) -> i32 {
    (match c {
        StrobeCurve::Period => proto::FixtureStrobeCurve::Period,
        StrobeCurve::Linear => proto::FixtureStrobeCurve::Linear,
        StrobeCurve::Declared => proto::FixtureStrobeCurve::Declared,
    }) as i32
}

/// A curve a request states; none for the automatic one, and for a number
/// that names no curve.
pub(super) fn strobe_curve_of(value: i32) -> Option<StrobeCurve> {
    match proto::FixtureStrobeCurve::try_from(value) {
        Ok(proto::FixtureStrobeCurve::Period) => Some(StrobeCurve::Period),
        Ok(proto::FixtureStrobeCurve::Linear) => Some(StrobeCurve::Linear),
        Ok(proto::FixtureStrobeCurve::Declared) => Some(StrobeCurve::Declared),
        _ => None,
    }
}

pub(super) fn movement(m: &MovementLimits) -> proto::FixtureMovementLimits {
    proto::FixtureMovementLimits {
        max_pan_speed: m.max_pan_speed,
        max_tilt_speed: m.max_tilt_speed,
    }
}

pub(super) fn fixture_settings(s: &types_wire::FixtureSettingsData) -> proto::FixtureTypeSettings {
    proto::FixtureTypeSettings {
        name: s.name.clone(),
        movement: Some(movement(&s.movement)),
        strobe_curve: s.strobe_curve.map(strobe_curve).unwrap_or_default(),
        strobe: s.strobe.as_ref().map(|t| proto::FixtureStrobeTable {
            steps: count(t.steps),
            automatic: strobe_curve(t.automatic),
        }),
        revision: s.version.clone(),
    }
}

pub(super) fn settings_saved(s: &types_wire::SettingsSaved) -> proto::FixtureTypeSettingsSaved {
    proto::FixtureTypeSettingsSaved {
        revision: s.version.clone(),
        rename: s.rename.as_ref().map(rename),
        venue_revisions: s
            .venue_versions
            .iter()
            .map(|(file, version)| (file.clone(), version.clone()))
            .collect(),
        venue_error: s.venue_error.as_ref().map(venue_error),
    }
}
