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

// The stage as a test describes it (the stage's metadata and state, in the
// shapes the web UI's stores hold), and the player's gRPC stream `WatchStage`
// as it would carry it, framed for gRPC-Web. Pure: the mock server holds a
// page's stream open and writes what this encodes into it; specs push a
// message through ./stage.ts.
//
// Encoded with the generated protobuf code, as the player's encoder does
// (src/controller/grpc/stage.rs): a layout names the fixtures and their
// channels, a frame is then bytes in the layout's order. A test's state
// need not name the fixtures its metadata does, nor the other way round, so
// the layout follows what the test last said: a new one whenever the
// metadata is pushed, or a state names other fixtures or channels than the
// layout holds.

import { create, toBinary } from "@bufbuild/protobuf";
import {
  StageUpdateSchema,
  type StageUpdate,
} from "../../src/gen/player/v1/player_pb";
import type {
  FixtureMetadata,
  PoseSnapshot,
  StageState,
  StageVenue,
  Vec3,
} from "../../src/gen/rules/mtrack_rules";

/** What a test pushes: the venue's fixtures and the venue, each saying only
 *  what the test cares about... */
export interface StageMetadataMessage {
  type: "metadata";
  fixtures: Record<string, Partial<FixtureMetadata>>;
  venue?: (Pick<StageVenue, "name"> & Partial<StageVenue>) | null;
}
/** ...or the stage now, likewise. */
export type StageStateMessage = { type: "state" } & Partial<StageState>;
export type StageMessage = StageMetadataMessage | StageStateMessage;

/** The stage with nothing on it. */
const NO_METADATA: Omit<StageMetadataMessage, "type"> = { fixtures: {} };

const NO_STATE: StageState = {
  fixtures: {},
  poses: {},
  cells: {},
  strobe_rates: {},
  active_effects: [],
  under_test: [],
};

const vec3 = (v: Vec3) => ({ x: v[0], y: v[1], z: v[2] });

function pose(p: PoseSnapshot) {
  return {
    pan: p.pan,
    tilt: p.tilt,
    aim: vec3(p.aim),
    floor: p.floor ? { x: p.floor[0], y: p.floor[1] } : undefined,
  };
}

/** One gRPC-Web message frame: a flag byte (0), the length (4 bytes, big
 *  endian), the message. (The mock server's stream is never ended, so it
 *  has no trailer frame, flag 0x80.) */
export function dataFrame(message: Uint8Array): Buffer {
  const header = Buffer.alloc(5);
  header.writeUInt32BE(message.length, 1);
  return Buffer.concat([header, message]);
}

function framed(update: StageUpdate): Buffer {
  return dataFrame(toBinary(StageUpdateSchema, update));
}

/** What one page's stream has said so far, and how to say what a test pushes
 *  next. */
export class StageStream {
  private metadata: Omit<StageMetadataMessage, "type"> = NO_METADATA;
  /** The channel names of each fixture the layout holds, sorted. */
  private channels: Record<string, string[]> = {};
  /** The layout's fixture names, in its order. */
  private order: string[] = [];
  private layoutId = 0;

  /** The frames that put `message` on the stage: a layout when the
   *  metadata is pushed or the state's fixtures differ from the layout's,
   *  and a frame for a state. */
  push(message: StageMessage): Buffer[] {
    if (message.type === "metadata") {
      const { type: _type, ...metadata } = message;
      void _type;
      this.metadata = metadata;
      return [this.layout(this.channels)];
    }
    const { type: _type, ...partial } = message;
    void _type;
    const state: StageState = { ...NO_STATE, ...partial };
    const frames: Buffer[] = [];
    const wanted = this.channelsOf(state);
    if (JSON.stringify(wanted) !== JSON.stringify(this.channels)) {
      frames.push(this.layout(wanted));
    }
    frames.push(this.frame(state));
    return frames;
  }

  private channelsOf(state: StageState): Record<string, string[]> {
    const channels: Record<string, string[]> = {};
    for (const [name, values] of Object.entries(state.fixtures)) {
      channels[name] = Object.keys(values).sort();
    }
    // A fixture only a pose, a cell or a test names is listed too, with no
    // channels.
    for (const name of [
      ...Object.keys(state.poses),
      ...Object.keys(state.cells),
      ...Object.keys(state.strobe_rates),
      ...state.under_test,
    ]) {
      channels[name] ??= [];
    }
    return channels;
  }

  private layout(channels: Record<string, string[]>): Buffer {
    this.channels = channels;
    const names = [
      ...new Set([
        ...Object.keys(this.metadata.fixtures),
        ...Object.keys(channels),
      ]),
    ].sort();
    this.order = names;
    const venue = this.metadata.venue;
    return framed(
      create(StageUpdateSchema, {
        update: {
          case: "layout",
          value: {
            id: ++this.layoutId,
            fixtures: names.map((name) => {
              const meta = this.metadata.fixtures[name];
              return {
                name,
                type: meta?.type ?? "",
                tags: meta?.tags ?? [],
                position: meta?.position ? vec3(meta.position) : undefined,
                rotation: meta?.rotation ? vec3(meta.rotation) : undefined,
                beamAngle: meta?.beam_angle ?? undefined,
                rig: meta?.rig ?? undefined,
                capabilities: meta?.capabilities ?? [],
                cells: (meta?.cells ?? []).map((c) => ({
                  name: c.name,
                  offset: vec3(c.offset),
                })),
                channels: channels[name] ?? [],
              };
            }),
            venue: venue
              ? {
                  name: venue.name,
                  focusPoints: Object.fromEntries(
                    Object.entries(venue.focus_points ?? {}).map(([n, p]) => [
                      n,
                      vec3(p),
                    ]),
                  ),
                  scenery: venue.scenery ?? undefined,
                  sceneryError: venue.scenery_error ?? undefined,
                }
              : undefined,
          },
        },
      }),
    );
  }

  private frame(state: StageState): Buffer {
    const index = (name: string) => this.order.indexOf(name);
    const bytes: number[] = [];
    for (const name of this.order) {
      for (const channel of this.channels[name] ?? []) {
        bytes.push(state.fixtures[name][channel]);
      }
    }
    return framed(
      create(StageUpdateSchema, {
        update: {
          case: "frame",
          value: {
            layout: this.layoutId,
            channels: new Uint8Array(bytes),
            strobes: Object.entries(state.strobe_rates).map(
              ([name, hertz]) => ({ fixture: index(name), hertz }),
            ),
            poses: Object.entries(state.poses).map(([name, p]) => ({
              fixture: index(name),
              pose: pose(p),
            })),
            cells: Object.entries(state.cells).map(([name, cells]) => {
              const channels = [
                ...new Set(
                  Object.values(cells).flatMap((values) => Object.keys(values)),
                ),
              ].sort();
              return {
                fixture: index(name),
                channels,
                cells: Object.entries(cells).map(([cell, values]) => ({
                  name: cell,
                  values: new Uint8Array(channels.map((c) => values[c] ?? 0)),
                })),
              };
            }),
            activeEffects: state.active_effects,
            underTest: state.under_test.map(index),
          },
        },
      }),
    );
  }
}
