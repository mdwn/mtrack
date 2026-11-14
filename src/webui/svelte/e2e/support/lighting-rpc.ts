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

// The lighting pages' facts as the player answers them over gRPC, made from
// the shapes the web UI holds of them (`Readiness`, `FitReport`, `Preview`,
// `TestOptions`, `TestAnswer`): the mirror of src/lib/grpc/lighting.ts, so a
// spec states the facts as the page shows them and answers the call with them:
//
//   const rpc = await mockRpc(page);
//   rpc.unary(LightingService.method.getLightingReadiness, readinessReply(report));
//   rpc.unary(LightingService.method.evaluateSongLighting, previewReply(preview));
//   rpc.error(LightingService.method.startFixtureTest, Code.InvalidArgument,
//     "the address must be 1 to 512", refusalReason("address"));
//
// The mock server answers the same calls with the same helpers.
import type { MessageInitShape } from "@bufbuild/protobuf";
import {
  FitCapability,
  FitDepth,
  FitHeight,
  FixtureTestDimmer,
  FixtureTestUnavailable,
  LightingSeverity,
  LightingWant,
  ReadinessCheckKey,
  ReadinessCheckState,
  type FitGroupSchema,
  type FixtureTestAnswerSchema,
  type FixtureTestOptionsSchema,
  type GetLightShowResponseSchema,
  type LightingFitSchema,
  type LightingPreviewSchema,
  type LightingReadinessSchema,
  type LightingWarningSchema,
  type ListLightingGroupsResponseSchema,
  type ReadinessFindingSchema,
  type ReadinessFixSchema,
  type ReadinessSummarySchema,
  type ReadinessWhatSchema,
  type ValidateLightShowResponseSchema,
} from "../../src/gen/player/v1/player_pb";
import type {
  Capability,
  Check,
  CheckKey,
  CheckState,
  Cluster,
  Depth,
  Finding,
  FindingWhat,
  FitGroup,
  FitReport,
  Fix,
  Height,
  LightingGroup,
  LightingValidation,
  Preview,
  Readiness,
  Severity,
  Summary,
  TestAnswer,
  TestOptions,
  TestUnavailable,
  Vec3,
  Want,
  Warning,
} from "../../src/gen/rules/mtrack_rules";

/** The metadata of a refused fixture test: why, in a word (`playing`,
 *  `no_dmx`, `universe`, `address`, `controls`, `type`, `mode`, `raw`,
 *  `files`), under the key the server uses. */
export function refusalReason(reason: string): HeadersInit {
  return { "x-refusal-reason": reason };
}

const SEVERITIES: Record<Severity, LightingSeverity> = {
  blocked: LightingSeverity.BLOCKED,
  attention: LightingSeverity.ATTENTION,
  unknown: LightingSeverity.UNKNOWN,
  note: LightingSeverity.NOTE,
};

const WANTS: Record<Want, LightingWant> = {
  move: LightingWant.MOVE,
  color: LightingWant.COLOR,
  strobe: LightingWant.STROBE,
  cells: LightingWant.CELLS,
  dimmer: LightingWant.DIMMER,
};

const CHECK_KEYS: Record<CheckKey, ReadinessCheckKey> = {
  fixtures: ReadinessCheckKey.FIXTURES,
  venue: ReadinessCheckKey.VENUE,
  groups: ReadinessCheckKey.GROUPS,
  shows: ReadinessCheckKey.SHOWS,
  output: ReadinessCheckKey.OUTPUT,
};

const CHECK_STATES: Record<CheckState, ReadinessCheckState> = {
  ready: ReadinessCheckState.READY,
  attention: ReadinessCheckState.ATTENTION,
  blocked: ReadinessCheckState.BLOCKED,
  unknown: ReadinessCheckState.UNKNOWN,
};

const CAPABILITIES: Record<Capability, FitCapability> = {
  color: FitCapability.COLOR,
  pan_tilt: FitCapability.PAN_TILT,
  strobe: FitCapability.STROBE,
  cells: FitCapability.CELLS,
  dimmer: FitCapability.DIMMER,
  white: FitCapability.WHITE,
  zoom: FitCapability.ZOOM,
  focus: FitCapability.FOCUS,
  gobo: FitCapability.GOBO,
  color_wheel: FitCapability.COLOR_WHEEL,
};

