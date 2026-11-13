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
// Reconnecting a gRPC-Web stream: the backoff both of the page's streams
// (the stage's `WatchStage` and the dashboard's `Watch`) share.

/** Backoff steps in ms between reconnects; the last repeats. */
export const BACKOFF_MS = [250, 500, 1000, 2000];

export type Sleep = (ms: number, signal: AbortSignal) => Promise<void>;

/** Waits `ms`, or less when `signal` aborts; never rejects. */
export function abortableSleep(ms: number, signal: AbortSignal): Promise<void> {
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

export interface ReconnectOptions {
  /** One connection: opens the stream and reads it until it ends or throws.
   *  Calls `delivered` for every message, which starts the backoff over. */
  session(signal: AbortSignal, delivered: () => void): Promise<void>;
  signal: AbortSignal;
  /** Waits before a reconnect; rejects never (resolves early on abort). */
  sleep?: Sleep;
  /** Backoff steps in ms; the last repeats. */
  backoff?: number[];
  /** Told of the error a connection ended with. */
  onError?(error: unknown): void;
}

/** Runs `session` until aborted, again after a backoff each time it ends or
 *  throws. The backoff starts over after a connection that delivered. */
export async function runReconnecting(o: ReconnectOptions): Promise<void> {
  const sleep = o.sleep ?? abortableSleep;
  const steps = o.backoff ?? BACKOFF_MS;
  let attempt = 0;
  while (!o.signal.aborted) {
    try {
      await o.session(o.signal, () => {
        attempt = 0;
      });
    } catch (error) {
      if (o.signal.aborted) return;
      o.onError?.(error);
    }
    if (o.signal.aborted) return;
    await sleep(steps[Math.min(attempt, steps.length - 1)], o.signal);
    attempt++;
  }
}
