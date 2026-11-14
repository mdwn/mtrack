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

// The lighting pages' calls over gRPC (`LightingService`): the light show
// files, their validation and evaluation, the overview's readiness and fit,
// the groups a cue can target and the fixture test, mapped into the shapes
// the components hold (`Readiness`, `FitReport`, `Preview`, `TestOptions`,
// `TestAnswer`) and the errors they tell apart. Only the mapping lives here,
// so a unit test can run it without a page; the calls are in
// src/lib/api/config.ts, src/lib/lighting/preview.ts and fixtureTest.ts.

import type { MessageInitShape } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import {
  FitCapability,
  FitDepth,
  FitHeight,
  FixtureStrobeCurve,
  FixtureTestDimmer,
  FixtureTestUnavailable,
  FixtureTypeFile,
  LightingSeverity,
  LightingWant,
  ReadinessCheckKey,
  ReadinessCheckState,
  type FitCluster,
  type FitGroup as FitGroupMessage,
  type FixtureTestAnswer,
  type FixtureTestControlSupport,
  type FixtureTestControlsSchema,
  type FixtureTestOptions,
  type FixtureTypeGdtfDetails,
  type FixtureTypeInfo,
  type FixtureTypeRename,
  type FixtureTypeSettings,
  type FixtureTypeSettingsSaved,
  type ListFixtureTypesResponse,
  type SaveFixtureTypeRequestSchema,
  type SaveFixtureTypeSettingsRequestSchema,
  type SaveFixtureTypeResponse,
  type DeleteFixtureTypeResponse,
  type AddVenueAimPointsResponse,
  type LightingFixture,
  type LightingVenue,
  type ListVenuesResponse,
  type MvrExportSummary as MvrExportSummaryMessage,
  type SaveVenueRequestSchema,
  type SaveVenueResponse,
  type VenueDetail as VenueDetailMessage,
  type VenuePatch as VenuePatchMessage,
  type VenueScene as VenueSceneMessage,
  type LightingVenueError,
  type LightingFit,
  type LightingPreview,
  type LightingReadiness,
  type LightingVec3,
  type LightingWarning,
  type ListLightingGroupsResponse,
  type PreviewChannels,
  type PreviewEffect as PreviewEffectMessage,
  type ReadinessFinding,
  type ReadinessFix,
  type ReadinessSummary,
  type ReadinessWhat,
  type StartFixtureTestRequestSchema,
} from "../../gen/player/v1/player_pb";
import type {
  AimPoints,
  Capability,
  Check,
  CheckKey,
  CheckState,
  Cluster,
  ControlSupport,
  Depth,
  Finding,
  FindingWhat,
  FitGroup,
  FitReport,
  Fix,
  FixtureSettingsData,
  FixtureType,
  FixtureTypeDeleted,
  FixtureTypeEntry,
  FixtureTypeGdtf,
  FixtureTypeList,
  FixtureTypeSaved,
  GdtfMode,
  Height,
  LightingGroup,
  MvrExportSummary,
  Preview,
  PreviewEffect,
  PreviewEvaluation,
  Readiness,
  Rename,
  SettingsRequest,
  SettingsSaved,
  Severity,
  StrobeCurve,
  Summary,
  TestAnswer,
  TestMode,
  TestOptions,
  TestRequest,
  TestUnavailable,
  TestWarning,
  Vec3,
  Venue,
  VenueDetail,
  VenueError,
  VenueList,
  VenuePatch,
  VenueSaved,
  VenueScene,
  Want,
  Warning,
} from "../../gen/rules/mtrack_rules";
import { BadAnswerError } from "../lighting/answer";
import { callError } from "./library";

// ---- Enums ----

const SEVERITIES: Record<LightingSeverity, Severity | undefined> = {
  [LightingSeverity.UNSPECIFIED]: undefined,
  [LightingSeverity.BLOCKED]: "blocked",
  [LightingSeverity.ATTENTION]: "attention",
  [LightingSeverity.UNKNOWN]: "unknown",
  [LightingSeverity.NOTE]: "note",
};

const WANTS: Record<LightingWant, Want | undefined> = {
  [LightingWant.UNSPECIFIED]: undefined,
  [LightingWant.MOVE]: "move",
  [LightingWant.COLOR]: "color",
  [LightingWant.STROBE]: "strobe",
  [LightingWant.CELLS]: "cells",
  [LightingWant.DIMMER]: "dimmer",
};

const CHECK_KEYS: Record<ReadinessCheckKey, CheckKey | undefined> = {
  [ReadinessCheckKey.UNSPECIFIED]: undefined,
  [ReadinessCheckKey.FIXTURES]: "fixtures",
  [ReadinessCheckKey.VENUE]: "venue",
  [ReadinessCheckKey.GROUPS]: "groups",
  [ReadinessCheckKey.SHOWS]: "shows",
  [ReadinessCheckKey.OUTPUT]: "output",
};

