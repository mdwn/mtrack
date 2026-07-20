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

import type { SubdivisionValue } from "./api/songs";

const NOTE_GLYPHS: Record<number, string> = {
  1: "𝅝",
  2: "𝅗𝅥",
  4: "♩",
  8: "♪",
  16: "𝅘𝅥𝅯",
  32: "𝅘𝅥𝅰",
};

/** The note glyph for a 1/den note value. */
export function noteGlyph(den: number): string {
  return NOTE_GLYPHS[den] ?? `1/${den}`;
}

/** Display parts for a subdivision option, relative to the meter's beat
 * note (the time-signature denominator): 4/4 subdivides quarters into
 * eighths/triplets/sixteenths, 3/2 halves into quarters, 7/8 eighths into
 * sixteenths, and so on. */
export function subdivisionParts(
  sub: SubdivisionValue,
  beatDen: number,
): { glyph: string; nameKey: string } {
  if (sub === "son") return { glyph: "3–2", nameKey: "metronome.subdiv.son" };
  if (sub === "rumba")
    return { glyph: "3–2", nameKey: "metronome.subdiv.rumba" };
  switch (sub) {
    case 2:
      return {
        glyph: noteGlyph(beatDen * 2),
        nameKey: `meter.notes.${beatDen * 2}`,
      };
    case 3:
      return {
        glyph: noteGlyph(beatDen * 2) + "³",
        nameKey: "metronome.subdiv.triplets",
      };
    case 4:
      return {
        glyph: noteGlyph(beatDen * 4),
        nameKey: `meter.notes.${beatDen * 4}`,
      };
    case 6:
      return {
        glyph: noteGlyph(beatDen * 4) + "³",
        nameKey: "metronome.subdiv.sextuplets",
      };
    default:
      return { glyph: noteGlyph(beatDen), nameKey: `meter.note.${beatDen}` };
  }
}

/** Compact chip text for a subdivision on a timeline marker. */
export function subdivisionChip(
  sub: SubdivisionValue,
  beatDen: number,
): string {
  if (typeof sub === "string") return sub;
  return subdivisionParts(sub, beatDen).glyph;
}

/** Compact per-beat accent pattern for marker chips:
 * 0 → ·, 1 → ▁, 2 → ▄, 3 → █. */
export function accentsGlyph(levels: number[]): string {
  const chars = ["·", "▁", "▄", "█"];
  return levels.map((level) => chars[Math.min(Math.max(level, 0), 3)]).join("");
}
