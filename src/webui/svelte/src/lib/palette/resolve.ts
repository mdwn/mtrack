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

// The colours a song plays in, settled as the player settles them before a
// song starts: each slot is the show's default, unless the song's own
// `palette:` overrides it, unless the playlist entry's does. An override for
// a slot none of the song's shows declares changes nothing (the player warns
// and plays on), so it is reported apart. Plain values in and out: the
// playlist editor's strip and the song and entry pickers share it.

import type { SongPalette } from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

/** Where a slot's colour came from. */
export type ColorSource = "show" | "song" | "entry";

/** A slot and the colour it plays in. */
export interface ResolvedSlot {
  name: string;
  /** `#rrggbb`. */
  color: string;
  source: ColorSource;
}

/** One song's slots as they play, and the overrides that name nothing. */
export interface ResolvedPalette {
  /** In the order the song's shows declare them. */
  slots: ResolvedSlot[];
  /** Override slots (the song's, then the entry's) no show declares. */
  unknown: string[];
}

/** An override's colour text as `#rrggbb`, by the parser's own reading; null
 *  for text that is no colour (the player refuses those at load, so only an
 *  edit in progress holds one). */
export function overrideColor(text: string | undefined): string | null {
  if (text === undefined) return null;
  return rules.color_hex(text) ?? null;
}

/** The slots of a song as they play with `entry` laid over it. A song the
 *  player has no palette for (none declared, none overridden) has none. */
export function resolvePalette(
  palette: SongPalette | undefined,
  entry: Record<string, string> = {},
): ResolvedPalette {
  const declared = palette?.slots ?? [];
  const song = palette?.overrides ?? {};
  const names = new Set(declared.map((s) => s.name));
  const slots = declared.map((slot): ResolvedSlot => {
    const fromEntry = overrideColor(entry[slot.name]);
    if (fromEntry !== null) {
      return { name: slot.name, color: fromEntry, source: "entry" };
    }
    const fromSong = overrideColor(song[slot.name]);
    if (fromSong !== null) {
      return { name: slot.name, color: fromSong, source: "song" };
    }
    return { name: slot.name, color: slot.color, source: "show" };
  });
  const unknown = [...new Set([...Object.keys(song), ...Object.keys(entry)])]
    .filter((slot) => !names.has(slot))
    .sort();
  return { slots, unknown };
}

/** The slots of a song as they play before any entry: the show's colour
 *  or the song's override. What an entry's own picker starts from. */
export function songColors(palette: SongPalette | undefined): ResolvedSlot[] {
  return resolvePalette(palette).slots;
}

/** Entry `i`'s unknown overrides: those naming a slot the song does not
 *  declare, which the playlist shows with a warning. */
export function unknownOverrides(
  palette: SongPalette | undefined,
  entry: Record<string, string>,
): string[] {
  const names = new Set((palette?.slots ?? []).map((s) => s.name));
  return Object.keys(entry)
    .filter((slot) => !names.has(slot))
    .sort();
}

/** One playlist entry in the strip. */
export interface StripEntry {
  /** The entry's place in the playlist, from 1. */
  position: number;
  song: string;
  palette: ResolvedPalette;
}

/** Every entry of a playlist with the colours it plays in, in order. */
export function resolvePlaylist(
  songs: readonly string[],
  palettes: Record<string, SongPalette>,
  entries: readonly Record<string, string>[],
): StripEntry[] {
  return songs.map((song, i) => ({
    position: i + 1,
    song,
    palette: resolvePalette(palettes[song], entries[i] ?? {}),
  }));
}
