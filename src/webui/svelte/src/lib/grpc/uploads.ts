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

// File bytes over gRPC and HTTP: "calls are gRPC; bytes are HTTP". A file is
// announced with `BeginUpload` (`ConfigService`), which answers an id and a
// path on the web server; the bytes are `PUT` to that path; and the call
// that uses them (`AddTrackFiles`, `AddSampleFile`, `ImportGdtf`,
// `InspectMvr`, `ImportMvr`) takes the id. An MVR export is the other way:
// `ExportMvr` stages the archive and answers the path to fetch it from.
//
// Only the mapping lives here, so a unit test can run it without a page: the
// answers become the shapes the components hold
// (`GdtfImport`, `MvrInspection`, `MvrImportAnswer`, `SampleUploaded`, and
// the track upload's answer), and a failed call the errors they tell apart.
// The calls, and the transfer of the bytes, are in src/lib/api/{songs,config,
// upload}.ts.

import { Code, ConnectError } from "@connectrpc/connect";
import type { MessageInitShape } from "@bufbuild/protobuf";
import { ApiError } from "../api/rest";
import { callError } from "./library";
import { BadAnswerError } from "../lighting/answer";
import {
  type AddSampleFileResponse,
  type AddTrackFilesResponse,
  type GdtfImport as GdtfImportMessage,
  type ImportMvrResponse,
  type LightingVec3,
  type MvrBounds,
  type MvrHandEdit,
  type MvrInspection as MvrInspectionMessage,
  type MvrOptionsSchema,
  type MvrPlan as MvrPlanMessage,
} from "../../gen/player/v1/player_pb";
import type {
  BoundsMm,
  GdtfImport,
  HandEdit,
  MvrImport,
  MvrImportAnswer,
  MvrInspection,
  MvrKeep,
  MvrPlan,
  SampleUploaded,
  Vec3,
  VenueError,
} from "../../gen/rules/mtrack_rules";

/** The metadata key of a refused GDTF or MVR import that says what refused
 *  it, in a word (`venue`, `record`, `name`, `library`, `write`,
 *  `unreadable`; src/controller/grpc/lighting_service.rs). */
const IMPORT_KIND_KEY = "x-import-kind";

/** The error a failed import or inspection shows ({@link callError}), with
 *  the kind of refusal the server names on the error: a wizard offers
 *  another venue name only for a refusal of the venue itself. */
export function importCallError(e: unknown, fallback: string): Error {
  const err = callError(e, fallback);
  if (e instanceof ConnectError && err instanceof ApiError) {
    const kind = e.metadata.get(IMPORT_KIND_KEY);
    // A refusal of something that is there already keeps its own kind.
    if (kind && e.code !== Code.AlreadyExists) err.kind = kind;
  }
  return err;
}

// ---- Tracks and samples ----

/** What a single track upload answers: the file, whether it replaced one, and
 *  the name the song has (a new one's may not be the one asked for). */
export interface TrackUploaded {
  status: "uploaded" | "replaced";
  replaced: boolean;
  song: string;
  file: string;
}

/** What an upload of several track files answers. */
export interface TracksUploaded {
  status: "uploaded";
  song: string;
  files: string[];
}

export function mapTrackUploaded(r: AddTrackFilesResponse): TrackUploaded {
  const [file] = r.files;
  if (!file) throw new BadAnswerError("a track upload");
  return {
    status: file.replaced ? "replaced" : "uploaded",
    replaced: file.replaced,
    song: r.song,
    file: file.file,
  };
}

export function mapTracksUploaded(r: AddTrackFilesResponse): TracksUploaded {
  return {
    status: "uploaded",
    song: r.song,
    files: r.files.map((f) => f.file),
  };
}

export function mapSampleUploaded(r: AddSampleFileResponse): SampleUploaded {
  return { status: "uploaded", file: r.file, path: r.path };
}

// ---- GDTF and MVR ----

export function mapGdtfImport(m: GdtfImportMessage): GdtfImport {
  return {
    type_name: m.typeName,
    fixture: m.fixture,
    manufacturer: m.manufacturer,
    modes: m.modes,
    archive: m.archive,
    already_imported: m.alreadyImported,
    renamed_from: m.renamedFrom ?? null,
    refused_modes: m.refusedModes.map((r) => ({
      mode: r.mode,
      reason: r.reason,
    })),
    fixture_file: m.fixtureFile ?? null,
  };
}

function vec(v: LightingVec3): Vec3 {
  return [v.x, v.y, v.z];
}

function optionalVec(v: LightingVec3 | undefined): Vec3 | null {
  return v ? vec(v) : null;
}

function handEdits(edits: MvrHandEdit[]): HandEdit[] {
  return edits.map((e) => ({ field: e.field, mine: e.mine, mvr: e.mvr }));
}

