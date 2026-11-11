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

// The gRPC stage stream read into what the websocket says (no browser):
// lib/stage/feed.ts against a stream the player's own encoder wrote, each
// update beside the websocket message for the same stage
// (e2e/fixtures/stage-stream.json, written and held to the encoder by a
// test in src/controller/grpc/stage.rs).

import type {
  StageMetadata,
  StageState,
} from "../../src/gen/rules/mtrack_rules";
import { test, expect } from "@playwright/test";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { get } from "svelte/store";
import {
  StageUpdateSchema,
  type StageFrame,
  type StageLayout,
} from "../../src/gen/player/v1/player_pb";
import {
  base64Bytes,
  frameState,
  layoutMetadata,
  StageReader,
} from "../../src/lib/stage/feed";
import {
  applyStageMetadata,
  applyStageState,
  cellStore,
  effectsStore,
  fixtureStore,
  metadataStore,
  poseStore,
  strobeRateStore,
  underTestStore,
  venueStore,
} from "../../src/lib/ws/stores";

interface Step {
  what: string;
  /** One serialized StageUpdate, base64: what a host hands the page. */
  update: string;
  /** The websocket's `metadata` message for the same stage (a layout). */
  metadata?: { type: "metadata" } & StageMetadata;
  /** The websocket's `state` message for the same stage (a frame). */
  state?: { type: "state" } & StageState;
  /** A frame of a layout the reader does not hold. */
  dropped?: boolean;
}

const steps: Step[] = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "e2e/fixtures/stage-stream.json"),
    "utf8",
  ),
).steps;

/** The stream's geometry and rates are 32-bit floats; the JSON's are not.
 *  Every number of a JSON message narrowed to what the stream can say (a
 *  channel's byte is the same either way). */
function narrowed<T>(value: T): T {
  if (typeof value === "number") return Math.fround(value) as T;
  if (Array.isArray(value)) return value.map(narrowed) as T;
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, narrowed(v)]),
    ) as T;
  }
  return value;
}

/** What a `metadata` message leaves in the stores, as the stream can say
 *  it: narrowed, and without the venues directory, which stays home. */
function ofMetadata(message: NonNullable<Step["metadata"]>): StageMetadata {
  return narrowed({
    fixtures: message.fixtures,
    venue: message.venue && { ...message.venue, dir: null },
  });
}

function ofState(message: NonNullable<Step["state"]>): StageState {
  const { type: _type, ...state } = message;
  void _type;
  return narrowed(state);
}

function decode(step: Step) {
  return fromBinary(StageUpdateSchema, base64Bytes(step.update));
}
function layoutOf(step: Step): StageLayout {
  const update = decode(step).update;
  if (update.case !== "layout") throw new Error(`${step.what}: not a layout`);
  return update.value;
}
function frameOf(step: Step): StageFrame {
  const update = decode(step).update;
  if (update.case !== "frame") throw new Error(`${step.what}: not a frame`);
  return update.value;
}

/** A reader whose sink keeps what it was last given. */
function reading() {
  const seen: { metadata?: StageMetadata; state?: StageState; writes: number } =
    { writes: 0 };
  const reader = new StageReader({
    metadata: (m) => {
      seen.metadata = m;
      seen.writes++;
    },
    state: (s) => {
      seen.state = s;
      seen.writes++;
    },
  });
  return { reader, seen };
}

const [first, unmoved, changed, unposed, early, second, , late] = steps;

