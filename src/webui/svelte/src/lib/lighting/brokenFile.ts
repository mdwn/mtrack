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

/** `path` with `replace_broken=<files>` added to its query, when given. */
export function withReplaceBroken(path: string, files?: string): string {
  if (!files) return path;
  const join = path.includes("?") ? "&" : "?";
  return `${path}${join}replace_broken=${encodeURIComponent(files)}`;
}

/**
 * Sends a venue or fixture type save or delete. When the server refuses it
 * because the files it would act on do not read (a 409 naming each, by its
 * path in the lighting directory, with its parse error), says so and asks
 * whether to replace (or remove) them; confirmed, the request goes again
 * naming exactly those files, comma-separated. Any other answer, or a
 * refusal the person keeps, is returned as it came.
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
  const broken = body.broken ? (body.broken_files ?? []) : [];
  if (broken.length === 0) return res;
  const tr = get(t);
  const [first] = broken;
  const question =
    broken.length === 1
      ? tr(
          acting === "save"
            ? "lighting.brokenFileReplace"
            : "lighting.brokenFileRemove",
          { values: { file: first.file, error: first.parse_error } },
        )
      : tr("lighting.brokenFilesReplace", {
          values: {
            files: broken.map((f) => `${f.file} (${f.parse_error})`).join("; "),
          },
        });
  const confirmed = await showConfirm(question, {
    danger: true,
    confirmLabel: tr(
      acting === "save"
        ? "lighting.brokenFileReplaceLabel"
        : "lighting.brokenFileRemoveLabel",
    ),
  });
  return confirmed ? send(broken.map((f) => f.file).join(",")) : res;
}