const HEIGHTS: Record<Height, FitHeight> = {
  deck: FitHeight.DECK,
  low: FitHeight.LOW,
  truss: FitHeight.TRUSS,
};

const DEPTHS: Record<Depth, FitDepth> = {
  downstage: FitDepth.DOWNSTAGE,
  mid: FitDepth.MID,
  upstage: FitDepth.UPSTAGE,
};

const UNAVAILABLE: Record<TestUnavailable, FixtureTestUnavailable> = {
  locked: FixtureTestUnavailable.LOCKED,
  playing: FixtureTestUnavailable.PLAYING,
  no_dmx: FixtureTestUnavailable.NO_DMX,
  no_universes: FixtureTestUnavailable.NO_UNIVERSES,
};

const vec3 = (v: Vec3) => ({ x: v[0], y: v[1], z: v[2] });

// ---- Light show files and groups ----

/** A light show file's text. */
export function lightShowReply(
  source: string,
): MessageInitShape<typeof GetLightShowResponseSchema> {
  return { source };
}

/** A validation: the problems, when there are some. */
export function validationReply(
  validation: LightingValidation,
): MessageInitShape<typeof ValidateLightShowResponseSchema> {
  return { valid: validation.valid, errors: validation.errors ?? [] };
}

/** The groups a cue can target. */
export function groupsReply(
  groups: LightingGroup[],
  resolved?: boolean,
): MessageInitShape<typeof ListLightingGroupsResponseSchema> {
  return { groups, resolved };
}

// ---- Readiness ----

function warning(w: Warning): MessageInitShape<typeof LightingWarningSchema> {
  return { kind: w.kind, message: w.message, severity: SEVERITIES[w.severity] };
}

function summary(s: Summary): MessageInitShape<typeof ReadinessSummarySchema> {
  switch (s.kind) {
    case "blocked":
    case "showsBlocked":
    case "attention":
    case "showsReady":
    case "typesReady":
    case "groupsReady":
    case "outputReady":
      return { kind: { case: s.kind, value: { count: s.count } } };
    case "unknown":
    case "noDmx":
    case "needsVenue":
    case "showsNone":
    case "groupsNone":
      return { kind: { case: s.kind, value: {} } };
    case "venueAllPlaced":
    case "venueSomeUnplaced":
      return {
        kind: {
          case: s.kind,
          value: {
            venue: s.venue,
            fixtures: s.fixtures,
            unplaced: s.unplaced,
          },
        },
      };
  }
}

function what(w: FindingWhat): MessageInitShape<typeof ReadinessWhatSchema> {
  switch (w.kind) {
    case "showError":
      return {
        kind: { case: "showError", value: { song: w.song, detail: w.detail } },
      };
    case "lint":
    case "patch":
      return { kind: { case: w.kind, value: { detail: w.detail } } };
    case "noDmx":
    case "noVenue":
    case "oladUnreachable":
      return { kind: { case: w.kind, value: {} } };
    case "typeUnresolved":
      return {
        kind: {
          case: "typeUnresolved",
          value: { fixture: w.fixture, type: w.type, reason: w.reason },
        },
      };
    case "venueFailed":
      return {
        kind: {
          case: "venueFailed",
          value: { venue: w.venue, fixture: w.fixture, reason: w.reason },
        },
      };
    case "emptyVenue":
      return { kind: { case: "emptyVenue", value: { venue: w.venue } } };
    case "groupEmpty":
      return {
        kind: {
          case: "groupEmpty",
          value: { group: w.group, venue: w.venue, songs: w.songs },
        },
      };
    case "universeUnconfigured":
    case "universeUnpatched":
      return {
        kind: { case: w.kind, value: { universe: w.universe } },
      };
  }
}

function fix(f: Fix): MessageInitShape<typeof ReadinessFixSchema> {
  switch (f.kind) {
    case "song":
      return { kind: { case: "song", value: { song: f.song } } };
    case "fit":
      return {
        kind: { case: "fit", value: { group: f.group ?? undefined } },
      };
    case "fixtures":
    case "venues":
    case "config":
    case "chooseVenue":
      return { kind: { case: f.kind, value: {} } };
  }
}

