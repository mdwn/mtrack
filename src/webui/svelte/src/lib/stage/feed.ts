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

// The gRPC stage stream (`WatchStage`) read into what the stage views
// draw from: a layout is the metadata (the venue's fixtures and the venue),
// a frame read against its layout is the state. The mirror of the player's
// src/controller/grpc/stage.rs. Pure: no DOM, no stores; whoever holds the
// reader says where the two go.

import { fromBinary } from "@bufbuild/protobuf";
import {
  StageUpdateSchema,
  type StageFrame,
  type StageLayout,
  type StagePose,
  type StageUpdate,
  type StageVec3,
} from "../../gen/player/v1/player_pb";
import type {
  Capability,
  FixtureMetadata,
  PoseSnapshot,
  StageMetadata,
  StageState,
  Vec3,
} from "../../gen/rules/mtrack_rules";

/**
 * A number the stream carried as a 32-bit float, as the shortest decimal
 * that is that float: what a person wrote in a venue file (`2.7`) comes
 * back as `2.7`, not as the float's exact value (`2.700000047683716`),
 * so an editor that shows it, or writes it back beside an edit to another
 * axis, does not rewrite the file's number. A value the stream rounded to
 * 32 bits stays rounded; there is no getting the rest back.
 */
export function f32(x: number): number {
  if (!Number.isFinite(x) || Number.isInteger(x)) return x;
  for (let digits = 1; digits <= 9; digits++) {
    const decimal = Number(x.toPrecision(digits));
    if (Math.fround(decimal) === x) return decimal;
  }
  return x;
}

const vec3 = (v: StageVec3): Vec3 => [f32(v.x), f32(v.y), f32(v.z)];

function pose(p: StagePose): PoseSnapshot {
  return {
    pan: f32(p.pan),
    tilt: f32(p.tilt),
    aim: p.aim ? vec3(p.aim) : [0, 0, 0],
    floor: p.floor ? [f32(p.floor.x), f32(p.floor.y)] : null,
  };
}

/**
 * What the stage views draw from, for this layout: the venue's fixtures and
 * the venue. A layout also lists fixtures the player is driving that the
 * venue does not (they have channels, no type and no placement); the stage
 * views have none of those, so neither does this.
 *
 * Geometry arrives as 32-bit floats (see {@link f32}). Nothing here is
 * what an edit is saved from: every save reads the venue file through
 * `GetVenue`, which names the venues directory and holds the numbers whole.
 */
export function layoutMetadata(layout: StageLayout): StageMetadata {
  const fixtures: Record<string, FixtureMetadata> = {};
  for (const f of layout.fixtures) {
    if (f.type === "") continue;
    fixtures[f.name] = {
      tags: [...f.tags],
      type: f.type,
      position: f.position ? vec3(f.position) : null,
      rotation: f.rotation ? vec3(f.rotation) : null,
      beam_angle: f.beamAngle == null ? null : f32(f.beamAngle),
      rig: f.rig ?? null,
      capabilities: f.capabilities as Capability[],
      cells: f.cells.map((c) => ({
        name: c.name,
        offset: c.offset ? vec3(c.offset) : [0, 0, 0],
      })),
    };
  }
  const venue = layout.venue;
  return {
    fixtures,
    venue: venue
      ? {
          name: venue.name,
          focus_points: Object.fromEntries(
            Object.entries(venue.focusPoints).map(([name, p]) => [
              name,
              vec3(p),
            ]),
          ),
          scenery: venue.scenery ?? null,
          scenery_error: venue.sceneryError ?? null,
        }
      : null,
  };
}

/**
 * The stage's state for this frame, read against `layout` and nothing else; null for a frame of another layout, which is dropped: its
 * bytes and indexes are in that layout's order. Throws on a frame that
 * names its layout and does not fit it (a wrong number of bytes, a fixture
 * index past the end), which the player never sends.
 */
