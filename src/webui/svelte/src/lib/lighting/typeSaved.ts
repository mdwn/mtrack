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

import { t } from "svelte-i18n";
import { get } from "svelte/store";
import type { Rename, VenueError } from "../../gen/rules/mtrack_rules";

/** What a fixture type's save says, whichever page saved it (the settings
 *  form, the type editor): "Saved.", the venue lines a rename retyped, or
 *  that the current venue no longer loads, which is not `ok`. */
export function typeSavedMessage(done: {
  rename?: Rename | null;
  venue_error?: VenueError | null;
}): { ok: boolean; text: string } {
  const tr = get(t);
  if (done.venue_error) {
    return {
      ok: false,
      text: tr("lighting.venueError.saved", {
        values: { ...done.venue_error },
      }),
    };
  }
  const said = [tr("common.saved")];
  if (done.rename && done.rename.lines > 0) {
    said.push(
      tr("lighting.settings.renamed", {
        values: {
          count: done.rename.lines,
          venues: done.rename.venues
            .map((v) => `${v.venue} (${v.lines})`)
            .join(", "),
        },
      }),
    );
  }
  return { ok: true, text: said.join(" ") };
}
