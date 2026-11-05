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

// An effect's parameters are held as the line wrote them: `name: value`
// in order, repeats and spellings included (`EffectParams.params`). The
// forms read a parameter by name (the last of a name, as the player takes
// it) and set only the one they edit; what they did not touch is written
// back as it was.

import type { EffectParams, Remarks } from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

/** The value of the last `name` parameter, as written. */
export function param(effect: EffectParams, name: string): string | undefined {
  for (let i = effect.params.length - 1; i >= 0; i--) {
    if (effect.params[i].name === name) return effect.params[i].value;
  }
  return undefined;
}

/** A quoted value without its quotes, for a control that shows it. */
export function unquoted(value: string | undefined): string | undefined {
  return value !== undefined ? value.replace(/^"(.*)"$/, "$1") : undefined;
}

/** A sequence parameter, `$name`, substituted before the value is read:
 *  the grammar's own `param_ref`. */
export function isParam(value: string): boolean {
  return rules.param_ref(value);
}

/** A string value in quotes, unless it is a parameter, which must stay bare. */
export function quoted(value: string): string {
  return isParam(value) ? value.trim() : `"${value}"`;
}

/** Every `color:` value, unquoted, in order. */
export function colorsOf(effect: EffectParams): string[] {
  return effect.params
    .filter((p) => p.name === "color")
    .map((p) => unquoted(p.value)!);
}

/** The effect with `name` set to `value` (its last one replaced, or a new
 *  one added), or without it when `value` is empty or undefined. */
export function withParam(
  effect: EffectParams,
  name: string,
  value: string | undefined,
): EffectParams {
  const params = effect.params.slice();
  let last = -1;
  for (let i = params.length - 1; i >= 0; i--) {
    if (params[i].name === name) {
      last = i;
      break;
    }
  }
  if (value === undefined || value === "") return withoutParams(effect, [name]);
  // Its text alone changes: the comments around it stay.
  if (last >= 0) params[last] = { ...params[last], value, is_ref: undefined };
  else params.push({ name, value });
  return { ...effect, params };
}

/** The effect without any of `names`. */
export function withoutParams(
  effect: EffectParams,
  names: string[],
): EffectParams {
  return {
    ...effect,
    params: dropped(effect.params, (p) => names.includes(p.name)).kept,
  };
}

/** `items` without those `drop` picks, the comments written around each
 *  one dropped kept as comment lines where it was: above the next item
 *  kept, else after the one before. `orphans`: those with no item left to
 *  go to. */
export function dropped<T extends Remarks>(
  items: T[],
  drop: (item: T, index: number) => boolean,
): { kept: T[]; orphans: string[] } {
  const kept: T[] = [];
  let pending: string[] = [];
  items.forEach((item, i) => {
    if (drop(item, i)) {
      pending.push(
        ...(item.comments_before ?? []),
        ...(item.comment ? [item.comment] : []),
        ...(item.comments_after ?? []),
      );
    } else if (pending.length > 0) {
      kept.push({
        ...item,
        comments_before: [...pending, ...(item.comments_before ?? [])],
      });
      pending = [];
    } else kept.push(item);
  });
  if (pending.length > 0 && kept.length > 0) {
    const last = kept[kept.length - 1];
    kept[kept.length - 1] = {
      ...last,
      comments_after: [...(last.comments_after ?? []), ...pending],
    };
    pending = [];
  }
  return { kept, orphans: pending };
}

/** The effect with these `color:` values and no other, where the first
 *  colour was (or at the front). */
export function withColors(
  effect: EffectParams,
  colors: string[],
): EffectParams {
  const first = effect.params.findIndex((p) => p.name === "color");
  const rest = effect.params.filter((p) => p.name !== "color");
  const at = first < 0 ? 0 : first;
  // Each colour keeps the comments of the one it replaces.
  const was = effect.params.filter((p) => p.name === "color");
  const entries = colors.map((c, i) => ({
    ...was[i],
    name: "color",
    value: quoted(c),
    is_ref: undefined,
  }));
  // A colour no longer there leaves its comments where it was.
  const gone = was.slice(colors.length);
  const params = [...rest.slice(0, at), ...entries, ...gone, ...rest.slice(at)];
  const from = at + entries.length;
  return {
    ...effect,
    params: dropped(params, (_, i) => i >= from && i < from + gone.length).kept,
  };
}
