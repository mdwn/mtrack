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
  chooseStageFeed,
  dedupingSink,
  runStageStream,
  stageFeedState,
} from "./grpc-feed";

class MemoryStorage {
  data = new Map<string, string>();
  getItem(k: string) {
    return this.data.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.data.set(k, v);
  }
}

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

describe("chooseStageFeed", () => {
  it("defaults to the websocket, with no timing", () => {
    const store = new MemoryStorage();
    expect(chooseStageFeed("", store)).toEqual({ feed: "ws", timing: false });
    expect(store.data.size).toBe(0);
  });

  it("takes the query, remembers it, and times", () => {
    const store = new MemoryStorage();
    expect(chooseStageFeed("?stageFeed=grpc", store)).toEqual({
      feed: "grpc",
      timing: true,
    });
    // Remembered for a load with no query.
    expect(chooseStageFeed("", store)).toEqual({ feed: "grpc", timing: true });
    // And ?stageFeed=ws goes back.
    expect(chooseStageFeed("?stageFeed=ws", store).feed).toBe("ws");
    expect(chooseStageFeed("", store).feed).toBe("ws");
  });

  it("ignores junk values and asks for timing alone", () => {
    const store = new MemoryStorage();
    store.data.set("mtrack.stageFeed", "bogus");
    expect(chooseStageFeed("?stageFeed=nope", store)).toEqual({
      feed: "ws",
      timing: false,
    });
    expect(chooseStageFeed("?stageTiming=1", store)).toEqual({
      feed: "ws",
      timing: true,
    });
  });

  it("works with storage that throws or is missing", () => {
    const broken = {
      getItem() {
        throw new Error("blocked");
      },
      setItem() {
        throw new Error("blocked");
      },
    };
    expect(chooseStageFeed("?stageFeed=grpc", broken).feed).toBe("grpc");
    expect(chooseStageFeed("", broken).feed).toBe("ws");
    expect(chooseStageFeed("?stageFeed=grpc", null).feed).toBe("grpc");
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

describe("the websocket's stage messages when gRPC is active", () => {
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
    stageFeedState.feed = "ws";
    stageFeedState.timing = null;
  });

  const send = (msg: object) =>
    socket.onmessage!({ data: JSON.stringify(msg) });
  const get = async <T>(store: {
    subscribe(f: (v: T) => void): () => void;
  }) => {
    let v!: T;
    store.subscribe((x) => (v = x))();
    return v;
  };
  const stateMsg = {
    type: "state",
    fixtures: { a: { dimmer: 1 } },
    poses: {},
    cells: {},
    strobe_rates: {},
    active_effects: [],
    under_test: [],
  };

  it("applies them on the websocket feed, ignores them on gRPC", async () => {
    const grpc = await import("./grpc-feed");
    const stores = await import("../ws/stores");
    stores.connect();

    grpc.stageFeedState.feed = "ws";
    send(stateMsg);
    send({ type: "metadata", fixtures: { a: { type: "t" } }, venue: null });
    expect(Object.keys(await get(stores.fixtureStore))).toEqual(["a"]);
    expect(Object.keys(await get(stores.metadataStore))).toEqual(["a"]);

    grpc.stageFeedState.feed = "grpc";
    send({ ...stateMsg, fixtures: { b: { dimmer: 2 } } });
    send({ type: "metadata", fixtures: { b: { type: "t" } }, venue: null });
    expect(Object.keys(await get(stores.fixtureStore))).toEqual(["a"]);
    expect(Object.keys(await get(stores.metadataStore))).toEqual(["a"]);

    // The stream's own writes still land, and are timed.
    grpc.stageFeedState.timing = new (
      await import("./timing")
    ).TimingRecorder();
    stores.applyStageState({ fixtures: { c: { dimmer: 3 } } });
    expect(Object.keys(await get(stores.fixtureStore))).toEqual(["c"]);
    expect(grpc.stageFeedState.timing.summary().count).toBe(1);

    // Other websocket messages are untouched.
    send({ type: "logs", lines: [{ level: "INFO", message: "hi" }] });
    expect((await get(stores.logStore)).length).toBe(1);
  });
});
