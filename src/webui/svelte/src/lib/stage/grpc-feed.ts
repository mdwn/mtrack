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
// SPIKE: the stage (plot and 3D) reading the player's gRPC stream
// `WatchStage` over gRPC-Web instead of the websocket's `metadata` and
// `state` messages, so the two can be timed against each other.
//
// To switch, load the web UI with `?stageFeed=grpc` (or `?stageFeed=ws` to
// go back). The choice is remembered in localStorage; the default is the
// websocket. Frame arrival timing is recorded when the switch has been used
// or with `?stageTiming=1`: in the console, `window.__stageTiming.summary()`
// and `.reset()`. Everything else on the websocket carries on either way.

import type { StageUpdate } from "../../gen/player/v1/player_pb";
import type { StageMetadata } from "../../gen/rules/mtrack_rules";
import { StageReader, type StageSink } from "./feed";
import { TimingRecorder } from "./timing";

export type StageFeed = "ws" | "grpc";

/** The frame rate asked of the stream: the player sends at its sampler's
 *  rate when that is lower (the request is a ceiling). */
export const STAGE_MAX_RATE = 1000;

const STORAGE_KEY = "mtrack.stageFeed";

export interface FeedChoice {
  feed: StageFeed;
  /** Timing is recorded: the switch was used (now or before), or asked. */
  timing: boolean;
}

interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const asFeed = (v: string | null | undefined): StageFeed | null =>
  v === "grpc" || v === "ws" ? v : null;

/** Reads the switch from the query (`search`) and the remembered choice;
 *  remembers a choice given in the query. Storage failures are ignored. */
export function chooseStageFeed(
  search: string,
  storage: KeyValueStore | null,
): FeedChoice {
  const query = new URLSearchParams(search);
  let stored: StageFeed | null = null;
  try {
    stored = asFeed(storage?.getItem(STORAGE_KEY));
  } catch {
    // No storage: the query and the default decide.
  }
  const asked = asFeed(query.get("stageFeed"));
  if (asked && asked !== stored) {
    try {
      storage?.setItem(STORAGE_KEY, asked);
    } catch {
      // Not remembered.
    }
  }
  const feed = asked ?? stored ?? "ws";
  return {
    feed,
    timing:
      asked !== null || stored !== null || query.get("stageTiming") === "1",
  };
}

/** Gates the websocket's stage messages and records state arrivals. */
export const stageFeedState = {
  feed: "ws" as StageFeed,
  timing: null as TimingRecorder | null,
  /** Whether the websocket's `metadata` and `state` messages apply. */
  websocketStage(): boolean {
    return stageFeedState.feed === "ws";
  },
  /** Called for each stage state applied, from either feed. */
  stateApplied(): void {
    stageFeedState.timing?.record(performance.now());
  },
};

/** The sink that skips a layout whose metadata is what was last applied, so
 *  a resent layout does not rebuild the 3D scene. */
export function dedupingSink(sink: StageSink): StageSink {
  let last: string | null = null;
  return {
    metadata(m: StageMetadata) {
      const json = JSON.stringify(m);
      if (json === last) return;
      last = json;
      sink.metadata(m);
    },
    state: (s) => sink.state(s),
  };
}

export interface StreamOptions {
  /** Opens the stream; ends or throws when it breaks. */
  open(signal: AbortSignal): AsyncIterable<StageUpdate>;
  sink: StageSink;
  signal: AbortSignal;
  /** Waits before a reconnect; rejects never (resolves early on abort). */
  sleep?(ms: number, signal: AbortSignal): Promise<void>;
  /** Backoff steps in ms; the last repeats. */
  backoff?: number[];
  onError?(error: unknown): void;
}

function abortableSleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) return resolve();
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", done);
      resolve();
    }, ms);
    signal.addEventListener("abort", done, { once: true });
  });
}

/** Reads the stream into `sink` until aborted, reconnecting with a short
 *  backoff when it errors or ends. The stage is left as it was while
 *  reconnecting; the backoff starts over after a connection that delivered. */
export async function runStageStream(o: StreamOptions): Promise<void> {
  const sleep = o.sleep ?? abortableSleep;
  const steps = o.backoff ?? [250, 500, 1000, 2000];
  let attempt = 0;
  while (!o.signal.aborted) {
    const reader = new StageReader(o.sink);
    try {
      for await (const update of o.open(o.signal)) {
        reader.take(update);
        attempt = 0;
      }
    } catch (error) {
      if (o.signal.aborted) return;
      o.onError?.(error);
    }
    if (o.signal.aborted) return;
    await sleep(steps[Math.min(attempt, steps.length - 1)], o.signal);
    attempt++;
  }
}

/** Starts the chosen feed for the page: installs `window.__stageTiming` when
 *  timing is on, and opens the gRPC stream when gRPC is chosen. Returns the
 *  choice. Call once, before the websocket connects. */
export async function startStageFeed(
  sink: StageSink,
  win: Window & { __stageTiming?: unknown } = window,
): Promise<FeedChoice> {
  let storage: Storage | null = null;
  try {
    storage = win.localStorage;
  } catch {
    // Blocked storage.
  }
  // The query may sit before or after the hash (`/#/lights?stageFeed=grpc`).
  const hashQuery = win.location.hash.split("?")[1] ?? "";
  const choice = chooseStageFeed(
    `${win.location.search}&${hashQuery}`,
    storage,
  );
  stageFeedState.feed = choice.feed;
  if (choice.timing) {
    const recorder = new TimingRecorder(choice.feed);
    stageFeedState.timing = recorder;
    win.__stageTiming = {
      feed: choice.feed,
      reset: () => recorder.reset(),
      summary: () => recorder.summary(),
    };
  }
  if (choice.feed === "grpc") {
    const { playerClient } = await import("../grpc/client");
    const abort = new AbortController();
    win.addEventListener("pagehide", () => abort.abort(), { once: true });
    void runStageStream({
      open: (signal) =>
        playerClient.watchStage({ maxRate: STAGE_MAX_RATE }, { signal }),
      sink: dedupingSink(sink),
      signal: abort.signal,
      onError: (e) => console.warn("stage stream:", e),
    });
  }
  return choice;
}
