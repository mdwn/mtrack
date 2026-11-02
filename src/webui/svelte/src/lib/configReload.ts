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
  /** For a deferred reload, the deferred reload generation it waits for:
   *  its wait is over once the playback updates' started reaches it. */
  awaits_reload?: number;
}

/** A save's notice beside "Saved": the reload waiting for playback to stop,
 *  or why it failed. */
export interface ReloadNotice {
  kind: "deferred" | "failed";
  text: string;
  /** A song reload that failed beside a deferred hardware reload: said
   *  with it, and kept once the hardware reload has started. */
  songsError?: string;
  /** For a deferred notice, the deferred reload generation it waits for. */
  awaits?: number;
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
      awaits: reply.awaits_reload,
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
    awaits: reply.awaits_reload,
  };
}

/** The deferred hardware reloads, as the playback updates count them: the
 *  last generation started (or refused), and the last refused, with why. */
export interface DeferredReloads {
  started: number;
  refused: { generation: number; error: string } | null;
}

/** What a deferred notice becomes, given the deferred reloads: the same
 *  notice until the generation it waits for has started, then nothing, or
 *  the failures that leaves -- the reload refused (it could not start at
 *  all) and the songs failure the save carried, both when there are both.
 *  Only the wait for playback to stop is followed: what the reload then
 *  makes of each device is shown per subsystem on the status page. Any
 *  other notice is answered as it is. */
export function settleNotice(
  notice: ReloadNotice | null,
  reloads: DeferredReloads,
): ReloadNotice | null {
  if (notice?.kind !== "deferred" || notice.awaits === undefined) return notice;
  if (reloads.started < notice.awaits) return notice;
  const failures: string[] = [];
  if (reloads.refused && reloads.refused.generation >= notice.awaits) {
    failures.push(
      get(t)("config.reloadRefused", {
        values: { reason: reloads.refused.error },
        default: `The hardware reload could not start: ${reloads.refused.error}`,
      }),
    );
  }
  if (notice.songsError) failures.push(notice.songsError);
  return failures.length > 0
    ? { kind: "failed", text: failures.join(" ") }
    : null;
}