const CHECK_STATES: Record<ReadinessCheckState, CheckState | undefined> = {
  [ReadinessCheckState.UNSPECIFIED]: undefined,
  [ReadinessCheckState.READY]: "ready",
  [ReadinessCheckState.ATTENTION]: "attention",
  [ReadinessCheckState.BLOCKED]: "blocked",
  [ReadinessCheckState.UNKNOWN]: "unknown",
};

const CAPABILITIES: Record<FitCapability, Capability | undefined> = {
  [FitCapability.UNSPECIFIED]: undefined,
  [FitCapability.COLOR]: "color",
  [FitCapability.PAN_TILT]: "pan_tilt",
  [FitCapability.STROBE]: "strobe",
  [FitCapability.CELLS]: "cells",
  [FitCapability.DIMMER]: "dimmer",
  [FitCapability.WHITE]: "white",
  [FitCapability.ZOOM]: "zoom",
  [FitCapability.FOCUS]: "focus",
  [FitCapability.GOBO]: "gobo",
  [FitCapability.COLOR_WHEEL]: "color_wheel",
};

const HEIGHTS: Record<FitHeight, Height | undefined> = {
  [FitHeight.UNSPECIFIED]: undefined,
  [FitHeight.DECK]: "deck",
  [FitHeight.LOW]: "low",
  [FitHeight.TRUSS]: "truss",
};

const DEPTHS: Record<FitDepth, Depth | undefined> = {
  [FitDepth.UNSPECIFIED]: undefined,
  [FitDepth.DOWNSTAGE]: "downstage",
  [FitDepth.MID]: "mid",
  [FitDepth.UPSTAGE]: "upstage",
};

const UNAVAILABLE: Record<FixtureTestUnavailable, TestUnavailable | undefined> =
  {
    [FixtureTestUnavailable.UNSPECIFIED]: undefined,
    [FixtureTestUnavailable.LOCKED]: "locked",
    [FixtureTestUnavailable.PLAYING]: "playing",
    [FixtureTestUnavailable.NO_DMX]: "no_dmx",
    [FixtureTestUnavailable.NO_UNIVERSES]: "no_universes",
  };

/** The wire name of an enum value, or an error naming what was not
 *  understood: a value this page does not know is a newer server's. */
function named<K extends number, V>(
  table: Record<K, V | undefined>,
  value: K,
  what: string,
): V {
  const name = table[value];
  if (name === undefined) {
    throw new BadAnswerError(what);
  }
  return name;
}

const severity = (s: LightingSeverity): Severity =>
  named(SEVERITIES, s, "a severity");
const want = (w: LightingWant): Want => named(WANTS, w, "a want");

const vec3 = (v: LightingVec3): Vec3 => [v.x, v.y, v.z];

// ---- Groups ----

/** The groups a cue can target. */
export function mapGroups(r: ListLightingGroupsResponse): LightingGroup[] {
  return r.groups.map((g) => ({ name: g.name, fixtures: [...g.fixtures] }));
}

// ---- Readiness ----

function warning(w: LightingWarning): Warning {
  return {
    kind: w.kind,
    message: w.message,
    severity: severity(w.severity),
  };
}

function summary(s: ReadinessSummary): Summary {
  const k = s.kind;
  switch (k.case) {
    case "blocked":
    case "showsBlocked":
    case "attention":
    case "showsReady":
    case "typesReady":
    case "groupsReady":
    case "outputReady":
      return { kind: k.case, count: k.value.count };
    case "unknown":
    case "noDmx":
    case "needsVenue":
    case "showsNone":
    case "groupsNone":
      return { kind: k.case };
    case "venueAllPlaced":
    case "venueSomeUnplaced":
      return {
        kind: k.case,
        venue: k.value.venue,
        fixtures: k.value.fixtures,
        unplaced: k.value.unplaced,
      };
    default:
      throw new BadAnswerError("a summary");
  }
}

function what(w: ReadinessWhat): FindingWhat {
  const k = w.kind;
  switch (k.case) {
    case "showError":
      return { kind: "showError", song: k.value.song, detail: k.value.detail };
    case "lint":
    case "patch":
      return { kind: k.case, detail: k.value.detail };
    case "noDmx":
    case "noVenue":
    case "oladUnreachable":
      return { kind: k.case };
    case "typeUnresolved":
      return {
        kind: "typeUnresolved",
        fixture: k.value.fixture,
        type: k.value.type,
        reason: k.value.reason,
      };
    case "venueFailed":
      return {
        kind: "venueFailed",
        venue: k.value.venue,
        fixture: k.value.fixture,
        reason: k.value.reason,
      };
    case "emptyVenue":
      return { kind: "emptyVenue", venue: k.value.venue };
    case "groupEmpty":
      return {
        kind: "groupEmpty",
        group: k.value.group,
        venue: k.value.venue,
        songs: k.value.songs,
      };
    case "universeUnconfigured":
    case "universeUnpatched":
      return { kind: k.case, universe: k.value.universe };
    default:
      throw new BadAnswerError("a finding");
  }
}

function fix(f: ReadinessFix): Fix {
  const k = f.kind;
  switch (k.case) {
    case "song":
      return { kind: "song", song: k.value.song };
    case "fit":
      return { kind: "fit", group: k.value.group ?? null };
    case "fixtures":
    case "venues":
    case "config":
    case "chooseVenue":
      return { kind: k.case };
    default:
      throw new BadAnswerError("a fix");
  }
}

