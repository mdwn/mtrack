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

// A venue or fixture type file that does not read, offered for replacing.

import { get } from "svelte/store";
import { t } from "svelte-i18n";
import type { LightingFileRefusal } from "../../gen/rules/mtrack_rules";
import { showConfirm } from "../dialog.svelte";

/** `path` with `replace_broken=<file>` added to its query, when given. */
export function withReplaceBroken(path: string, file?: string): string {
  if (!file) return path;
  const join = path.includes("?") ? "&" : "?";
  return `${path}${join}replace_broken=${encodeURIComponent(file)}`;
}

/**
 * Sends a venue or fixture type save or delete. When the server refuses it
 * because the file it would act on does not read (a 409 carrying the file
 * and its parse error), says so and asks whether to replace (or remove) that
 * file; confirmed, the request goes again naming exactly that file. Any
 * other answer, or a refusal the person keeps, is returned as it came.
 */
export async function offeringToReplaceBroken(
  send: (replaceBroken?: string) => Promise<Response>,
  acting: "save" | "delete",
): Promise<Response> {
  const res = await send();
  if (res.status !== 409) return res;
  const body: Partial<LightingFileRefusal> = await res
    .clone()
    .json()
    .catch(() => ({}));
  if (!body.broken || !body.file || !body.parse_error) return res;
  const tr = get(t);
  const values = { file: body.file, error: body.parse_error };
  const confirmed = await showConfirm(
    tr(
      acting === "save"
        ? "lighting.brokenFileReplace"
        : "lighting.brokenFileRemove",
      { values },
    ),
    {
      danger: true,
      confirmLabel: tr(
        acting === "save"
          ? "lighting.brokenFileReplaceLabel"
          : "lighting.brokenFileRemoveLabel",
      ),
    },
  );
  return confirmed ? send(body.file) : res;
}
