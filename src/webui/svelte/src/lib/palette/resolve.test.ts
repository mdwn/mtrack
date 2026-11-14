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
import { describe, expect, it, vi } from "vitest";
import type { SongPalette } from "../../gen/rules/mtrack_rules";

// The colour reading is the player's (a wasm module); a stand-in that reads
// `#rrggbb` and two names keeps this to the resolution order.
vi.mock("../rules", () => ({
  rules: {
    color_hex: (text: string): string | undefined => {
      const t = text.trim().replace(/^"|"$/g, "").toLowerCase();
      if (/^#[0-9a-f]{6}$/.test(t)) return t;
      return { cyan: "#00ffff", red: "#ff0000" }[t];
    },
  },
}));

const {
  overrideColor,
  resolvePalette,
  resolvePlaylist,
  songColors,
  unknownOverrides,
} = await import("./resolve");

const RED_SONG: SongPalette = {
  slots: [
    { name: "primary", color: "#c01020" },
    { name: "secondary", color: "#ff8000" },
  ],
  overrides: {},
};

describe("a song's colors as they play", () => {
  it("are the show's defaults with nothing overridden", () => {
    const { slots, unknown } = resolvePalette(RED_SONG);
    expect(slots).toEqual([
      { name: "primary", color: "#c01020", source: "show" },
      { name: "secondary", color: "#ff8000", source: "show" },
    ]);
    expect(unknown).toEqual([]);
  });

  it("take the song's override over the show's default", () => {
    const palette = { ...RED_SONG, overrides: { primary: "#2040ff" } };
    const { slots } = resolvePalette(palette);
    expect(slots[0]).toEqual({
      name: "primary",
      color: "#2040ff",
      source: "song",
    });
    expect(slots[1].source).toBe("show");
  });

  it("take the entry's override over the song's, and read names", () => {
    const palette = { ...RED_SONG, overrides: { primary: "#2040ff" } };
    const { slots } = resolvePalette(palette, {
      primary: "cyan",
      secondary: "#00ff00",
    });
    expect(slots).toEqual([
      { name: "primary", color: "#00ffff", source: "entry" },
      { name: "secondary", color: "#00ff00", source: "entry" },
    ]);
  });

  it("keep the default for an override that is no color", () => {
    const { slots } = resolvePalette(RED_SONG, { primary: "notacolor" });
    expect(slots[0]).toEqual({
      name: "primary",
      color: "#c01020",
      source: "show",
    });
  });

  it("leave out an override for a slot no show declares, and say so", () => {
    const palette = { ...RED_SONG, overrides: { ghost: "#111111" } };
    const { slots, unknown } = resolvePalette(palette, {
      phantom: "#222222",
      ghost: "#333333",
      primary: "#444444",
    });
    expect(slots.map((s) => s.name)).toEqual(["primary", "secondary"]);
    expect(unknown).toEqual(["ghost", "phantom"]);
  });

  it("are none for a song with no palette", () => {
    expect(resolvePalette(undefined, { primary: "#444444" })).toEqual({
      slots: [],
      unknown: ["primary"],
    });
  });
});

describe("a song's own colors, before any entry", () => {
  it("are what an entry's picker starts from", () => {
    const palette = { ...RED_SONG, overrides: { secondary: "cyan" } };
    expect(songColors(palette).map((s) => s.color)).toEqual([
      "#c01020",
      "#00ffff",
    ]);
  });
});

describe("a playlist's strip", () => {
  const palettes: Record<string, SongPalette> = {
    Red: RED_SONG,
    Blue: {
      slots: [{ name: "primary", color: "#0000ff" }],
      overrides: {},
    },
  };

  it("has an entry per song in order, each with its own entry's colors", () => {
    const strip = resolvePlaylist(["Red", "Blue", "Red", "Plain"], palettes, [
      {},
      {},
      { primary: "#2040ff" },
    ]);
    expect(strip.map((e) => [e.position, e.song])).toEqual([
      [1, "Red"],
      [2, "Blue"],
      [3, "Red"],
      [4, "Plain"],
    ]);
    // The same song twice plays in different colors by entry.
    expect(strip[0].palette.slots[0].color).toBe("#c01020");
    expect(strip[2].palette.slots[0].color).toBe("#2040ff");
    expect(strip[2].palette.slots[0].source).toBe("entry");
    // A song the player has no palette for has no colors, and no warning.
    expect(strip[3].palette).toEqual({ slots: [], unknown: [] });
  });

  it("reads a missing entry list as no overrides", () => {
    const strip = resolvePlaylist(["Red"], palettes, []);
    expect(strip[0].palette.slots[0].source).toBe("show");
  });
});

describe("an override's color", () => {
  it("is the parser's reading, or none", () => {
    expect(overrideColor("#ABCDEF")).toBe("#abcdef");
    expect(overrideColor("red")).toBe("#ff0000");
    expect(overrideColor("nope")).toBeNull();
    expect(overrideColor(undefined)).toBeNull();
  });
});

describe("an entry's overrides naming nothing", () => {
  it("are the slots its song does not declare, sorted", () => {
    expect(
      unknownOverrides(RED_SONG, {
        zed: "#111111",
        primary: "#222222",
        a: "x",
      }),
    ).toEqual(["a", "zed"]);
    expect(unknownOverrides(undefined, { a: "x" })).toEqual(["a"]);
    expect(unknownOverrides(RED_SONG, {})).toEqual([]);
  });
});
