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

import { rules } from "../rules";

/** A fade length as written (`1measure`, `4beats`, `2s`) for reading, by
 *  the player's own reading of it (`rules.fade_length_text`): `1 measure`,
 *  `4 beats`; text that is no length shows as written. */
export function formatFadeLength(length: string): string {
  return rules.fade_length_text(length) ?? length.trim();
}