function finding(f: ReadinessFinding): Finding {
  if (!f.what || !f.fix) {
    throw new BadAnswerError("a finding");
  }
  return {
    severity: severity(f.severity),
    what: what(f.what),
    fix: fix(f.fix),
    song: f.song ?? null,
  };
}

/** The Lighting overview's five checks, and the facts behind them. */
export function mapReadiness(r: LightingReadiness): Readiness {
  const types = r.fixtureTypes;
  const output = r.output;
  if (!types || !output) {
    throw new BadAnswerError("the readiness checks");
  }
  return {
    dmx: r.dmx,
    venue: r.venue
      ? {
          name: r.venue.name,
          fixtures: r.venue.fixtures,
          placed: r.venue.placed,
          focus_points: [...r.venue.focusPoints],
        }
      : null,
    venue_error: r.venueError
      ? {
          venue: r.venueError.venue,
          fixture: r.venueError.fixture,
          reason: r.venueError.reason,
        }
      : null,
    patch_warnings: r.patchWarnings.map(warning),
    fixture_types: {
      in_use: [...types.inUse],
      unresolved: types.unresolved.map((u) => ({
        fixture: u.fixture,
        type: u.type,
        reason: u.reason,
      })),
      library: types.library.map((f) => ({
        kind: f.kind,
        file: f.file,
        message: f.message,
      })),
    },
    groups: r.groups.map((g) => ({
      name: g.name,
      fixtures: g.fixtures,
      songs: [...g.songs],
    })),
    // A show without an error has no `error` key.
    shows: r.shows.map((s) => ({
      song: s.song,
      files: [...s.files],
      ...(s.error !== undefined ? { error: s.error } : {}),
      warnings: s.warnings.map(warning),
    })),
    output: {
      universes: [...output.universes],
      unconfigured: [...output.unconfigured],
      olad: output.olad
        ? {
            reachable: output.olad.reachable,
            unpatched: [...output.olad.unpatched],
          }
        : null,
    },
    checks: r.checks.map(
      (c): Check => ({
        key: named(CHECK_KEYS, c.key, "a check"),
        state: named(CHECK_STATES, c.state, "a check"),
        summary: c.summary.map(summary),
        findings: c.findings.map(finding),
      }),
    ),
  };
}

// ---- Fit ----

function wants(ws: LightingWant[]): Want[] {
  return ws.map(want);
}

function cluster(c: FitCluster): Cluster {
  return {
    fixtures: [...c.fixtures],
    type: c.type,
    where: c.where ?? null,
    height:
      c.height === undefined ? null : named(HEIGHTS, c.height, "a height"),
    depth: c.depth === undefined ? null : named(DEPTHS, c.depth, "a depth"),
  };
}

function fitGroup(g: FitGroupMessage): FitGroup {
  const s = g.suggestion;
  return {
    name: g.name,
    defined: g.defined,
    needs: {
      all_of: [...(g.needs?.allOf ?? [])],
      any_of: [...(g.needs?.anyOf ?? [])],
      prefer: [...(g.needs?.prefer ?? [])],
    },
    tags: [...g.tags],
    fixtures: [...g.fixtures],
    songs: [...g.songs],
    wants: wants(g.wants),
    suggestion: s
      ? {
          fixtures: [...s.fixtures],
          tags: [...s.tags],
          reason: {
            count: s.reason?.count ?? 0,
            type: s.reason?.type ?? "",
            where: s.reason?.where ?? null,
            height:
              s.reason?.height === undefined
                ? null
                : named(HEIGHTS, s.reason.height, "a height"),
            depth:
              s.reason?.depth === undefined
                ? null
                : named(DEPTHS, s.reason.depth, "a depth"),
            can: wants(s.reason?.can ?? []),
          },
        }
      : null,
    others: g.others.map(cluster),
    unmet: wants(g.unmet),
    unmet_together: g.unmetTogether,
  };
}

/** The facts and suggestions behind the Fit shows page. */
export function mapFit(r: LightingFit): FitReport {
  const output = r.output;
  if (!output) {
    throw new BadAnswerError("the fit");
  }
  return {
    venue: r.venue
      ? {
          name: r.venue.name,
          fixtures: r.venue.fixtures.map((f) => ({
            name: f.name,
            type: f.type,
            tags: [...f.tags],
            position: f.position ? vec3(f.position) : null,
            capabilities: f.capabilities.map((c) =>
              named(CAPABILITIES, c, "a capability"),
            ),
          })),
          focus_points: [...r.venue.focusPoints],
          error: r.venue.error
            ? {
                fixture: r.venue.error.fixture ?? null,
                reason: r.venue.error.reason,
              }
            : null,
        }
      : null,
    groups: r.groups.map(fitGroup),
    focus_points_wanted: r.focusPointsWanted.map((f) => ({
      name: f.name,
      songs: [...f.songs],
    })),
    output: {
      unconfigured: [...output.unconfigured],
      unpatched: [...output.unpatched],
      reachable: output.reachable ?? null,
      ola_http_port: output.olaHttpPort ?? null,
    },
  };
}