test.describe("the stage stream, read as the websocket's messages", () => {
  test("the recording is the one these tests were written for", () => {
    expect(steps.map((s) => s.what)).toEqual([
      "the first layout",
      "a frame with nothing moved: the mover points where the layout says",
      "values, the pose, the strobes, the cells and the test all change",
      "the mover has no pose now, though the layout gave it one",
      "a frame of a layout that has not arrived",
      "the second layout: one more driven fixture, the same venue",
      "a frame of the second layout",
      "a late frame of the first layout",
    ]);
  });

  test("every update of the stream reads as the websocket's message for the same stage", () => {
    const { reader, seen } = reading();
    for (const step of steps) {
      const writes = seen.writes;
      const outcome = reader.takeBytes(base64Bytes(step.update));
      if (step.metadata) {
        expect(outcome, step.what).toBe("layout");
        expect(seen.metadata, step.what).toEqual(ofMetadata(step.metadata));
      } else if (step.state) {
        expect(outcome, step.what).toBe("frame");
        expect(seen.state, step.what).toEqual(ofState(step.state));
      } else {
        expect(step.dropped, step.what).toBe(true);
        expect(outcome, step.what).toBe("dropped");
        expect(seen.writes, step.what).toBe(writes);
      }
    }
  });

  test("a layout is the metadata message: the venue's fixtures and the venue", () => {
    const layout = layoutOf(first);
    const metadata = layoutMetadata(layout);
    expect(metadata).toEqual(ofMetadata(first.metadata!));

    // The layout also lists a fixture the player drives that the venue
    // does not; the metadata message has only the venue's.
    expect(layout.fixtures.map((f) => f.name)).toEqual([
      "bar",
      "head",
      "spare",
      "stray",
    ]);
    expect(Object.keys(metadata.fixtures).sort()).toEqual([
      "bar",
      "head",
      "spare",
    ]);
    expect(metadata.fixtures.head).toEqual({
      tags: ["back", "mover"],
      type: "Head",
      position: [-2, 3.5, 4.25],
      rotation: [0, 0, 180],
      beam_angle: 12.5,
      rig: "rigs/head.glb",
      capabilities: ["color", "pan_tilt", "color_wheel"],
      cells: [],
    });
    // What the venue does not give is null, as the JSON has it.
    expect(metadata.fixtures.spare).toEqual({
      tags: [],
      type: "Par",
      position: null,
      rotation: null,
      beam_angle: null,
      rig: null,
      capabilities: [],
      cells: [],
    });
    expect(metadata.fixtures.bar.cells).toEqual([
      { name: "1", offset: [-0.25, 0, 0] },
      { name: "2", offset: [0.25, 0, 0] },
    ]);
    expect(metadata.venue).toEqual({
      name: "club",
      dir: null,
      focus_points: { drummer: [0, 2.75, 1.5] },
      scenery: "scenery/club.glb",
      scenery_error: null,
    });
    // The JSON said where the venues are kept; the stream does not.
    expect(first.metadata!.venue!.dir).toBe("/somewhere/venues");
  });

  test("a layout with no venue is metadata with none", () => {
    const layout = layoutOf(first);
    layout.venue = undefined;
    layout.fixtures = [];
    expect(layoutMetadata(layout)).toEqual({ fixtures: {}, venue: null });
  });

  test("a frame is the state message: bytes by the layout's channel names", () => {
    const state = frameState(layoutOf(first), frameOf(unmoved));
    expect(state).toEqual(ofState(unmoved.state!));
    expect(state!.fixtures).toEqual({
      bar: { dimmer: 255, red: 10 },
      head: { dimmer: 1, pan: 2, tilt: 3 },
      stray: { dimmer: 77 },
    });
    expect(state!.active_effects).toEqual(["chase", "wash"]);
  });

  test("a fixture the frame gives no pose keeps the layout's", () => {
    const layout = layoutOf(first);
    const frame = frameOf(unmoved);
    // Nothing has moved since the layout, so the frame says no poses...
    expect(frame.poses).toEqual([]);
    // ...and the mover points where the layout has it; the others, which
    // the layout gives no pose, have none.
    const state = frameState(layout, frame)!;
    expect(state.poses).toEqual(narrowed(unmoved.state!.poses));
    expect(Object.keys(state.poses)).toEqual(["head"]);
    expect(state.poses.head.pan).toBe(90);
    expect(state.poses.head.floor).toEqual([1.5, -2]);
  });

  test("a pose in the frame replaces the layout's, and one with no pose means none", () => {
    const layout = layoutOf(first);

    const moved = frameState(layout, frameOf(changed))!;
    expect(moved.poses.head.pan).toBe(120);
    // A beam with no footprint on the deck.
    expect(moved.poses.head.floor).toBeNull();
    expect(moved.poses).toEqual(narrowed(changed.state!.poses));

    const frame = frameOf(unposed);
    expect(frame.poses.map((p) => [p.fixture, p.pose])).toEqual([
      [1, undefined],
    ]);
    expect(frameState(layout, frame)!.poses).toEqual({});
    expect(unposed.state!.poses).toEqual({});
  });

  test("cells, strobe rates and fixtures under test go by the layout's names", () => {
    const layout = layoutOf(first);

    const before = frameState(layout, frameOf(unmoved))!;
    expect(before.cells).toEqual({ bar: { "1": { red: 7 }, "2": { red: 9 } } });
    expect(before.strobe_rates).toEqual({ head: 12.5 });
    expect(before.under_test).toEqual(["head"]);

    const after = frameState(layout, frameOf(changed))!;
    expect(after.cells).toEqual({});
    expect(after.strobe_rates).toEqual({ stray: 3 });
    expect(after.under_test).toEqual(["bar", "stray"]);
    expect(after.active_effects).toEqual([]);
  });

  test("a frame of a layout that is not the one held is dropped", () => {
    const one = layoutOf(first);
    const two = layoutOf(second);
    expect([one.id, two.id]).toEqual([1, 2]);
    expect(frameOf(early).layout).toBe(2);
    expect(frameOf(late).layout).toBe(1);
    expect(frameState(one, frameOf(early))).toBeNull();
    expect(frameState(two, frameOf(late))).toBeNull();
    expect(frameState(two, frameOf(early))).not.toBeNull();

    // And with no layout held at all.
    const { reader, seen } = reading();
    expect(reader.takeBytes(base64Bytes(unmoved.update))).toBe("dropped");
    expect(seen.writes).toBe(0);
  });

  test("a reset forgets the layout and empties the stage", () => {
    const { reader, seen } = reading();
    reader.takeBytes(base64Bytes(first.update));
    reader.takeBytes(base64Bytes(unmoved.update));
    reader.reset();
    expect(seen.metadata).toEqual({ fixtures: {}, venue: null });
    expect(seen.state).toEqual({
      fixtures: {},
      poses: {},
      cells: {},
      strobe_rates: {},
      active_effects: [],
      under_test: [],
    });
    // The old layout's frames mean nothing now.
    expect(reader.takeBytes(base64Bytes(unmoved.update))).toBe("dropped");
  });

  test("what is not an update, or does not fit its layout, is an error", () => {
    const { reader } = reading();
    expect(() => base64Bytes("not base64!")).toThrow();
    expect(() =>
      reader.takeBytes(new Uint8Array([0xff, 0xff, 0xff])),
    ).toThrow();

    const layout = layoutOf(first);
    const short = frameOf(unmoved);
    short.channels = short.channels.slice(1);
    expect(() => frameState(layout, short)).toThrow(/5 bytes .* 6 channels/);
    const beyond = frameOf(unmoved);
    beyond.underTest = [9];
    expect(() => frameState(layout, beyond)).toThrow(/fixture 9 of 4/);

    // An update that is neither a layout nor a frame is let by.
    const neither = toBinary(StageUpdateSchema, create(StageUpdateSchema));
    expect(reader.takeBytes(neither)).toBe("ignored");
  });
});

