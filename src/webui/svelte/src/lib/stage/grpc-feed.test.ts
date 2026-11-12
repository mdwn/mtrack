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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "@bufbuild/protobuf";
import {
  StageFixtureSchema,
  StageFrameSchema,
  StageLayoutSchema,
  StageUpdateSchema,
  type StageUpdate,
} from "../../gen/player/v1/player_pb";
import type { StageMetadata, StageState } from "../../gen/rules/mtrack_rules";
import {
  dedupingSink,
  FeedLease,
  runStageStream,
  wantsStageTiming,
  type LeaseTimers,
} from "./grpc-feed";

function layout(id: number, venue = "v"): StageUpdate {
  return create(StageUpdateSchema, {
    update: {
      case: "layout",
      value: create(StageLayoutSchema, {
        id,
        venue: { name: venue },
        fixtures: [
          create(StageFixtureSchema, {
            name: "wash",
            type: "dimmer",
            channels: ["dimmer"],
          }),
        ],
      }),
    },
  });
}

function frame(layoutId: number, dimmer: number): StageUpdate {
  return create(StageUpdateSchema, {
    update: {
      case: "frame",
      value: create(StageFrameSchema, {
        layout: layoutId,
        channels: new Uint8Array([dimmer]),
      }),
    },
  });
}

function recorder() {
  const metadata: StageMetadata[] = [];
  const state: StageState[] = [];
  return {
    metadata,
    state,
    sink: {
      metadata: (m: StageMetadata) => metadata.push(m),
      state: (s: StageState) => state.push(s),
    },
  };
}

describe("wantsStageTiming", () => {
  it("is asked for with ?stageTiming=1, before or after the hash", () => {
    expect(wantsStageTiming({ search: "", hash: "" })).toBe(false);
    expect(wantsStageTiming({ search: "?stageTiming=1", hash: "" })).toBe(true);
    expect(
      wantsStageTiming({ search: "", hash: "#/lights?stageTiming=1" }),
    ).toBe(true);
    expect(wantsStageTiming({ search: "?stageTiming=0", hash: "" })).toBe(
      false,
    );
    // The old switch is gone: it asks for nothing.
    expect(wantsStageTiming({ search: "?stageFeed=grpc", hash: "" })).toBe(
      false,
    );
  });
});

/** Timers that run only when a test says. */
function manualTimers() {
  const pending = new Map<number, () => void>();
  let next = 1;
  const timers: LeaseTimers = {
    set(fn) {
      pending.set(next, fn);
      return next++;
    },
    clear(handle) {
      pending.delete(handle as number);
    },
  };
  return {
    timers,
    pending: () => pending.size,
    fire() {
      const fns = [...pending.values()];
      pending.clear();
      fns.forEach((fn) => fn());
    },
  };
}

describe("FeedLease", () => {
  it("opens once for any number of holders, and closes after the grace", () => {
    const t = manualTimers();
    let opened = 0;
    let closed = 0;
    const lease = new FeedLease(
      () => {
        opened++;
        return () => closed++;
      },
      5000,
      t.timers,
    );
    expect(lease.open).toBe(false);
    const a = lease.acquire();
    const b = lease.acquire();
    expect(opened).toBe(1);
    expect(lease.open).toBe(true);

    a();
    a(); // letting go twice counts once
    expect(t.pending()).toBe(0);
    b();
    expect(t.pending()).toBe(1);
    expect(closed).toBe(0);
    t.fire();
    expect(closed).toBe(1);
    expect(lease.open).toBe(false);
  });

  it("keeps the stream when someone holds it again within the grace", () => {
    const t = manualTimers();
    let opened = 0;
    let closed = 0;
    const lease = new FeedLease(
      () => {
        opened++;
        return () => closed++;
      },
      5000,
      t.timers,
    );
    lease.acquire()();
    expect(t.pending()).toBe(1);
    const again = lease.acquire();
    expect(t.pending()).toBe(0);
    t.fire();
    expect(opened).toBe(1);
    expect(closed).toBe(0);

    // And opens afresh after a close.
    again();
    t.fire();
    expect(closed).toBe(1);
    lease.acquire();
    expect(opened).toBe(2);
  });
});

