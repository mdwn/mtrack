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

import { get } from "svelte/store";
import { t } from "svelte-i18n";

/** What a config or profile save's reply says of the hardware reload: it
 *  ran (`applied`), waits until playback stops (`deferred`), or could not
 *  run (`failed`), with the server's `message` when it was not applied. A
 *  save while a song plays is kept and its reload deferred, so "Saved"
 *  alone would read as a change already in effect. */
export interface ConfigReload {
  applied?: boolean;
  reload?: "applied" | "deferred" | "failed";
  message?: string;
  /** A song reload beside it that failed: the player keeps its songs. */
  songs_error?: string;
}

/** A save's notice beside "Saved": the reload waiting for playback to stop,
 *  or why it failed. */
export interface ReloadNotice {
  kind: "deferred" | "failed";
  text: string;
  /** A song reload that failed beside a deferred hardware reload: said
   *  with it, and kept once the hardware reload has run. */
  songsError?: string;
}

/** The notice a save's reply calls for, or null when its change is in
 *  effect (or the reply says nothing of a reload). */
export function reloadNotice(
  reply: ConfigReload | null | undefined,
): ReloadNotice | null {
  if (!reply || reply.applied !== false) return null;
  const songsError = reply.songs_error
    ? get(t)("config.songsNotReloaded", {
        values: { reason: reply.songs_error },
        default: `The songs could not be reloaded: ${reply.songs_error}`,
      })
    : undefined;
  // Deferred, with the songs failed beside it: a deferred notice still, so
  // its reload is followed, saying both.
  if (reply.reload === "deferred" && songsError) {
    return {
      kind: "deferred",
      text: `${get(t)("config.reloadDeferred", {
        default: "Saved. The hardware reloads when playback stops.",
      })} ${songsError}`,
      songsError,
    };
  }
  if (reply.reload === "failed" || reply.songs_error) {
    return {
      kind: "failed",
      text:
        reply.message ??
        get(t)("config.reloadFailed", {
          default: "Saved, but the hardware did not reload.",
        }),
    };
  }
  return {
    kind: "deferred",
    text: get(t)("config.reloadDeferred", {
      default: "Saved. The hardware reloads when playback stops.",
    }),
  };
}

/** What became of a deferred reload, once it has run: done, or failed and
 *  why. */
export type DeferredOutcome =
  | { failed: false }
  | { failed: true; reason: string };

/** Follows a deferred notice's hardware reload through the playback
 *  updates: `hardware_reload_pending` seen set and then clear again is the
 *  reload over, and `hardware_reload_failed` then says whether it failed.
 *  Feed it each update. */
export class DeferredReloadWatch {
  private seenPending = false;

  /** The deferred reload's outcome, once it has run, while a deferred
   *  notice is shown (`showing`); null until then. */
  settle(
    showing: boolean,
    pending: boolean,
    failed: string | null,
  ): DeferredOutcome | null {
    if (!showing) {
      this.seenPending = false;
      return null;
    }
    if (pending) {
      this.seenPending = true;
      return null;
    }
    if (!this.seenPending) return null;
    this.seenPending = false;
    return failed ? { failed: true, reason: failed } : { failed: false };
  }
}

/** What a deferred notice becomes once its reload has run: the reload's
 *  failure, else the songs' failure it carried, else nothing. */
export function settledNotice(
  notice: ReloadNotice,
  outcome: DeferredOutcome,
): ReloadNotice | null {
  if (outcome.failed) return deferredFailedNotice(outcome.reason);
  return notice.songsError ? { kind: "failed", text: notice.songsError } : null;
}

/** The notice for a deferred reload that has failed. */
export function deferredFailedNotice(reason: string): ReloadNotice {
  return {
    kind: "failed",
    text: get(t)("config.reloadFailedLater", {
      values: { reason },
      default: `The hardware reload failed: ${reason}`,
    }),
  };
}