// ---- The preview ----

function channels(c: PreviewChannels): Record<string, number> {
  return { ...c.values };
}

function previewEffect(e: PreviewEffectMessage): PreviewEffect {
  return {
    id: e.id,
    groups: e.groups ? [...e.groups.names] : null,
    kind: e.kind,
    layer: e.layer,
    elapsed: e.elapsed,
    duration: e.duration,
    fixtures: [...e.fixtures],
  };
}

/** A song's shows evaluated at the asked times, each evaluation in the
 *  shape of a live frame of the stage stream. */
export function mapPreview(p: LightingPreview): Preview {
  return {
    song: p.song,
    evaluations: p.evaluations.map((e): PreviewEvaluation => {
      const frame = e.frame;
      return {
        time: e.time,
        fixtures: Object.fromEntries(
          Object.entries(frame?.fixtures ?? {}).map(([name, c]) => [
            name,
            channels(c),
          ]),
        ),
        poses: Object.fromEntries(
          Object.entries(frame?.poses ?? {}).map(([name, pose]) => [
            name,
            {
              pan: pose.pan,
              tilt: pose.tilt,
              aim: pose.aim ? vec3(pose.aim) : [0, 0, 0],
              floor: pose.floor ? [pose.floor.x, pose.floor.y] : null,
            },
          ]),
        ),
        cells: Object.fromEntries(
          Object.entries(frame?.cells ?? {}).map(([fixture, cells]) => [
            fixture,
            Object.fromEntries(
              Object.entries(cells.cells).map(([cell, c]) => [
                cell,
                channels(c),
              ]),
            ),
          ]),
        ),
        strobe_rates: { ...(frame?.strobeRates ?? {}) },
        active_effects: e.activeEffects.map(previewEffect),
      };
    }),
    untouched: [...p.untouched],
    venue_error: p.venueError ?? null,
  };
}

// ---- The fixture test ----

function controlSupport(c: FixtureTestControlSupport): ControlSupport {
  return {
    color: c.color,
    dimmer:
      c.dimmer === undefined
        ? null
        : c.dimmer === FixtureTestDimmer.CHANNEL
          ? "channel"
          : "folded",
    strobe: c.strobe
      ? { min_hz: c.strobe.minHz, max_hz: c.strobe.maxHz }
      : null,
    pan: c.pan ? { min: c.pan.min, max: c.pan.max } : null,
    tilt: c.tilt ? { min: c.tilt.min, max: c.tilt.max } : null,
    levels: [...c.levels],
  };
}

/** What a test of a fixture type can do. */
export function mapTestOptions(o: FixtureTestOptions): TestOptions {
  return {
    fixture_type: o.fixtureType,
    gdtf: o.gdtf,
    modes: o.modes.map(
      (m): TestMode => ({
        name: m.name ?? null,
        drivable: m.drivable,
        error: m.error ?? null,
        footprint: m.footprint ?? null,
        controls: m.controls ? controlSupport(m.controls) : null,
        channels: m.channels.map((c) => ({ offset: c.offset, name: c.name })),
      }),
    ),
    default_mode: o.defaultMode ?? null,
    universes: o.universes.map((u) => ({
      universe: u.universe,
      name: u.name ?? null,
      patched: u.patched ?? null,
    })),
    olad: {
      reachable: o.olad?.reachable ?? null,
      port: o.olad?.port ?? null,
    },
    available: o.available,
    unavailable:
      o.unavailable === undefined
        ? null
        : named(UNAVAILABLE, o.unavailable, "why a test cannot run"),
    unavailable_message: o.unavailableMessage ?? null,
  };
}

function testWarning(w: FixtureTestAnswer["warnings"][number]): TestWarning {
  const k = w.kind;
  switch (k.case) {
    case "venueOverlap":
      return {
        kind: "venue_overlap",
        fixtures: [...k.value.fixtures],
        message: k.value.message,
      };
    case "pastUniverseEnd":
      return { kind: "past_universe_end", message: k.value.message };
    case "oladUnreachable":
      return { kind: "olad_unreachable", message: k.value.message };
    case "universeUnpatched":
      return { kind: "universe_unpatched", message: k.value.message };
    case "unsupportedControl":
      return {
        kind: "unsupported_control",
        control: k.value.control,
        message: k.value.message,
      };
    default:
      throw new BadAnswerError("a test warning");
  }
}

/** What a test sent, and what it should be told. */
export function mapTestAnswer(a: FixtureTestAnswer): TestAnswer {
  return {
    active: a.active,
    universe: a.universe,
    address: a.address,
    footprint: a.footprint,
    fixture_type: a.fixtureType,
    mode: a.mode ?? null,
    expires_in_secs: a.expiresInSecs,
    frame: a.frame.map((c) => ({
      offset: c.offset,
      address: c.address,
      name: c.name,
      value: c.value,
      raw: c.raw,
    })),
    warnings: a.warnings.map(testWarning),
  };
}

