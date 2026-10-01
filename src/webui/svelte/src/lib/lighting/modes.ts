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

/**
 * What a GDTF mode lets a show do, in the words the fixture's page shows,
 * worked out from the capability names the archive's inspection reports.
 */

import type { GdtfMode } from "../api/config";

/** A thing a show can do with a fixture, keyed to its i18n words. */
export interface Ability {
  /** The suffix of the `lighting.gdtf.can.*` message. */
  key:
    | "color"
    | "dimmer"
    | "dimmerThroughColor"
    | "strobe"
    | "strobeUpTo"
    | "strobeAny"
    | "move"
    | "cells"
    | "wheel";
  params?: Record<string, string | number>;
}

/** A mode the distiller could not turn into a fixture type. */
export function isRefused(mode: GdtfMode): boolean {
  return mode.refused !== undefined;
}

/** A frequency without trailing noise: `25`, `0.4`. */
function hz(value: number): string {
  return String(Number(value.toFixed(2)));
}

/** What a show can do in this mode, in the order the panel lists it. */
export function abilities(mode: GdtfMode): Ability[] {
  const caps = new Set(mode.capabilities ?? []);
  const out: Ability[] = [];
  if (caps.has("color")) out.push({ key: "color" });
  if (caps.has("dimmer")) out.push({ key: "dimmer" });
  // No dimmer channel, but the colour can be dimmed by mixing less of it.
  else if (caps.has("color")) out.push({ key: "dimmerThroughColor" });
  if (caps.has("strobe")) {
    const range = mode.strobe_range;
    if (range && range.min_hz != null) {
      out.push({
        key: "strobe",
        params: { min: hz(range.min_hz), max: hz(range.max_hz) },
      });
    } else if (range) {
      out.push({ key: "strobeUpTo", params: { max: hz(range.max_hz) } });
    } else {
      out.push({ key: "strobeAny" });
    }
  }
  if (caps.has("pan_tilt")) out.push({ key: "move" });
  if ((mode.cells ?? 0) > 0)
    out.push({ key: "cells", params: { count: mode.cells ?? 0 } });
  if (caps.has("color_wheel") && !caps.has("color")) out.push({ key: "wheel" });
  return out;
}

/** Channels a static can set but no effect drives, named for the panel. */
export function extras(mode: GdtfMode): string[] {
  const caps = new Set(mode.capabilities ?? []);
  return ["white", "zoom", "focus", "gobo"].filter((c) => caps.has(c));
}

/** What a mode lacks that another mode of the archive has. */
export interface Missing {
  /** The suffix of the `lighting.gdtf.missing.*` message. */
  what: "color" | "dimmer" | "strobe" | "move" | "cells";
  /** A mode that has it. */
  mode: string;
}

/** The abilities this mode lacks that some other offered mode has, each with
 *  the first such mode. Dimming is not lacking while colour can dim it. */
export function missingHere(mode: GdtfMode, all: GdtfMode[]): Missing[] {
  const has = (m: GdtfMode, what: Missing["what"]): boolean => {
    const caps = new Set(m.capabilities ?? []);
    switch (what) {
      case "color":
        return caps.has("color");
      case "dimmer":
        return caps.has("dimmer") || caps.has("color");
      case "strobe":
        return caps.has("strobe");
      case "move":
        return caps.has("pan_tilt");
      case "cells":
        return (m.cells ?? 0) > 0;
    }
  };
  const out: Missing[] = [];
  for (const what of ["color", "dimmer", "strobe", "move", "cells"] as const) {
    if (has(mode, what)) continue;
    const other = all.find(
      (m) => m.name !== mode.name && !isRefused(m) && has(m, what),
    );
    if (other) out.push({ what, mode: other.name });
  }
  return out;
}

/** Modes whose name (or address count) contains the filter, any case. */
export function filterModes(modes: GdtfMode[], query: string): GdtfMode[] {
  const q = query.trim().toLowerCase();
  if (!q) return modes;
  return modes.filter(
    (m) =>
      m.name.toLowerCase().includes(q) ||
      `${m.footprint} addresses`.includes(q),
  );
}

/** The file stem the server writes a type under: lower-case ASCII letters
 *  and digits, runs of anything else as one underscore (the server's
 *  `fixture_filename_stem`). */
export function fileStem(name: string): string {
  let out = "";
  for (const c of name) {
    if (/[A-Za-z0-9]/.test(c)) out += c.toLowerCase();
    else if (!out.endsWith("_") && out.length > 0) out += "_";
  }
  out = out.replace(/_+$/, "");
  return out || "fixture";
}