function bounds(b: MvrBounds | undefined): BoundsMm | null {
  if (!b) return null;
  if (!b.min || !b.max) throw new BadAnswerError("an MVR's bounds");
  return { min: vec(b.min), max: vec(b.max) };
}

function mapPlan(p: MvrPlanMessage): MvrPlan {
  if (!p.origin) throw new BadAnswerError("an MVR plan");
  return {
    venue_name: p.venueName,
    venue_file: p.venueFile,
    archive: p.archive,
    merge: p.merge,
    origin: vec(p.origin),
    fixture_types: p.fixtureTypes.map((t) => ({
      name: t.name,
      archive: t.archive,
      modes: [...t.modes],
      existing: t.existing,
      fixture_file: t.fixtureFile ?? null,
    })),
    fixtures: p.fixtures.map((f) => ({
      name: f.name,
      layer: f.layer,
      fixture_type: f.fixtureType ?? null,
      mode: f.mode ?? null,
      patch: f.patch ? [f.patch.universe, f.patch.address] : null,
      position: optionalVec(f.position),
      rotation: optionalVec(f.rotation),
      tags: [...f.tags],
      todo: f.todo ?? null,
      change: f.change ?? null,
      overwrites: handEdits(f.overwrites),
      kept_edits: [...f.keptEdits],
    })),
    removed_fixtures: p.removedFixtures.map((f) => ({
      name: f.name,
      tags: [...f.tags],
    })),
    kept_fixtures: [...p.keptFixtures],
    focus_points: p.focusPoints.map((f) => {
      if (!f.point) throw new BadAnswerError("an MVR plan's focus point");
      return {
        name: f.name,
        point: vec(f.point),
        change: f.change ?? null,
        overwrites: handEdits(f.overwrites),
        kept_edit: f.keptEdit,
      };
    }),
    removed_focus_points: [...p.removedFocusPoints],
    kept_focus_points: [...p.keptFocusPoints],
    scenery_objects: p.sceneryObjects,
    scenery_meshes_undrawn: p.sceneryMeshesUndrawn,
    warnings: [...p.warnings],
  };
}

/** What an inspection answers: the plan, and the file seen from above. */
export function mapMvrInspection(m: MvrInspectionMessage): MvrInspection {
  if (!m.report || !m.scene) throw new BadAnswerError("an MVR inspection");
  return {
    file_name: m.fileName,
    report: mapPlan(m.report),
    scene: {
      fixtures: m.scene.fixtures.map((f) => ({
        name: f.name,
        layer: f.layer,
        position_mm: optionalVec(f.positionMm),
      })),
      focus_points: m.scene.focusPoints.map((f) => ({
        name: f.name,
        position_mm: optionalVec(f.positionMm),
      })),
      scenery: m.scene.scenery.map((o) => ({
        name: o.name,
        kind: o.kind,
        deck: o.deck,
        bounds_mm: bounds(o.boundsMm),
      })),
      deck_mm: bounds(m.scene.deck),
    },
  };
}

function venueError(e: ImportMvrResponse["venueError"]): VenueError | null {
  return e ? { venue: e.venue, fixture: e.fixture, reason: e.reason } : null;
}

/** An import's answer: the plan (a request to plan), or the report with the
 *  venue's reload and the failure it left. */
export function mapMvrImportAnswer(m: ImportMvrResponse): MvrImportAnswer {
  if (!m.write) {
    if (!m.plan) throw new BadAnswerError("an MVR import plan");
    return { write: false, plan: mapPlan(m.plan) };
  }
  if (!m.report?.plan) throw new BadAnswerError("an MVR import");
  const report: MvrImport = {
    ...mapPlan(m.report.plan),
    written: [...m.report.written],
    distillation_warnings: Object.fromEntries(
      Object.entries(m.report.distillationWarnings).map(([type, w]) => [
        type,
        [...w.values],
      ]),
    ),
  };
  return {
    write: true,
    report,
    reloaded: m.reloaded ?? false,
    venue_error: venueError(m.venueError),
  };
}

/** The choices an inspection or import takes, as the request carries them:
 *  `origin` in millimeters, `keep` the hand edits a merge keeps. */
export function mvrOptions(options: {
  name?: string;
  origin?: Vec3;
  venuesDir?: string;
  fixtureTypesDir?: string;
  keep?: MvrKeep;
}): MessageInitShape<typeof MvrOptionsSchema> {
  const keep = options.keep;
  return {
    name: options.name || undefined,
    originMm: options.origin
      ? { x: options.origin[0], y: options.origin[1], z: options.origin[2] }
      : undefined,
    venuesDir: options.venuesDir || undefined,
    fixtureTypesDir: options.fixtureTypesDir || undefined,
    keep: keep
      ? {
          fixtures: Object.fromEntries(
            Object.entries(keep.fixtures ?? {}).map(([name, fields]) => [
              name,
              { fields },
            ]),
          ),
          focusPoints: keep.focus_points ?? [],
        }
      : undefined,
  };
}