function finding(f: Finding): MessageInitShape<typeof ReadinessFindingSchema> {
  return {
    severity: SEVERITIES[f.severity],
    what: what(f.what),
    fix: fix(f.fix),
    song: f.song ?? undefined,
  };
}

function check(c: Check) {
  return {
    key: CHECK_KEYS[c.key],
    state: CHECK_STATES[c.state],
    summary: c.summary.map(summary),
    findings: c.findings.map(finding),
  };
}

/** The Lighting overview's facts and checks. */
export function readinessReply(
  r: Readiness,
): MessageInitShape<typeof LightingReadinessSchema> {
  return {
    dmx: r.dmx,
    venue: r.venue
      ? {
          name: r.venue.name,
          fixtures: r.venue.fixtures,
          placed: r.venue.placed,
          focusPoints: r.venue.focus_points,
        }
      : undefined,
    venueError: r.venue_error ?? undefined,
    patchWarnings: r.patch_warnings.map(warning),
    fixtureTypes: {
      inUse: r.fixture_types.in_use,
      unresolved: r.fixture_types.unresolved,
      library: r.fixture_types.library,
    },
    groups: r.groups,
    shows: r.shows.map((s) => ({
      song: s.song,
      files: s.files,
      error: s.error,
      warnings: s.warnings.map(warning),
    })),
    output: {
      universes: r.output.universes,
      unconfigured: r.output.unconfigured,
      olad: r.output.olad ?? undefined,
    },
    checks: r.checks.map(check),
  };
}

// ---- Fit ----

function cluster(c: Cluster) {
  return {
    fixtures: c.fixtures,
    type: c.type,
    where: c.where ?? undefined,
    height: c.height ? HEIGHTS[c.height] : undefined,
    depth: c.depth ? DEPTHS[c.depth] : undefined,
  };
}

function fitGroup(g: FitGroup): MessageInitShape<typeof FitGroupSchema> {
  return {
    name: g.name,
    defined: g.defined,
    needs: {
      allOf: g.needs.all_of,
      anyOf: g.needs.any_of,
      prefer: g.needs.prefer,
    },
    tags: g.tags,
    fixtures: g.fixtures,
    songs: g.songs,
    wants: g.wants.map((w) => WANTS[w]),
    suggestion: g.suggestion
      ? {
          fixtures: g.suggestion.fixtures,
          tags: g.suggestion.tags,
          reason: {
            count: g.suggestion.reason.count,
            type: g.suggestion.reason.type,
            where: g.suggestion.reason.where ?? undefined,
            height: g.suggestion.reason.height
              ? HEIGHTS[g.suggestion.reason.height]
              : undefined,
            depth: g.suggestion.reason.depth
              ? DEPTHS[g.suggestion.reason.depth]
              : undefined,
            can: g.suggestion.reason.can.map((w) => WANTS[w]),
          },
        }
      : undefined,
    others: g.others.map(cluster),
    unmet: g.unmet.map((w) => WANTS[w]),
    unmetTogether: g.unmet_together,
  };
}

/** The Fit shows page's facts. */
export function fitReply(
  r: FitReport,
): MessageInitShape<typeof LightingFitSchema> {
  return {
    venue: r.venue
      ? {
          name: r.venue.name,
          fixtures: r.venue.fixtures.map((f) => ({
            name: f.name,
            type: f.type,
            tags: f.tags,
            position: f.position ? vec3(f.position) : undefined,
            capabilities: f.capabilities.map((c) => CAPABILITIES[c]),
          })),
          focusPoints: r.venue.focus_points,
          error: r.venue.error
            ? {
                fixture: r.venue.error.fixture ?? undefined,
                reason: r.venue.error.reason,
              }
            : undefined,
        }
      : undefined,
    groups: r.groups.map(fitGroup),
    focusPointsWanted: r.focus_points_wanted,
    output: {
      unconfigured: r.output.unconfigured,
      unpatched: r.output.unpatched,
      reachable: r.output.reachable ?? undefined,
      olaHttpPort: r.output.ola_http_port ?? undefined,
    },
  };
}

// ---- The preview ----