/** The controls a test sends, split into the typed ones and the channel
 *  levels (any other name): `color`, `dimmer`, `strobe` (Hz, or `null` for
 *  off), `pan` and `tilt` are controls of their own. */
function testControls(
  controls: NonNullable<TestRequest["controls"]>,
): MessageInitShape<typeof FixtureTestControlsSchema> {
  const out: {
    color?: string;
    dimmer?: number;
    strobe?: { hertz?: number };
    pan?: number;
    tilt?: number;
    levels: Record<string, number>;
  } = { levels: {} };
  for (const [key, value] of Object.entries(controls)) {
    switch (key) {
      case "color":
        if (typeof value === "string") out.color = value;
        break;
      case "dimmer":
        if (typeof value === "number") out.dimmer = value;
        break;
      case "strobe":
        out.strobe = typeof value === "number" ? { hertz: value } : {};
        break;
      case "pan":
        if (typeof value === "number") out.pan = value;
        break;
      case "tilt":
        if (typeof value === "number") out.tilt = value;
        break;
      default:
        if (typeof value === "number") out.levels[key] = value;
    }
  }
  return out;
}

/** The call for a test the page holds. */
export function startFixtureTestRequest(
  body: TestRequest,
  dir?: string,
  venuesDir?: string,
): MessageInitShape<typeof StartFixtureTestRequestSchema> {
  return {
    fixtureType: body.fixture_type,
    mode: body.mode ?? undefined,
    universe: body.universe,
    address: body.address,
    controls: testControls(body.controls ?? {}),
    raw: Object.fromEntries(
      Object.entries(body.raw ?? {}).map(([k, v]) => [k, v]),
    ),
    dir: dir || undefined,
    venuesDir: venuesDir || undefined,
  };
}

// ---- Fixture types ----

const STROBE_CURVES: Record<FixtureStrobeCurve, StrobeCurve | null> = {
  [FixtureStrobeCurve.UNSPECIFIED]: null,
  [FixtureStrobeCurve.PERIOD]: "period",
  [FixtureStrobeCurve.LINEAR]: "linear",
  [FixtureStrobeCurve.DECLARED]: "declared",
};

const STROBE_CURVE_VALUES: Record<StrobeCurve, FixtureStrobeCurve> = {
  period: FixtureStrobeCurve.PERIOD,
  linear: FixtureStrobeCurve.LINEAR,
  declared: FixtureStrobeCurve.DECLARED,
};

function curveOf(c: FixtureStrobeCurve): StrobeCurve | null {
  if (!(c in STROBE_CURVES)) throw new BadAnswerError("a strobe curve");
  return STROBE_CURVES[c];
}

function venueError(e: LightingVenueError | undefined): VenueError | null {
  return e ? { venue: e.venue, fixture: e.fixture, reason: e.reason } : null;
}

function rename(r: FixtureTypeRename | undefined): Rename | null {
  return r
    ? {
        from: r.from,
        to: r.to,
        venues: r.venues.map((v) => ({
          venue: v.venue,
          file: v.file,
          lines: v.lines,
        })),
        lines: r.lines,
      }
    : null;
}

/** A fixture type as the pages hold it. A listing's entry has a
 *  `footprint` (and, for a GDTF type, `gdtf`: null when its archive did
 *  not read); a single type's has its file's text as `dsl`. */
export function mapFixtureTypeEntry(
  i: FixtureTypeInfo,
  listing: boolean,
): FixtureTypeEntry {
  const t = i.fixtureType;
  const fixtureType: FixtureType = {
    name: t?.name ?? "",
    channels: { ...(t?.channels ?? {}) },
    max_strobe_frequency: t?.maxStrobeFrequency ?? null,
    min_strobe_frequency: t?.minStrobeFrequency ?? null,
    strobe_dmx_offset: t?.strobeDmxOffset ?? null,
  };
  const entry: FixtureTypeEntry = {
    referential: i.referential,
    rich: i.rich,
    fixture_type: fixtureType,
    file: i.file ?? null,
    extension: i.extension ?? null,
    owner: i.owner
      ? {
          file: i.owner.file,
          editable: i.owner.editable,
          reason: i.owner.reason ?? null,
        }
      : null,
    form_editable: i.formEditable,
  };
  if (listing) {
    entry.footprint = i.footprint ?? null;
    if (i.referential) {
      entry.gdtf = i.gdtf
        ? {
            fixture: i.gdtf.fixture,
            manufacturer: i.gdtf.manufacturer,
            modes: i.gdtf.modes,
            beam: i.gdtf.beam
              ? {
                  type: i.gdtf.beam.beamType ?? null,
                  angle: i.gdtf.beam.angle ?? null,
                }
              : null,
            thumbnail: i.gdtf.thumbnail ?? null,
            used_by: i.gdtf.usedBy,
            in_use: i.gdtf.inUse.map((m) => ({
              mode: m.mode,
              count: m.count,
            })),
          }
        : null;
    }
  } else {
    entry.dsl = i.dsl ?? null;
  }
  return entry;
}

/** The fixture types of a directory, by name, and the files that did not
 *  read. */