describe("the stream into the stores' sink", () => {
  it("applies a layout then a frame", async () => {
    const r = recorder();
    const abort = new AbortController();
    await runStageStream({
      open: async function* () {
        yield layout(1);
        yield frame(1, 200);
        yield frame(2, 9); // another layout's: dropped
        abort.abort();
      },
      sink: r.sink,
      signal: abort.signal,
      sleep: async () => {},
    });
    expect(r.metadata).toHaveLength(1);
    expect(Object.keys(r.metadata[0].fixtures)).toEqual(["wash"]);
    expect(r.state).toHaveLength(1);
    expect(r.state[0].fixtures.wash).toEqual({ dimmer: 200 });
  });

  it("reconnects with backoff after an error and after an end", async () => {
    const r = recorder();
    const abort = new AbortController();
    const waits: number[] = [];
    let opened = 0;
    const errors: unknown[] = [];
    await runStageStream({
      open: async function* () {
        opened++;
        if (opened === 1) throw new Error("boom");
        if (opened === 2) return; // ends with nothing
        yield layout(1);
        yield frame(1, 7);
        if (opened === 3) return;
        abort.abort();
      },
      sink: r.sink,
      signal: abort.signal,
      backoff: [10, 20, 40],
      sleep: async (ms) => {
        waits.push(ms);
      },
      onError: (e) => errors.push(e),
    });
    expect(opened).toBe(4);
    expect(errors).toHaveLength(1);
    // 1st failure 10, 2nd (empty end) 20, then a delivering connection
    // starts the backoff over.
    expect(waits).toEqual([10, 20, 10]);
    expect(r.state.map((s) => s.fixtures.wash.dimmer)).toEqual([7, 7]);
  });

  it("does not reconnect once aborted", async () => {
    const abort = new AbortController();
    let opened = 0;
    await runStageStream({
      open: async function* () {
        opened++;
        abort.abort();
        yield layout(1);
        throw new Error("aborted");
      },
      sink: recorder().sink,
      signal: abort.signal,
      sleep: async () => {
        throw new Error("should not sleep");
      },
    });
    expect(opened).toBe(1);
  });

  it("does not re-apply an identical layout, also across reconnects", async () => {
    const r = recorder();
    const sink = dedupingSink(r.sink);
    const abort = new AbortController();
    let opened = 0;
    await runStageStream({
      open: async function* () {
        opened++;
        yield layout(1);
        yield layout(2); // same venue and fixtures, new id
        yield frame(2, 1);
        if (opened === 2) {
          yield layout(1, "other");
          abort.abort();
        }
      },
      sink,
      signal: abort.signal,
      sleep: async () => {},
    });
    expect(opened).toBe(2);
    expect(r.metadata.map((m) => m.venue?.name)).toEqual(["v", "other"]);
    expect(r.state).toHaveLength(2);
  });
});

describe("the stage stores", () => {
  type Sock = { onmessage?: (e: { data: string }) => void };
  let socket: Sock;

  beforeEach(() => {
    vi.resetModules();
    vi.stubGlobal("location", { protocol: "http:", host: "x", search: "" });
    vi.stubGlobal(
      "WebSocket",
      class {
        constructor() {
          socket = this as unknown as Sock;
        }
        close() {}
      },
    );
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const send = (msg: object) =>
    socket.onmessage!({ data: JSON.stringify(msg) });
  const get = <T>(store: { subscribe(f: (v: T) => void): () => void }) => {
    let v!: T;
    store.subscribe((x) => (v = x))();
    return v;
  };

  it("ignore the websocket's old stage messages and are timed on the stream's writes", async () => {
    const stores = await import("../ws/stores");
    stores.connect();

    send({
      type: "state",
      fixtures: { a: { dimmer: 1 } },
      poses: {},
      cells: {},
      strobe_rates: {},
      active_effects: [],
      under_test: [],
    });
    send({ type: "metadata", fixtures: { a: { type: "t" } }, venue: null });
    expect(get(stores.fixtureStore)).toEqual({});
    expect(get(stores.metadataStore)).toEqual({});

    // (The modules are loaded afresh for each test: its own timing.)
    const timing = await import("./timing");
    timing.stageTiming.recorder = new timing.TimingRecorder();
    stores.applyStageState({ fixtures: { c: { dimmer: 3 } } });
    expect(Object.keys(get(stores.fixtureStore))).toEqual(["c"]);
    expect(timing.stageTiming.recorder.summary().count).toBe(1);

    // Other websocket messages are untouched.
    send({ type: "logs", lines: [{ level: "INFO", message: "hi" }] });
    expect(get(stores.logStore).length).toBe(1);
  });

  it("hold the stream open while watched, and open nothing with no lease", async () => {
    const stores = await import("../ws/stores");
    // No lease provided (the stage-only page): watching opens nothing.
    stores.metadataStore.subscribe(() => {})();

    const t = manualTimers();
    let opened = 0;
    let closed = 0;
    stores.provideStageFeed(
      new FeedLease(
        () => {
          opened++;
          return () => closed++;
        },
        5000,
        t.timers,
      ),
    );
    // A store that is not the stage's does not hold it.
    stores.playbackStore.subscribe(() => {})();
    expect(opened).toBe(0);

    const stop1 = stores.metadataStore.subscribe(() => {});
    const stop2 = stores.fixtureStore.subscribe(() => {});
    expect(opened).toBe(1);
    stop1();
    stop2();
    expect(closed).toBe(0);
    t.fire();
    expect(closed).toBe(1);
  });
});