export function frameState(
  layout: StageLayout,
  frame: StageFrame,
): StageState | null {
  if (frame.layout !== layout.id) return null;

  const name = (index: number): string => {
    const fixture = layout.fixtures[index];
    if (!fixture) {
      throw new Error(
        `stage frame names fixture ${index} of ${layout.fixtures.length}`,
      );
    }
    return fixture.name;
  };

  const state: StageState = {
    fixtures: {},
    poses: {},
    cells: {},
    strobe_rates: {},
    active_effects: [...frame.activeEffects],
    under_test: frame.underTest.map(name),
  };

  // The bytes: each fixture's channels in the layout's order, end to end.
  const named = layout.fixtures.reduce((n, f) => n + f.channels.length, 0);
  if (named !== frame.channels.length) {
    throw new Error(
      `stage frame has ${frame.channels.length} bytes for a layout of ${named} channels`,
    );
  }
  let at = 0;
  for (const fixture of layout.fixtures) {
    // Where the layout says it points, until the frame says otherwise.
    if (fixture.pose) state.poses[fixture.name] = pose(fixture.pose);
    if (fixture.channels.length === 0) continue;
    const values: Record<string, number> = {};
    for (const channel of fixture.channels) {
      values[channel] = frame.channels[at++];
    }
    state.fixtures[fixture.name] = values;
  }

  for (const change of frame.poses) {
    const fixture = name(change.fixture);
    if (change.pose) state.poses[fixture] = pose(change.pose);
    else delete state.poses[fixture];
  }
  for (const group of frame.cells) {
    const cells = (state.cells[name(group.fixture)] ??= {});
    for (const cell of group.cells) {
      const values: Record<string, number> = {};
      group.channels.forEach((channel, i) => {
        values[channel] = cell.values[i];
      });
      cells[cell.name] = values;
    }
  }
  for (const strobe of frame.strobes) {
    state.strobe_rates[name(strobe.fixture)] = f32(strobe.hertz);
  }
  return state;
}

/** The stage with nothing on it: before a layout, and after a reset. */
export const NO_METADATA: StageMetadata = { fixtures: {}, venue: null };
export const NO_STATE: StageState = {
  fixtures: {},
  poses: {},
  cells: {},
  strobe_rates: {},
  active_effects: [],
  under_test: [],
};

/** Where a reader's stage goes: the stores, for a page. */
export interface StageSink {
  metadata(metadata: StageMetadata): void;
  state(state: StageState): void;
}

/** What became of an update. */
export type StageUpdateOutcome =
  /** A layout: held, and its metadata given to the sink. */
  | "layout"
  /** A frame of the held layout: its state given to the sink. */
  | "frame"
  /** A frame of a layout that is not the one held (or with none held). */
  | "dropped"
  /** An update that is neither: one a newer player may send. */
  | "ignored";

/**
 * One stage stream's reader: holds the layout most recently sent and reads
 * each frame against it.
 */
export class StageReader {
  private layout: StageLayout | null = null;
  private readonly sink: StageSink;

  constructor(sink: StageSink) {
    this.sink = sink;
  }

  /** Takes one update of the stream. Throws on a frame that does not fit
   *  the layout it names. */
  take(update: StageUpdate): StageUpdateOutcome {
    switch (update.update.case) {
      case "layout":
        this.layout = update.update.value;
        this.sink.metadata(layoutMetadata(this.layout));
        return "layout";
      case "frame": {
        const state =
          this.layout && frameState(this.layout, update.update.value);
        if (!state) return "dropped";
        this.sink.state(state);
        return "frame";
      }
      default:
        return "ignored";
    }
  }

  /** Takes one serialized `player.v1.StageUpdate`. Throws on bytes that
   *  are not one. */
  takeBytes(bytes: Uint8Array): StageUpdateOutcome {
    return this.take(fromBinary(StageUpdateSchema, bytes));
  }

  /** Forgets the layout and empties the stage: the stream ended. */
  reset(): void {
    this.layout = null;
    this.sink.metadata(NO_METADATA);
    this.sink.state(NO_STATE);
  }
}

/** Standard-alphabet base64 to bytes. Throws on text that is not base64. */
export function base64Bytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