export function mapFixtureTypeList(
  r: ListFixtureTypesResponse,
): FixtureTypeList {
  return {
    fixture_types: Object.fromEntries(
      Object.entries(r.fixtureTypes).map(([name, info]) => [
        name,
        mapFixtureTypeEntry(info, true),
      ]),
    ),
    errors: r.errors.map((e) => ({ file: e.file, error: e.error })),
  };
}

function gdtfMode(m: FixtureTypeGdtfDetails["modes"][number]): GdtfMode {
  const mode: GdtfMode = { name: m.name, footprint: m.footprint };
  const o = m.outcome;
  switch (o.case) {
    case "refused":
      mode.refused = o.value;
      break;
    case "facts":
      mode.capabilities = o.value.capabilities.map((c) =>
        named(CAPABILITIES, c, "a capability"),
      );
      mode.wants = o.value.wants.map(want);
      mode.cells = o.value.cells;
      if (o.value.strobeRange) {
        mode.strobe_range = {
          min_hz: o.value.strobeRange.minHz ?? null,
          max_hz: o.value.strobeRange.maxHz,
        };
      }
      mode.channels = o.value.channels.map((c) => [c.offset, c.name]);
      break;
    default:
      throw new BadAnswerError("a mode");
  }
  return mode;
}

/** What a GDTF fixture's archive holds, for its page. */
export function mapFixtureTypeGdtf(d: FixtureTypeGdtfDetails): FixtureTypeGdtf {
  return {
    archive: d.archive,
    rig: d.rig ?? null,
    thumbnail: d.thumbnail ?? null,
    beam: d.beam
      ? {
          type: d.beam.beamType ?? null,
          beam_angle: d.beam.beamAngle ?? null,
          field_angle: d.beam.fieldAngle ?? null,
          luminous_flux: d.beam.luminousFlux ?? null,
          color_temperature: d.beam.colorTemperature ?? null,
          power: d.beam.power ?? null,
        }
      : null,
    about: d.about ?? null,
    venues: d.venues.map((v) => ({
      name: v.name,
      fixtures: v.fixtures.map((f) => ({
        name: f.name,
        mode: f.mode ?? null,
      })),
    })),
    inspection: { modes: d.modes.map(gdtfMode) },
  };
}

/** A fixture type saved (`status` is always `saved`), and what a rename
 *  did to the venues. */
export function mapFixtureTypeSaved(
  r: SaveFixtureTypeResponse,
): FixtureTypeSaved {
  return {
    status: "saved",
    name: r.name,
    rename: rename(r.rename),
    venue_error: venueError(r.venueError),
  };
}

/** A fixture type deleted (`status` is always `deleted`), and what became
 *  of its archive. */
export function mapFixtureTypeDeleted(
  r: DeleteFixtureTypeResponse,
): FixtureTypeDeleted {
  return {
    status: "deleted",
    name: r.name,
    archive_removed: r.archiveRemoved ?? null,
    archive_kept: r.archiveKept,
  };
}

/** A GDTF fixture's own settings, with the revision they are at (the form
 *  calls it `version`). */
export function mapFixtureSettings(
  s: FixtureTypeSettings,
): FixtureSettingsData {
  return {
    name: s.name,
    movement: {
      max_pan_speed: s.movement?.maxPanSpeed ?? null,
      max_tilt_speed: s.movement?.maxTiltSpeed ?? null,
    },
    strobe_curve: curveOf(s.strobeCurve),
    strobe: s.strobe
      ? {
          steps: s.strobe.steps,
          automatic: curveOf(s.strobe.automatic) ?? "declared",
        }
      : null,
    version: s.revision,
  };
}

/** What a settings save did (or, planned, would do). */
export function mapSettingsSaved(s: FixtureTypeSettingsSaved): SettingsSaved {
  return {
    version: s.revision,
    rename: rename(s.rename),
    venue_versions: { ...s.venueRevisions },
    venue_error: venueError(s.venueError),
  };
}

/** What a fixture type save writes: the file's whole text, or the
 *  channel-map form. */
export type FixtureTypeBody =
  | { text: string }
  | {
      channels: Record<string, number>;
      max_strobe_frequency?: number | null;
      min_strobe_frequency?: number | null;
      strobe_dmx_offset?: number | null;
    };

/** The call for a fixture type save: `ext` the form asked for (`fixture`
 *  moves a `.light` to a `.fixture`), `from` the old name of a rename. */
export function saveFixtureTypeRequest(
  name: string,
  body: FixtureTypeBody,
  opts: {
    ext?: "light" | "fixture";
    dir?: string;
    from?: string | null;
    venuesDir?: string;
  } = {},
): MessageInitShape<typeof SaveFixtureTypeRequestSchema> {
  return {
    name,
    from: opts.from || undefined,
    dir: opts.dir || undefined,
    venuesDir: opts.venuesDir || undefined,
    fileForm:
      opts.ext === "light"
        ? FixtureTypeFile.LIGHT
        : opts.ext === "fixture"
          ? FixtureTypeFile.FIXTURE
          : FixtureTypeFile.UNSPECIFIED,
    body:
      "text" in body
        ? { case: "dsl", value: body.text }
        : {
            case: "channelMap",
            value: {
              channels: body.channels,
              maxStrobeFrequency: body.max_strobe_frequency ?? undefined,
              minStrobeFrequency: body.min_strobe_frequency ?? undefined,
              strobeDmxOffset: body.strobe_dmx_offset ?? undefined,
            },
          },
  };
}