test.describe("the stage stores, from either source", () => {
  const stores = () => ({
    metadata: get(metadataStore),
    venue: get(venueStore),
    fixtures: get(fixtureStore),
    poses: get(poseStore),
    cells: get(cellStore),
    strobe_rates: get(strobeRateStore),
    active_effects: get(effectsStore),
    under_test: get(underTestStore),
  });

  // Importing the stores opened no websocket: there is none to open here
  // (no `location`), and the import at the top of this file did not throw.
  test("the stream and the websocket end in the same store writes", () => {
    const reader = new StageReader({
      metadata: applyStageMetadata,
      state: applyStageState,
    });
    for (const step of steps) {
      if (step.dropped) {
        const before = stores();
        reader.takeBytes(base64Bytes(step.update));
        expect(stores(), step.what).toEqual(before);
        continue;
      }
      reader.takeBytes(base64Bytes(step.update));
      const fromStream = stores();

      // The same stage as the websocket says it, through the handlers'
      // own functions.
      if (step.metadata) {
        applyStageMetadata({
          ...step.metadata,
          venue: step.metadata.venue && { ...step.metadata.venue, dir: null },
        });
      }
      if (step.state) applyStageState(step.state);
      expect(fromStream, step.what).toEqual(narrowed(stores()));
    }
    // The recording ends on the second layout's frame.
    expect(Object.keys(stores().fixtures).sort()).toEqual([
      "bar",
      "extra",
      "head",
      "stray",
    ]);
    expect(stores().venue?.name).toBe("club");
  });
});
