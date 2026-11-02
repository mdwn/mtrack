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
}

/** A save's notice beside "Saved": the reload waiting for playback to stop,
 *  or why it failed. */
export interface ReloadNotice {
  kind: "deferred" | "failed";
  text: string;
}

/** The notice a save's reply calls for, or null when its change is in
 *  effect (or the reply says nothing of a reload). */
export function reloadNotice(
  reply: ConfigReload | null | undefined,
): ReloadNotice | null {
  if (!reply || reply.applied !== false) return null;
  if (reply.reload === "failed") {
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
