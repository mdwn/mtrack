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
  FixtureStrobeCurve,
  FixtureTestDimmer,
  FixtureTestUnavailable,
  FixtureTypeFile,
  LightingSeverity,
  LightingWant,
  ReadinessCheckKey,
  ReadinessCheckState,
  type FitGroupSchema,
  type FixtureTestAnswerSchema,
  type FixtureTestOptionsSchema,
  type AddVenueAimPointsResponseSchema,
  type DeleteFixtureTypeResponseSchema,
  type FixtureTypeGdtfDetailsSchema,
  type FixtureTypeInfoSchema,
  type FixtureTypeSettingsSavedSchema,
  type FixtureTypeSettingsSchema,
  type KeepMvrExportResponseSchema,
  type ListFixtureTypesResponseSchema,
  type ListVenuesResponseSchema,
  type MvrExportSummarySchema,
  type SaveVenueRequest,
  type SaveVenueResponseSchema,
  type VenueDetailSchema,
  type VenuePatchSchema,
  type VenueSceneSchema,
  type SaveFixtureTypeRequest,
  type SaveFixtureTypeResponseSchema,
  type SaveFixtureTypeSettingsRequest,
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
import type { VenueBody } from "../../src/lib/grpc/lighting";
import type {
  AimPoints,
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
  FixtureSettingsData,
  FixtureTypeDeleted,
  FixtureTypeEntry,
  FixtureTypeGdtf,
  FixtureTypeList,
  FixtureTypeSaved,
  Height,
  LightingGroup,
  LightingValidation,
  MvrExportSummary,
  Preview,
  Readiness,
  Rename,
  SettingsRequest,
  SettingsSaved,
  Severity,
  StrobeCurve,
  Summary,
  TestAnswer,
  TestOptions,
  TestUnavailable,
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

// ---- Fixture types ----

const STROBE_CURVES: Record<StrobeCurve, FixtureStrobeCurve> = {
  period: FixtureStrobeCurve.PERIOD,
  linear: FixtureStrobeCurve.LINEAR,
  declared: FixtureStrobeCurve.DECLARED,
};

const curve = (c: StrobeCurve | null): FixtureStrobeCurve =>
  c ? STROBE_CURVES[c] : FixtureStrobeCurve.UNSPECIFIED;

/** A fixture type's entry with everything filled in, for a spec that states
 *  only what it is about: a hand-written type with a file `<name>.light`
 *  unless `over` says otherwise. */
export function typeEntry(
  name: string,
  over: Partial<FixtureTypeEntry> = {},
): FixtureTypeEntry {
  const extension = over.extension === undefined ? "light" : over.extension;
  const file =
    over.file === undefined && extension ? `${name}.${extension}` : over.file;
  return {
    referential: false,
    rich: false,
    fixture_type: {
      name,
      channels: {},
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    file: file ?? null,
    extension,
    owner: file ? { file, editable: true, reason: null } : null,
    form_editable: false,
    ...over,
  };
}

/** A fixture type, as `GetFixtureType` and the listing answer it. */
export function fixtureTypeInfoReply(
  e: FixtureTypeEntry,
): MessageInitShape<typeof FixtureTypeInfoSchema> {
  return {
    referential: e.referential,
    footprint: e.footprint ?? undefined,
    rich: e.rich,
    fixtureType: {
      name: e.fixture_type.name,
      channels: e.fixture_type.channels,
      maxStrobeFrequency: e.fixture_type.max_strobe_frequency ?? undefined,
      minStrobeFrequency: e.fixture_type.min_strobe_frequency ?? undefined,
      strobeDmxOffset: e.fixture_type.strobe_dmx_offset ?? undefined,
    },
    file: e.file ?? undefined,
    extension: e.extension ?? undefined,
    owner: e.owner
      ? {
          file: e.owner.file,
          editable: e.owner.editable,
          reason: e.owner.reason ?? undefined,
        }
      : undefined,
    formEditable: e.form_editable,
    gdtf: e.gdtf
      ? {
          fixture: e.gdtf.fixture,
          manufacturer: e.gdtf.manufacturer,
          modes: e.gdtf.modes,
          beam: e.gdtf.beam
            ? {
                beamType: e.gdtf.beam.type ?? undefined,
                angle: e.gdtf.beam.angle ?? undefined,
              }
            : undefined,
          thumbnail: e.gdtf.thumbnail ?? undefined,
          usedBy: e.gdtf.used_by,
          inUse: e.gdtf.in_use,
        }
      : undefined,
    dsl: e.dsl ?? undefined,
  };
}

/** The fixture types of a directory. */
export function fixtureTypeListReply(
  list: FixtureTypeList,
): MessageInitShape<typeof ListFixtureTypesResponseSchema> {
  return {
    fixtureTypes: Object.fromEntries(
      Object.entries(list.fixture_types).map(([name, e]) => [
        name,
        fixtureTypeInfoReply(e),
      ]),
    ),
    errors: list.errors,
  };
}

/** What a GDTF fixture's archive holds. */
export function fixtureTypeGdtfReply(
  g: FixtureTypeGdtf,
): MessageInitShape<typeof FixtureTypeGdtfDetailsSchema> {
  return {
    archive: g.archive,
    rig: g.rig ?? undefined,
    thumbnail: g.thumbnail ?? undefined,
    beam: g.beam
      ? {
          beamType: g.beam.type ?? undefined,
          beamAngle: g.beam.beam_angle ?? undefined,
          fieldAngle: g.beam.field_angle ?? undefined,
          luminousFlux: g.beam.luminous_flux ?? undefined,
          colorTemperature: g.beam.color_temperature ?? undefined,
          power: g.beam.power ?? undefined,
        }
      : undefined,
    about: g.about ?? undefined,
    venues: g.venues.map((v) => ({
      name: v.name,
      fixtures: v.fixtures.map((f) => ({
        name: f.name,
        mode: f.mode ?? undefined,
      })),
    })),
    modes: g.inspection.modes.map((m) => ({
      name: m.name,
      footprint: m.footprint,
      outcome:
        m.refused !== undefined
          ? { case: "refused" as const, value: m.refused }
          : {
              case: "facts" as const,
              value: {
                capabilities: (m.capabilities ?? []).map(
                  (c) => CAPABILITIES[c],
                ),
                wants: (m.wants ?? []).map((w) => WANTS[w]),
                cells: m.cells ?? 0,
                strobeRange: m.strobe_range
                  ? {
                      minHz: m.strobe_range.min_hz ?? undefined,
                      maxHz: m.strobe_range.max_hz,
                    }
                  : undefined,
                channels: (m.channels ?? []).map(([offset, name]) => ({
                  offset,
                  name,
                })),
              },
            },
    })),
  };
}

function renameReply(r: Rename | null) {
  return r ?? undefined;
}

function venueErrorReply(e: VenueError | null) {
  return e ?? undefined;
}

/** A fixture type saved; a rename says what it did to the venues. */
export function fixtureTypeSavedReply(
  s: Pick<FixtureTypeSaved, "name"> &
    Partial<Pick<FixtureTypeSaved, "rename" | "venue_error">>,
): MessageInitShape<typeof SaveFixtureTypeResponseSchema> {
  return {
    name: s.name,
    rename: renameReply(s.rename ?? null),
    venueError: venueErrorReply(s.venue_error ?? null),
  };
}

/** A fixture type deleted, and what became of its archive. */
export function fixtureTypeDeletedReply(
  d: Pick<FixtureTypeDeleted, "name"> & Partial<FixtureTypeDeleted>,
): MessageInitShape<typeof DeleteFixtureTypeResponseSchema> {
  return {
    name: d.name,
    archiveRemoved: d.archive_removed ?? undefined,
    archiveKept: d.archive_kept ?? false,
  };
}

/** A GDTF fixture's own settings. */
export function fixtureSettingsReply(
  s: FixtureSettingsData,
): MessageInitShape<typeof FixtureTypeSettingsSchema> {
  return {
    name: s.name,
    movement: {
      maxPanSpeed: s.movement.max_pan_speed ?? undefined,
      maxTiltSpeed: s.movement.max_tilt_speed ?? undefined,
    },
    strobeCurve: curve(s.strobe_curve),
    strobe: s.strobe
      ? { steps: s.strobe.steps, automatic: curve(s.strobe.automatic) }
      : undefined,
    revision: s.version,
  };
}

/** What a settings save did, or, planned, would do. */
export function settingsSavedReply(
  s: SettingsSaved,
): MessageInitShape<typeof FixtureTypeSettingsSavedSchema> {
  return {
    revision: s.version,
    rename: renameReply(s.rename),
    venueRevisions: s.venue_versions,
    venueError: venueErrorReply(s.venue_error),
  };
}

/** What a `SaveFixtureType` call sent, as the page's old `PUT` carried it:
 *  the name, the directories and the old name of a rename, the form asked
 *  for, and the body (the file's text, or the channel-map form). */
export function savedTypeRequest(r: SaveFixtureTypeRequest) {
  const ext =
    r.fileForm === FixtureTypeFile.LIGHT
      ? "light"
      : r.fileForm === FixtureTypeFile.FIXTURE
        ? "fixture"
        : null;
  return {
    name: r.name,
    from: r.from ?? null,
    dir: r.dir ?? null,
    venuesDir: r.venuesDir ?? null,
    ext,
    text: r.body.case === "dsl" ? r.body.value : null,
    form:
      r.body.case === "channelMap"
        ? {
            channels: { ...r.body.value.channels },
            max_strobe_frequency: r.body.value.maxStrobeFrequency ?? null,
            min_strobe_frequency: r.body.value.minStrobeFrequency ?? null,
            strobe_dmx_offset: r.body.value.strobeDmxOffset ?? null,
          }
        : null,
  };
}

/** What a `SaveFixtureTypeSettings` call sent, as the page's old `POST`
 *  carried it: the body, and the revision it was made against (the old
 *  `If-Match`). */
export function savedSettingsRequest(
  r: SaveFixtureTypeSettingsRequest,
): SettingsRequest & {
  type: string;
  revision: string | null;
  dir: string | null;
  venuesDir: string | null;
} {
  const curves: Record<FixtureStrobeCurve, StrobeCurve | null> = {
    [FixtureStrobeCurve.UNSPECIFIED]: null,
    [FixtureStrobeCurve.PERIOD]: "period",
    [FixtureStrobeCurve.LINEAR]: "linear",
    [FixtureStrobeCurve.DECLARED]: "declared",
  };
  return {
    type: r.name,
    name: r.newName,
    movement: {
      max_pan_speed: r.movement?.maxPanSpeed ?? null,
      max_tilt_speed: r.movement?.maxTiltSpeed ?? null,
    },
    strobe_curve: curves[r.strobeCurve],
    write: r.write,
    venue_versions: { ...r.venueRevisions },
    revision: r.expectedRevision ?? null,
    dir: r.dir ?? null,
    venuesDir: r.venuesDir ?? null,
  };
}

// ---- Venues ----

type VenueInit = NonNullable<
  MessageInitShape<typeof VenueDetailSchema>["venue"]
>;

/** A venue as the page holds it, a `null` mode or beam angle for none (what
 *  a save sends) allowed beside an absent one. */
export type VenueLike = Omit<Venue, "fixtures"> & {
  fixtures: Record<
    string,
    Omit<
      Venue["fixtures"][string],
      "mode" | "beam_angle" | "position" | "rotation"
    > & {
      mode?: string | null;
      beam_angle?: number | null;
      position?: Vec3 | null;
      rotation?: Vec3 | null;
    }
  >;
};

function venueMessage(v: VenueLike): VenueInit {
  return {
    name: v.name,
    fixtures: Object.fromEntries(
      Object.entries(v.fixtures).map(([name, f]) => [
        name,
        {
          name: f.name,
          fixtureType: f.fixture_type,
          universe: f.universe,
          startChannel: f.start_channel,
          tags: f.tags,
          position: f.position ? vec3(f.position) : undefined,
          rotation: f.rotation ? vec3(f.rotation) : undefined,
          mode: f.mode ?? undefined,
          beamAngle: f.beam_angle ?? undefined,
        },
      ]),
    ),
    focusPoints: Object.fromEntries(
      Object.entries(v.focus_points).map(([name, p]) => [name, vec3(p)]),
    ),
    source: v.source
      ? { mvr: v.source.mvr, origin: vec3(v.source.origin) }
      : undefined,
  };
}

/** The venues of a directory, with their files and revisions (the page's
 *  `versions`) and the files that did not read. */
export function venueListReply(
  list: Omit<VenueList, "venues"> & { venues: Record<string, VenueLike> },
): MessageInitShape<typeof ListVenuesResponseSchema> {
  return {
    venues: Object.fromEntries(
      Object.entries(list.venues).map(([name, v]) => [name, venueMessage(v)]),
    ),
    owners: Object.fromEntries(
      Object.entries(list.owners).map(([name, o]) => [
        name,
        { file: o.file, editable: o.editable, reason: o.reason ?? undefined },
      ]),
    ),
    errors: list.errors ?? [],
    revisions: list.versions ?? {},
  };
}

/** A venue with its file's text and revision. */
export function venueDetailReply(
  d: Omit<VenueDetail, "venue"> & { venue: VenueLike },
): MessageInitShape<typeof VenueDetailSchema> {
  return { venue: venueMessage(d.venue), dsl: d.dsl, revision: d.version };
}

/** A venue saved. */
export function venueSavedReply(
  s: Partial<Pick<VenueSaved, "reloaded" | "version" | "venue_error">> &
    Pick<VenueSaved, "name">,
): MessageInitShape<typeof SaveVenueResponseSchema> {
  return {
    name: s.name,
    reloaded: s.reloaded ?? false,
    revision: s.version ?? "mock-v2",
    venueError: s.venue_error ?? undefined,
  };
}

/** The addresses a venue's fixtures occupy. */
export function venuePatchReply(
  p: VenuePatch,
): MessageInitShape<typeof VenuePatchSchema> {
  return {
    spans: p.spans.map((s) => ({
      fixture: s.fixture,
      universe: s.universe,
      address: s.address,
      footprint: s.footprint ?? undefined,
      fixtureType: s.type,
      mode: s.mode ?? undefined,
    })),
    overlaps: p.overlaps.map((o) => ({
      first: o.first,
      second: o.second,
      firstGang: o.first_gang,
      secondGang: o.second_gang,
      universe: o.universe,
      from: o.from,
      to: o.to,
    })),
    overruns: p.overruns,
  };
}

/** What the 3D view draws for a venue from its files. */
export function venueSceneReply(
  s: VenueScene,
): MessageInitShape<typeof VenueSceneSchema> {
  return {
    fixtures: Object.fromEntries(
      Object.entries(s.fixtures).map(([name, f]) => [
        name,
        {
          tags: f.tags,
          fixtureType: f.type,
          position: f.position ? vec3(f.position) : undefined,
          rotation: f.rotation ? vec3(f.rotation) : undefined,
          beamAngle: f.beam_angle ?? undefined,
          rig: f.rig ?? undefined,
          mode: f.mode ?? undefined,
        },
      ]),
    ),
    venue: {
      name: s.venue.name,
      focusPoints: Object.fromEntries(
        Object.entries(s.venue.focus_points).map(([name, p]) => [
          name,
          vec3(p),
        ]),
      ),
      scenery: s.venue.scenery ?? undefined,
      sceneryError: s.venue.scenery_error ?? undefined,
    },
  };
}

/** The focus points an aim point addition made. */
export function aimPointsReply(
  a: AimPoints,
): MessageInitShape<typeof AddVenueAimPointsResponseSchema> {
  return {
    created: a.created.map((c) => ({
      name: c.name,
      fixture: c.fixture,
      point: vec3(c.point),
    })),
    reloaded: a.reloaded,
    revision: a.version,
    venueError: a.venue_error ?? undefined,
  };
}

/** What an MVR export would hold. */
export function mvrExportSummaryReply(
  s: MvrExportSummary,
): MessageInitShape<typeof MvrExportSummarySchema> {
  return {
    fileName: s.file_name,
    venue: s.venue,
    output: s.output,
    fixtures: s.fixtures,
    focusPoints: s.focus_points,
    embeddedGdtfs: s.embedded_gdtfs,
    generatedGdtfs: s.generated_gdtfs,
    positionedFixtures: s.positioned_fixtures,
    fixedFixtures: s.fixed_fixtures.map((f) => ({
      name: f.name,
      focus: f.focus ?? undefined,
    })),
    warnings: s.warnings,
  };
}

/** A kept copy of an export (`kept` the project path), or the file that
 *  was left alone (`existing`). */
export function keptReply(
  outcome: { kept: string } | { existing: string },
): MessageInitShape<typeof KeepMvrExportResponseSchema> {
  return {
    outcome:
      "kept" in outcome
        ? { case: "kept", value: outcome.kept }
        : { case: "existing", value: outcome.existing },
  };
}

/** What a `SaveVenue` call sent, as the page's old `PUT` carried it: the
 *  venue's fixtures as rows, its focus points and its provenance. */
export function savedVenueRequest(r: SaveVenueRequest): VenueBody & {
  name: string;
  from: string | null;
  dir: string | null;
  version: string | null;
  text: string | null;
} {
  const point = (p: { x: number; y: number; z: number }): Vec3 => [
    p.x,
    p.y,
    p.z,
  ];
  const form = r.body.case === "form" ? r.body.value : null;
  return {
    name: r.name,
    from: r.from ?? null,
    dir: r.dir ?? null,
    version: r.expectedRevision ?? null,
    text: r.body.case === "dsl" ? r.body.value : null,
    fixtures: (form?.fixtures ?? []).map((f) => ({
      name: f.name,
      fixture_type: f.fixtureType,
      universe: f.universe,
      start_channel: f.startChannel,
      tags: [...f.tags],
      position: f.position ? point(f.position) : null,
      rotation: f.rotation ? point(f.rotation) : null,
      mode: f.mode ?? null,
      beam_angle: f.beamAngle ?? null,
    })),
    focus_points: Object.fromEntries(
      Object.entries(form?.focusPoints ?? {}).map(([n, p]) => [n, point(p)]),
    ),
    source: form?.source
      ? {
          mvr: form.source.mvr,
          origin: form.source.origin ? point(form.source.origin) : [0, 0, 0],
        }
      : null,
  };
}