/** The call for a plan or a save of a GDTF fixture's settings, with the
 *  revision the form read them at (none is not checked). */
export function saveFixtureSettingsRequest(
  name: string,
  body: SettingsRequest,
  dirs: { dir?: string; venuesDir?: string },
  revision?: string,
): MessageInitShape<typeof SaveFixtureTypeSettingsRequestSchema> {
  return {
    name,
    dir: dirs.dir || undefined,
    venuesDir: dirs.venuesDir || undefined,
    expectedRevision: revision || undefined,
    newName: body.name,
    movement: {
      maxPanSpeed: body.movement?.max_pan_speed ?? undefined,
      maxTiltSpeed: body.movement?.max_tilt_speed ?? undefined,
    },
    strobeCurve: body.strobe_curve
      ? STROBE_CURVE_VALUES[body.strobe_curve]
      : FixtureStrobeCurve.UNSPECIFIED,
    write: body.write,
    venueRevisions: body.venue_versions ?? {},
  };
}

// ---- Venues ----

function point(v: LightingVec3): Vec3 {
  return [v.x, v.y, v.z];
}

function optionalPoint(v: LightingVec3 | undefined): Vec3 | null {
  return v ? point(v) : null;
}

function venueFixture(f: LightingFixture): Venue["fixtures"][string] {
  const fixture: Venue["fixtures"][string] = {
    name: f.name,
    fixture_type: f.fixtureType,
    universe: f.universe,
    start_channel: f.startChannel,
    tags: [...f.tags],
    position: optionalPoint(f.position),
    rotation: optionalPoint(f.rotation),
  };
  // Present only when the venue says it, as the venue always served them.
  if (f.mode !== undefined) fixture.mode = f.mode;
  if (f.beamAngle !== undefined) fixture.beam_angle = f.beamAngle;
  return fixture;
}

/** A venue as the pages hold it. */
export function mapVenue(v: LightingVenue): Venue {
  return {
    name: v.name,
    fixtures: Object.fromEntries(
      Object.entries(v.fixtures).map(([name, f]) => [name, venueFixture(f)]),
    ),
    focus_points: Object.fromEntries(
      Object.entries(v.focusPoints).map(([name, p]) => [name, point(p)]),
    ),
    source: v.source
      ? {
          mvr: v.source.mvr,
          origin: v.source.origin ? point(v.source.origin) : [0, 0, 0],
        }
      : null,
  };
}

/** The venues of a directory with their files and revisions (the pages
 *  call a revision `version`), and the files that did not read. */
export function mapVenueList(r: ListVenuesResponse): VenueList {
  return {
    venues: Object.fromEntries(
      Object.entries(r.venues).map(([name, v]) => [name, mapVenue(v)]),
    ),
    owners: Object.fromEntries(
      Object.entries(r.owners).map(([name, o]) => [
        name,
        { file: o.file, editable: o.editable, reason: o.reason ?? null },
      ]),
    ),
    errors: r.errors.map((e) => ({ file: e.file, error: e.error })),
    versions: { ...r.revisions },
  };
}

/** A venue with its file's text and revision. */
export function mapVenueDetail(d: VenueDetailMessage): VenueDetail {
  if (!d.venue) throw new BadAnswerError("a venue");
  return { venue: mapVenue(d.venue), dsl: d.dsl, version: d.revision };
}

/** What a venue save answers (`status` is always `saved`). */
export function mapVenueSaved(r: SaveVenueResponse): VenueSaved {
  return {
    status: "saved",
    name: r.name,
    reloaded: r.reloaded,
    version: r.revision,
    venue_error: venueError(r.venueError),
  };
}

/** What a venue save writes: its fixtures as rows (each as the venue
 *  serves it, a `null` mode or beam angle for none), its focus points and
 *  its provenance. */
export interface VenueBody {
  fixtures: (Omit<Venue["fixtures"][string], "mode" | "beam_angle"> & {
    mode?: string | null;
    beam_angle?: number | null;
  })[];
  focus_points?: Venue["focus_points"];
  source?: Venue["source"];
}

function vec(
  v: Vec3 | null | undefined,
): { x: number; y: number; z: number } | undefined {
  return v ? { x: v[0], y: v[1], z: v[2] } : undefined;
}

/** The call for a venue save: `from` the old name of a rename, `version`
 *  the revision the venue was read at (none is not checked). */
