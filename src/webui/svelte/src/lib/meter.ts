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

/** Clave hits as half-beat offsets over a two-measure 4/4 cycle (16). */
export const CLAVE_PATTERNS: Record<"son" | "rumba", number[]> = {
  son: [0, 3, 6, 10, 12],
  rumba: [0, 3, 7, 10, 12],
};

/** How to draw a subdivision option: a (beamed) note group or a clave
 * hit pattern, plus the i18n key of its caption. */
export type SubdivisionIcon =
  | {
      kind: "notes";
      den: number;
      count: number;
      tuplet?: number;
      nameKey: string;
    }
  | { kind: "clave"; pattern: number[]; nameKey: string };

/** Icon and caption for a subdivision option, relative to the meter's beat
 * note (the time-signature denominator): 4/4 subdivides quarters into
 * eighths/triplets/sixteenths, 3/2 halves into quarters, 7/8 eighths into
 * sixteenths, and so on. */
export function subdivisionIcon(
  sub: SubdivisionValue,
  beatDen: number,
): SubdivisionIcon {
  if (sub === "son" || sub === "rumba") {
    return {
      kind: "clave",
      pattern: CLAVE_PATTERNS[sub],
      nameKey: `metronome.subdiv.${sub}`,
    };
  }
  switch (sub) {
    case 2:
      return {
        kind: "notes",
        den: beatDen * 2,
        count: 2,
        nameKey: `meter.notes.${beatDen * 2}`,
      };
    case 3:
      return {
        kind: "notes",
        den: beatDen * 2,
        count: 3,
        tuplet: 3,
        nameKey: "metronome.subdiv.triplets",
      };
    case 4:
      return {
        kind: "notes",
        den: beatDen * 4,
        count: 4,
        nameKey: `meter.notes.${beatDen * 4}`,
      };
    case 6:
      return {
        kind: "notes",
        den: beatDen * 4,
        count: 6,
        tuplet: 6,
        nameKey: "metronome.subdiv.sextuplets",
      };
    default:
      return {
        kind: "notes",
        den: beatDen,
        count: 1,
        nameKey: `meter.note.${beatDen}`,
      };
  }
}

/** Compact chip text for a subdivision on a timeline marker, e.g. "1/8",
 * "1/8t" (triplet feel), "son". */
export function subdivisionChip(
  sub: SubdivisionValue,
  beatDen: number,
): string {
  if (typeof sub === "string") return sub;
  switch (sub) {
    case 2:
      return `1/${beatDen * 2}`;
    case 3:
      return `1/${beatDen * 2}t`;
    case 4:
      return `1/${beatDen * 4}`;
    case 6:
      return `1/${beatDen * 4}t`;
    default:
      return `1/${beatDen}`;
  }
}

/** Compact per-beat accent pattern for marker chips:
 * 0 → ·, 1 → ▁, 2 → ▄, 3 → █. */
export function accentsGlyph(levels: number[]): string {
  const chars = ["·", "▁", "▄", "█"];
  return levels.map((level) => chars[Math.min(Math.max(level, 0), 3)]).join("");
}