/** A song's shows evaluated at the asked times. */
export function previewReply(
  p: Preview,
): MessageInitShape<typeof LightingPreviewSchema> {
  const channels = (c: Record<string, number>) => ({ values: c });
  return {
    song: p.song,
    evaluations: p.evaluations.map((e) => ({
      time: e.time,
      frame: {
        fixtures: Object.fromEntries(
          Object.entries(e.fixtures).map(([k, c]) => [k, channels(c)]),
        ),
        poses: Object.fromEntries(
          Object.entries(e.poses).map(([k, pose]) => [
            k,
            {
              pan: pose.pan,
              tilt: pose.tilt,
              aim: vec3(pose.aim),
              floor: pose.floor
                ? { x: pose.floor[0], y: pose.floor[1] }
                : undefined,
            },
          ]),
        ),
        cells: Object.fromEntries(
          Object.entries(e.cells).map(([fixture, cells]) => [
            fixture,
            {
              cells: Object.fromEntries(
                Object.entries(cells).map(([cell, c]) => [cell, channels(c)]),
              ),
            },
          ]),
        ),
        strobeRates: e.strobe_rates,
      },
      activeEffects: e.active_effects.map((a) => ({
        id: a.id,
        groups: a.groups ? { names: a.groups } : undefined,
        kind: a.kind,
        layer: a.layer,
        elapsed: a.elapsed,
        duration: a.duration,
        fixtures: a.fixtures,
      })),
    })),
    untouched: p.untouched,
    venueError: p.venue_error ?? undefined,
  };
}

// ---- The fixture test ----

/** What a test of a fixture type can do. */
export function testOptionsReply(
  o: TestOptions,
): MessageInitShape<typeof FixtureTestOptionsSchema> {
  return {
    fixtureType: o.fixture_type,
    gdtf: o.gdtf,
    modes: o.modes.map((m) => ({
      name: m.name ?? undefined,
      drivable: m.drivable,
      error: m.error ?? undefined,
      footprint: m.footprint ?? undefined,
      controls: m.controls
        ? {
            color: m.controls.color,
            dimmer: m.controls.dimmer
              ? m.controls.dimmer === "channel"
                ? FixtureTestDimmer.CHANNEL
                : FixtureTestDimmer.FOLDED
              : undefined,
            strobe: m.controls.strobe
              ? {
                  minHz: m.controls.strobe.min_hz,
                  maxHz: m.controls.strobe.max_hz,
                }
              : undefined,
            pan: m.controls.pan ?? undefined,
            tilt: m.controls.tilt ?? undefined,
            levels: m.controls.levels,
          }
        : undefined,
      channels: m.channels,
    })),
    defaultMode: o.default_mode ?? undefined,
    universes: o.universes.map((u) => ({
      universe: u.universe,
      name: u.name ?? undefined,
      patched: u.patched ?? undefined,
    })),
    olad: {
      reachable: o.olad.reachable ?? undefined,
      port: o.olad.port ?? undefined,
    },
    available: o.available,
    unavailable: o.unavailable ? UNAVAILABLE[o.unavailable] : undefined,
    unavailableMessage: o.unavailable_message ?? undefined,
  };
}

/** What a test sent, and what it should be told. */
export function testAnswerReply(
  a: TestAnswer,
): MessageInitShape<typeof FixtureTestAnswerSchema> {
  return {
    active: a.active,
    universe: a.universe,
    address: a.address,
    footprint: a.footprint,
    fixtureType: a.fixture_type,
    mode: a.mode ?? undefined,
    expiresInSecs: a.expires_in_secs,
    frame: a.frame,
    warnings: a.warnings.map((w) => {
      switch (w.kind) {
        case "venue_overlap":
          return {
            kind: {
              case: "venueOverlap" as const,
              value: { fixtures: w.fixtures, message: w.message },
            },
          };
        case "past_universe_end":
          return {
            kind: {
              case: "pastUniverseEnd" as const,
              value: { message: w.message },
            },
          };
        case "olad_unreachable":
          return {
            kind: {
              case: "oladUnreachable" as const,
              value: { message: w.message },
            },
          };
        case "universe_unpatched":
          return {
            kind: {
              case: "universeUnpatched" as const,
              value: { message: w.message },
            },
          };
        case "unsupported_control":
          return {
            kind: {
              case: "unsupportedControl" as const,
              value: { control: w.control, message: w.message },
            },
          };
      }
    }),
  };
}
