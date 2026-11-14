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

// The colour reading and the edits are the player's (a wasm module); stand-ins
// keep this to the slot picker's own logic.
vi.mock("../rules", () => ({
  rules: {
    color_hex: (text: string): string | undefined => {
      const t = text.trim().replace(/^"|"$/g, "").toLowerCase();
      if (/^#[0-9a-f]{6}$/.test(t)) return t;
      return { orange: "#ffa500" }[t];
    },
    palette_edit: (
      _file: unknown,
      edit: { op: string; name?: string },
    ): unknown =>
      edit.op === "remove"
        ? { error: { code: "in_use", slot: edit.name, uses: [] } }
        : { document: { sequences: [], shows: [] } },
  },
}));

const {
  describeUse,
  editPalette,
  isColorText,
  mergeSlots,
  ownSlots,
  refOf,
  resolveColor,
  slotOfRef,
} = await import("./palette");

describe("a reference to a slot", () => {
  it("is read off the text and written back", () => {
    expect(slotOfRef("palette.primary")).toBe("primary");
    expect(slotOfRef(" palette.back-light ")).toBe("back-light");
    expect(refOf("primary")).toBe("palette.primary");
    expect(slotOfRef(refOf("x"))).toBe("x");
  });

  it("is not a color of its own, a bare prefix or another word", () => {
    expect(slotOfRef("#ff0000")).toBeNull();
    expect(slotOfRef("palette.")).toBeNull();
    expect(slotOfRef("palettes.x")).toBeNull();
    expect(slotOfRef("orange")).toBeNull();
  });
});

describe("the colors a form offers", () => {
  const slots = [
    { name: "primary", color: "#c01020" },
    { name: "accent", color: "#ffa500" },
  ];

  it("settle a reference to its slot's color, and a literal to itself", () => {
    expect(resolveColor("palette.accent", slots)).toBe("#ffa500");
    expect(resolveColor("orange", slots)).toBe("#ffa500");
    expect(resolveColor("#112233", slots)).toBe("#112233");
  });

  it("settle a reference to a slot not declared, and a non-color, to none", () => {
    expect(resolveColor("palette.ghost", slots)).toBeNull();
    expect(resolveColor("notacolor", slots)).toBeNull();
  });

  it("tell a sequence's color argument from another value", () => {
    expect(isColorText('"#c01020"')).toBe(true);
    expect(isColorText("palette.primary")).toBe(true);
    expect(isColorText("front")).toBe(false);
    expect(isColorText(undefined)).toBe(false);
  });
});

describe("a file's slots", () => {
  it("are read off its palette block, colors as #rrggbb", () => {
    const file = {
      sequences: [],
      shows: [],
      palette: {
        slots: [
          { name: "primary", value: '"#C01020"' },
          { name: "accent", value: '"orange"' },
        ],
      },
    };
    expect(ownSlots(file)).toEqual([
      { name: "primary", color: "#c01020" },
      { name: "accent", color: "#ffa500" },
    ]);
    expect(ownSlots({ sequences: [], shows: [] })).toEqual([]);
  });

  it("merge with the first declaration of a slot setting its color", () => {
    const merged = mergeSlots(
      [{ name: "a", color: "#111111" }],
      [
        { name: "a", color: "#222222" },
        { name: "b", color: "#333333" },
      ],
    );
    expect(merged).toEqual([
      { name: "a", color: "#111111" },
      { name: "b", color: "#333333" },
    ]);
  });
});

describe("a palette edit", () => {
  const file = { sequences: [], shows: [] };

  it("gives the file the player made, or the refusal", () => {
    expect(editPalette(file, { op: "add", name: "x", value: "red" })).toEqual({
      file: { sequences: [], shows: [] },
    });
    const refused = editPalette(file, { op: "remove", name: "x" });
    expect(refused.file).toBeUndefined();
    expect(refused.error?.code).toBe("in_use");
  });

  it("names a use in words", () => {
    const tr = (key: string, opts?: { values?: Record<string, string> }) =>
      `${key}:${JSON.stringify(opts?.values)}`;
    expect(
      describeUse(
        { container: "show", name: "Main", at: "@1/1", param: "color" },
        tr,
      ),
    ).toContain("palette.use.at");
    expect(
      describeUse({ container: "sequence", name: "hit", param: "color" }, tr),
    ).toContain("palette.use.param");
  });
});
