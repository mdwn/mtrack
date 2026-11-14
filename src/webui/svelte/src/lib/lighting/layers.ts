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

// The player's lighting layers, bottom to top (`dmx.lighting.layers`): the
// names a cue's `layer:` takes, the lanes the timeline draws, and what the
// layout is parsed against. The player reports them with the lighting
// groups (`GET /api/lighting/groups`); until that answers, the parser's
// default three stand in.

import { derived, get, writable } from "svelte/store";
import { rules } from "../rules";

/** The layers the player last reported; `null` before it has. */
export const playerLayers = writable<string[] | null>(null);

/** The layers to use now, bottom to top: the player's, else the
 *  parser's defaults. */
export const layerNames = derived(
  playerLayers,
  ($layers) => $layers ?? rules.vocabulary().layers,
);

/** The parser's words with the player's layers (`rules.vocabulary`). */
export function vocabulary() {
  return rules.vocabulary(get(layerNames));
}

/** Records the layers the player reported; an empty report (an older
 *  player, which sends none) leaves the defaults. */
export function setPlayerLayers(layers: string[] | undefined) {
  playerLayers.set(layers && layers.length > 0 ? layers : null);
}

/** The most layers a player has. */
export const MAX_LAYERS = 16;

/** Whether `name` is a layer name the parser reads: a letter or `_`,
 *  then letters, digits, `_` or `-`. */
export function isLayerName(name: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_-]*$/.test(name);
}

/** Why `names` are not a set of layers, or null when they are: between
 *  one and sixteen unique identifiers. The player checks them again. */
export function layersProblem(names: string[]): string | null {
  if (names.length === 0) return "empty";
  if (names.length > MAX_LAYERS) return "tooMany";
  if (names.some((n) => !isLayerName(n))) return "notIdentifier";
  if (new Set(names).size !== names.length) return "duplicate";
  return null;
}