export function saveVenueRequest(
  name: string,
  data: VenueBody,
  opts: { dir?: string; from?: string | null; version?: string } = {},
): MessageInitShape<typeof SaveVenueRequestSchema> {
  return {
    name,
    from: opts.from || undefined,
    dir: opts.dir || undefined,
    expectedRevision: opts.version || undefined,
    body: {
      case: "form",
      value: {
        fixtures: data.fixtures.map((f) => ({
          name: f.name,
          fixtureType: f.fixture_type,
          universe: f.universe,
          startChannel: f.start_channel,
          tags: f.tags ?? [],
          position: vec(f.position),
          rotation: vec(f.rotation),
          mode: f.mode ?? undefined,
          beamAngle: f.beam_angle ?? undefined,
        })),
        focusPoints: Object.fromEntries(
          Object.entries(data.focus_points ?? {}).map(([n, p]) => [
            n,
            { x: p[0], y: p[1], z: p[2] },
          ]),
        ),
        source: data.source
          ? { mvr: data.source.mvr, origin: vec(data.source.origin) }
          : undefined,
      },
    },
  };
}

/** The addresses a venue's fixtures occupy, and the findings on them. */
export function mapVenuePatch(p: VenuePatchMessage): VenuePatch {
  return {
    spans: p.spans.map((s) => ({
      fixture: s.fixture,
      universe: s.universe,
      address: s.address,
      footprint: s.footprint ?? null,
      type: s.fixtureType,
      mode: s.mode ?? null,
    })),
    overlaps: p.overlaps.map((o) => ({
      first: o.first,
      second: o.second,
      first_gang: [...o.firstGang],
      second_gang: [...o.secondGang],
      universe: o.universe,
      from: o.from,
      to: o.to,
    })),
    overruns: p.overruns.map((o) => ({
      fixture: o.fixture,
      universe: o.universe,
      address: o.address,
      footprint: o.footprint,
      last: o.last,
    })),
  };
}

/** What the 3D view draws for a venue from its files. */
export function mapVenueScene(s: VenueSceneMessage): VenueScene {
  if (!s.venue) throw new BadAnswerError("a venue's scene");
  return {
    fixtures: Object.fromEntries(
      Object.entries(s.fixtures).map(([name, f]) => [
        name,
        {
          tags: [...f.tags],
          type: f.fixtureType,
          position: optionalPoint(f.position),
          rotation: optionalPoint(f.rotation),
          beam_angle: f.beamAngle ?? null,
          rig: f.rig ?? null,
          mode: f.mode ?? null,
        },
      ]),
    ),
    venue: {
      name: s.venue.name,
      focus_points: Object.fromEntries(
        Object.entries(s.venue.focusPoints).map(([name, p]) => [
          name,
          point(p),
        ]),
      ),
      scenery: s.venue.scenery ?? null,
      scenery_error: s.venue.sceneryError ?? null,
    },
  };
}

/** The focus points an aim point addition made. */
export function mapAimPoints(a: AddVenueAimPointsResponse): AimPoints {
  return {
    created: a.created.map((c) => {
      if (!c.point) throw new BadAnswerError("an aim point");
      return { name: c.name, fixture: c.fixture, point: point(c.point) };
    }),
    reloaded: a.reloaded,
    version: a.revision,
    venue_error: venueError(a.venueError),
  };
}

/** What an MVR export would hold, and the name a download would carry. */
export function mapMvrExportSummary(
  s: MvrExportSummaryMessage,
): MvrExportSummary {
  return {
    file_name: s.fileName,
    venue: s.venue,
    output: s.output,
    fixtures: s.fixtures,
    focus_points: s.focusPoints,
    embedded_gdtfs: [...s.embeddedGdtfs],
    generated_gdtfs: { ...s.generatedGdtfs },
    positioned_fixtures: s.positionedFixtures,
    fixed_fixtures: s.fixedFixtures.map((f) => ({
      name: f.name,
      focus: f.focus ?? null,
    })),
    warnings: [...s.warnings],
  };
}

// ---- Errors ----

/** The metadata key of a refused fixture test that says why, in a word
 *  (src/controller/grpc/lighting_service.rs). */
const REFUSAL_REASON_KEY = "x-refusal-reason";

/** The message a locked player's refusal carries: the lock layer refuses
 *  before the handler, so no reason rides on it. */
const LOCKED_MESSAGE = "Player is locked";

/** A fixture test the server refused: its reason in a word (`locked`,
 *  `playing`, `no_dmx`, `universe`, `address`, `controls`, `type`, `mode`,
 *  `raw`, `files`), and the sentence. */
export class TestRefused extends Error {
  reason: string | null;

  constructor(message: string, reason: string | null) {
    super(message);
    this.name = "TestRefused";
    this.reason = reason;
  }
}

/** The refusal of a fixture test call ({@link callError}'s message), with
 *  the reason the server gave; a locked player's has `locked`. Anything that
 *  is not a failed call passes through. */
export function testCallError(e: unknown, fallback: string): Error {
  if (!(e instanceof ConnectError)) return callError(e, fallback);
  const err = callError(e, fallback);
  const reason =
    e.metadata.get(REFUSAL_REASON_KEY) ??
    (e.code === Code.FailedPrecondition &&
    e.rawMessage.startsWith(LOCKED_MESSAGE)
      ? "locked"
      : null);
  return new TestRefused(err.message, reason);
}
