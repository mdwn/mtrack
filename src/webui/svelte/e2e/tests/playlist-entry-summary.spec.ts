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

// The playlist editor's entry helpers, as plain values (no browser).

import { test, expect } from "@playwright/test";
import {
  ambientForMode,
  ambientModeOf,
  entrySummary,
  withAmbientField,
} from "../../src/lib/playlistEntry";
import en from "../../src/lib/i18n/en.json" with { type: "json" };
import { loadRules } from "./rules-node";

// Lengths read by the player's own rule (`config::length`).
test.beforeAll(loadRules);

const strings = en as Record<string, string>;

/** English, with `{name}` filled in, as svelte-i18n would. */
function tr(key: string, opts?: { values?: Record<string, string> }): string {
  const text = strings[key];
  if (text === undefined) throw new Error(`missing string ${key}`);
  return text.replace(/\{(\w+)\}/g, (_, name) => opts?.values?.[name] ?? "");
}

const noFades = { fade_out: null, fade_in: null };

test.describe("entrySummary", () => {
  test("an entry that sets nothing sums to nothing", () => {
    expect(entrySummary({}, tr)).toBe("");
    expect(
      entrySummary({ fades: noFades, autoPlay: null, ambient: null }, tr),
    ).toBe("");
  });

  test("auto play at once, or with a zero gap, is no gap", () => {
    expect(entrySummary({ autoPlay: true }, tr)).toBe("");
    expect(entrySummary({ autoPlay: "0s" }, tr)).toBe("");
    expect(entrySummary({ autoPlay: "0ms" }, tr)).toBe("");
    expect(entrySummary({ autoPlay: "2s" }, tr)).toBe("2s gap");
    // A gap is read as the player reads one (`AutoPlayConfig::parse`):
    // any zero is none, and `2m` is two minutes.
    expect(entrySummary({ autoPlay: "0m" }, tr)).toBe("");
    expect(entrySummary({ autoPlay: "0h" }, tr)).toBe("");
    expect(entrySummary({ autoPlay: "500ms" }, tr)).toBe("0.5s gap");
    expect(entrySummary({ autoPlay: "2m" }, tr)).toBe("120s gap");
  });

  test("the ambient light: none, or its own", () => {
    expect(entrySummary({ ambient: false }, tr)).toBe("no ambient before");
    expect(entrySummary({ ambient: {} }, tr)).toBe("own ambient");
    expect(entrySummary({ ambient: { show: "a.light" } }, tr)).toBe(
      "own ambient",
    );
  });

  test("everything set joins in order", () => {
    expect(
      entrySummary(
        {
          fades: { fade_out: { length: "1measure" }, fade_in: "4beats" },
          autoPlay: "2s",
          ambient: false,
        },
        tr,
      ),
    ).toBe("fade out 1 measure · fade in 4 beats · 2s gap · no ambient before");
  });
});

test.describe("ambient picker helpers", () => {
  test("modes map to and from the settings", () => {
    expect(ambientModeOf(null)).toBe("inherit");
    expect(ambientModeOf(false)).toBe("off");
    expect(ambientModeOf({})).toBe("custom");
    expect(ambientModeOf({ show: "x" })).toBe("custom");
    expect(ambientForMode("inherit")).toBeNull();
    expect(ambientForMode("off")).toBe(false);
    expect(ambientForMode("custom")).toEqual({});
  });

  test("a field set is kept; emptied, it is left out to inherit", () => {
    const set = withAmbientField({ delay: "1s" }, "show", "a.light");
    expect(set).toEqual({ delay: "1s", show: "a.light" });
    expect(withAmbientField(set, "delay", "")).toEqual({ show: "a.light" });
    // From none or inherited, a field starts settings of its own.
    expect(withAmbientField(false, "fade_in", "2s")).toEqual({
      fade_in: "2s",
    });
    expect(withAmbientField(null, "fade_in", "2s")).toEqual({
      fade_in: "2s",
    });
  });
});
