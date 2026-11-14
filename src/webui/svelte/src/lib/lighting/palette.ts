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

// A show's palette as the editors hold it: the slots a `.light` file declares
// (`palette { slot: "colour" }`) and the `palette.<slot>` references its
// effects make. The edits themselves are the player's (`rules.palette_edit`),
// so the UI never writes the grammar; what is here reads a document's palette
// for display, and settles a colour text to the `#rrggbb` a swatch paints.

import { getContext, setContext } from "svelte";
import type { LightFile } from "./types";
import type {
  PaletteEdit,
  PaletteEditError,
  PaletteUse,
  SequenceArg,
} from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

/** A palette slot with its colour as `#rrggbb`. */
export interface SlotColor {
  name: string;
  color: string;
}

/** What a reference to a slot is written as. */
export const REFERENCE_PREFIX = "palette.";

/** The slot a colour text refers to (`palette.accent` is `accent`), or null
 *  for a text that is a colour of its own. */
export function slotOfRef(text: string): string | null {
  const t = text.trim();
  if (!t.startsWith(REFERENCE_PREFIX) || t.length === REFERENCE_PREFIX.length) {
    return null;
  }
  return t.slice(REFERENCE_PREFIX.length);
}

/** The reference to `slot`, as an effect's `color:` writes it. */
export function refOf(slot: string): string {
  return `${REFERENCE_PREFIX}${slot}`;
}

/** The slots a file's own `palette { }` block declares, each colour as
 *  `#rrggbb` by the parser's reading of what is written. */
export function ownSlots(file: LightFile): SlotColor[] {
  return (file.palette?.slots ?? []).map((slot) => ({
    name: slot.name,
    color: rules.color_hex(slot.value) ?? "#000000",
  }));
}

/** Every slot a set of files declares, the first to declare one setting its
 *  colour (an override names a slot, not a file). */
export function mergeSlots(...lists: SlotColor[][]): SlotColor[] {
  const seen = new Map<string, SlotColor>();
  for (const list of lists) {
    for (const slot of list) {
      if (!seen.has(slot.name)) seen.set(slot.name, slot);
    }
  }
  return [...seen.values()];
}

/** A colour text as `#rrggbb`: the colour of its slot when it is a
 *  reference (null for a slot not declared), else the parser's reading of
 *  it (null when it is none). */
export function resolveColor(
  text: string,
  slots: readonly SlotColor[],
): string | null {
  const slot = slotOfRef(text);
  if (slot !== null) return slots.find((s) => s.name === slot)?.color ?? null;
  return rules.color_hex(text) ?? null;
}

/** The result of a palette edit: the file with it made, or why not. */
export type PaletteEdited =
  | { file: LightFile; error?: undefined }
  | { file?: undefined; error: PaletteEditError };

/** The file with a palette edit made (`rules.palette_edit`). */
export function editPalette(file: LightFile, edit: PaletteEdit): PaletteEdited {
  const result = rules.palette_edit(file, edit);
  if (result.document) return { file: result.document };
  return {
    error: result.error ?? { code: "unknown", slot: "", uses: [] },
  };
}

/** A use of a slot in words a person reads ("show Main at @1/1 (color)"). */
export function describeUse(
  use: PaletteUse,
  tr: (key: string, opts?: { values?: Record<string, string> }) => string,
): string {
  const where =
    use.container === "show"
      ? tr("palette.use.show", { values: { name: use.name || "default" } })
      : tr("palette.use.sequence", { values: { name: use.name } });
  return use.at
    ? tr("palette.use.at", {
        values: { where, at: use.at, param: use.param },
      })
    : tr("palette.use.param", { values: { where, param: use.param } });
}

// ---- The slots a form offers ----

/** The editor's slots, shared with the forms below it as a getter so a
 *  form reads the slots as they are when it draws. */
const PALETTE_CONTEXT = Symbol("palette-slots");

/** Offers `slots` to the forms below this component. */
export function providePalette(slots: () => SlotColor[]): void {
  setContext(PALETTE_CONTEXT, slots);
}

/** The slots the editor above offers; none outside one. */
export function usePalette(): () => SlotColor[] {
  return (
    getContext<(() => SlotColor[]) | undefined>(PALETTE_CONTEXT) ?? (() => [])
  );
}

/** The editor's sequences' declared parameters, by sequence name, for the
 *  forms that bind arguments. */
const SEQUENCE_PARAMS_CONTEXT = Symbol("sequence-params");

/** Offers the sequences' parameters to the forms below this component. */
export function provideSequenceParams(
  params: () => Record<string, SequenceArg[]>,
): void {
  setContext(SEQUENCE_PARAMS_CONTEXT, params);
}

/** The sequences' parameters the editor above offers; none outside one. */
export function useSequenceParams(): () => Record<string, SequenceArg[]> {
  return (
    getContext<(() => Record<string, SequenceArg[]>) | undefined>(
      SEQUENCE_PARAMS_CONTEXT,
    ) ?? (() => ({}))
  );
}

/** Whether a written value is a color: a palette reference, or a color the
 *  parser reads (quoted or not). */
export function isColorText(value: string | undefined): boolean {
  if (value === undefined) return false;
  return slotOfRef(value) !== null || rules.color_hex(value) !== undefined;
}
