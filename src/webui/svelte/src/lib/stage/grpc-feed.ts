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
// The stage (plot and 3D) read from the player's gRPC stream `WatchStage`
// over gRPC-Web: the one feed the stage has. One stream per page, opened
// when something first shows the stage and closed a while after the last
// thing that does stops (a page that never shows it never opens it).
//
// Frame arrival timing is recorded with `?stageTiming=1`: in the console,
// `window.__stageTiming.summary()` and `.reset()`.

import type { StageUpdate } from "../../gen/player/v1/player_pb";
import type { StageMetadata } from "../../gen/rules/mtrack_rules";
import { BACKOFF_MS, runReconnecting, type Sleep } from "../grpc/reconnect";
import { StageReader, type StageSink } from "./feed";
import { stageTiming, TimingRecorder } from "./timing";

/** Backoff steps in ms between reconnects; the last repeats. The page's
 *  streams share them (../grpc/reconnect). */
export const STAGE_BACKOFF = BACKOFF_MS;

/** How long the stream stays open after the last watcher leaves, in ms:
 *  long enough that moving between two pages that show the stage (or a page
 *  re-rendering) does not close and reopen it. */
export const STAGE_GRACE_MS = 5000;

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
  sleep?: Sleep;
  /** Backoff steps in ms; the last repeats. */
  backoff?: number[];
  onError?(error: unknown): void;
}

/** Reads the stream into `sink` until aborted, reconnecting with a short
 *  backoff when it errors or ends. The stage is left as it was while
 *  reconnecting; the backoff starts over after a connection that delivered. */
export async function runStageStream(o: StreamOptions): Promise<void> {
  await runReconnecting({
    session: async (signal, delivered) => {
      const reader = new StageReader(o.sink);
      for await (const update of o.open(signal)) {
        reader.take(update);
        delivered();
      }
    },
    signal: o.signal,
    sleep: o.sleep,
    backoff: o.backoff ?? STAGE_BACKOFF,
    onError: o.onError,
  });
}

/** Timers a {@link FeedLease} waits with; the page's own, unless a test
 *  gives others. */
export interface LeaseTimers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

/**
 * Shares one stream between everything that shows the stage: the first
 * `acquire` starts it, and it stops `graceMs` after the last `release`
 * (a new `acquire` in that time keeps it).
 */
export class FeedLease {
  private holders = 0;
  private stop: (() => void) | null = null;
  private closing: unknown = null;
  private readonly start: () => () => void;
  private readonly graceMs: number;
  private readonly timers: LeaseTimers;

  /** `start` opens the stream and returns what closes it. */
  constructor(
    start: () => () => void,
    graceMs = STAGE_GRACE_MS,
    timers: LeaseTimers = {
      set: (fn, ms) => setTimeout(fn, ms),
      clear: (handle) => clearTimeout(handle as number),
    },
  ) {
    this.start = start;
    this.graceMs = graceMs;
    this.timers = timers;
  }

  /** Holds the stream open; the returned function lets go (once). */
  acquire(): () => void {
    this.holders++;
    if (this.closing !== null) {
      this.timers.clear(this.closing);
      this.closing = null;
    }
    this.stop ??= this.start();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--this.holders > 0) return;
      this.closing = this.timers.set(() => {
        this.closing = null;
        if (this.holders === 0) {
          this.stop?.();
          this.stop = null;
        }
      }, this.graceMs);
    };
  }

  /** Whether the stream is open. */
  get open(): boolean {
    return this.stop !== null;
  }
}

/** Whether the page's URL asks for arrival timing (`?stageTiming=1`, before
 *  or after the hash). */
export function wantsStageTiming(
  location: Pick<Location, "search" | "hash">,
): boolean {
  const query = `${location.search}&${location.hash.split("?")[1] ?? ""}`;
  return new URLSearchParams(query).get("stageTiming") === "1";
}

/** Installs `window.__stageTiming` when the page's URL asks for it. */
export function installStageTiming(
  win: Window & { __stageTiming?: unknown } = window,
): void {
  if (!wantsStageTiming(win.location)) return;
  const recorder = new TimingRecorder("grpc");
  stageTiming.recorder = recorder;
  win.__stageTiming = {
    reset: () => recorder.reset(),
    summary: () => recorder.summary(),
  };
}

/** Opens the page's stage stream into `sink`; returns what closes it. The
 *  player sends at its own rate. A page that goes away closes it. */
export function openStageStream(
  sink: StageSink,
  win: Window = window,
): () => void {
  const abort = new AbortController();
  const onHide = () => abort.abort();
  win.addEventListener("pagehide", onHide, { once: true });
  void (async () => {
    const { playerClient } = await import("../grpc/client");
    await runStageStream({
      open: (signal) => playerClient.watchStage({}, { signal }),
      sink: dedupingSink(sink),
      signal: abort.signal,
      onError: (e) => console.warn("stage stream:", e),
    });
  })();
  return () => {
    win.removeEventListener("pagehide", onHide);
    abort.abort();
  };
}
