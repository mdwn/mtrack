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

// The lighting timeline's drawing helpers over the player's own tempo map
// (no browser): where a click snaps, what a dragged length is called, which
// bar and beat a new cue gets, and which lines the ruler has room for. The
// positions themselves are the player's (`rules.tempo_query`,
// `rules.tempo_distance`, `rules.layout`), held to it by the Rust tests in
// `crates/mtrack-rules-wasm/tests/lighting.rs`; what is checked here is
// the UI's rounding and labelling on top of them.

import { test, expect } from "@playwright/test";
import { loadRules } from "./rules-node";
import { rules } from "../../src/lib/rules";
import {
  durationStringToMs,
  getGridLines,
  msToDurationString,
  msToTimestamp,
  snapDurationToGrid,
  snapToNearestGrid,
  tempoSourceOf,
  timestampToMs,
} from "../../src/lib/lighting/timeline-state";
import type { TempoChange, TempoSection } from "../../src/lib/lighting/types";
import { measureBeatTimestamp } from "../../src/lib/lighting/types";
import type { TempoSource } from "../../src/gen/rules/mtrack_rules";

test.beforeAll(async () => {
  await loadRules();
});

function tempo(
  bpm: number,
  sig: [number, number],
  changes: TempoChange[],
  startMs = 0,
): TempoSource {
  const section: TempoSection = {
    start: { value: startMs, unit: "ms" },
    bpm,
    time_signature: sig,
    changes,
  };
  return tempoSourceOf(section, null)!;
}

const at = (
  measure: number,
  beat: number,
  rest: Omit<TempoChange, "timestamp">,
): TempoChange => ({ timestamp: measureBeatTimestamp(measure, beat), ...rest });

const bar = (t: TempoSource, measure: number, beat = 1) =>
  timestampToMs(measureBeatTimestamp(measure, beat), t);

function close(got: number, want: number, what = "") {
  expect(
    Math.abs(got - want),
    `${what}: got ${got}, want ${want}`,
  ).toBeLessThan(1e-6);
}

test("a tempo section is read through the parser as the block it writes", () => {
  const t = tempo(120, [4, 4], [at(3, 1, { bpm: 180, transition: "8" })], 500);
  close(bar(t, 3), 500 + 4000, "bar 3");
  // 8 beats averaging 150 take 3.2s.
  close(bar(t, 5), 500 + 4000 + 3200, "bar 5");
  close(durationStringToMs("2beats", t, 500), 1000, "two beats");
  close(durationStringToMs("1measure", t, 500), 2000, "a bar");
  close(durationStringToMs("1.5s", null, 0), 1500, "seconds without a tempo");
});

test("snapping lands on the grid through a ramp", () => {
  const t = tempo(120, [4, 4], [at(3, 1, { bpm: 180, transition: "8" })]);
  for (let beat = 1; beat <= 4; beat++) {
    const ms = bar(t, 4, beat);
    close(snapToNearestGrid(ms + 20, t, "beat"), ms, `beat ${beat}`);
    close(snapToNearestGrid(ms - 20, t, "1/2"), ms, `half beat ${beat}`);
  }
  close(snapToNearestGrid(bar(t, 5) - 30, t, "measure"), bar(t, 5));
  close(snapToNearestGrid(bar(t, 4) + 30, t, "measure"), bar(t, 4));
  // A click past the last beat of a bar snaps to the next bar's downbeat.
  close(snapToNearestGrid(bar(t, 5) - 10, t, "beat"), bar(t, 5));
});

test("a new cue gets the bar and beat of the nearest beat", () => {
  const t = tempo(
    120,
    [4, 4],
    [at(3, 1, { bpm: 180, transition: "8" }), at(7, 1, { bpm: 90 })],
  );
  const bar7 = 7200 + (8 * 60000) / 180;
  expect(msToTimestamp(bar7, t)).toEqual(measureBeatTimestamp(7, 1));
  expect(msToTimestamp(bar7 + 100, t)).toEqual(measureBeatTimestamp(7, 1));
  expect(msToTimestamp(bar(t, 4, 4) + 20, t)).toEqual(
    measureBeatTimestamp(4, 4),
  );
  expect(msToTimestamp(bar(t, 5) - 20, t)).toEqual(measureBeatTimestamp(5, 1));
  expect(msToTimestamp(1234, null)).toEqual({ type: "absolute", ms: 1234 });
});

test("a snapped beat or bar reads back as one, wherever it starts", () => {
  for (const bpm of [97, 120, 133.7]) {
    const t = tempo(
      bpm,
      [4, 4],
      [at(9, 1, { bpm: bpm + 40, transition: "2m" })],
    );
    for (let m = 1; m <= 40; m++) {
      for (let b = 1; b <= 4; b++) {
        const from = bar(t, m, b);
        const what = `${bpm} @${m}/${b}`;
        // A drag a little past each length, snapped back to it.
        const beat = snapDurationToGrid(
          durationStringToMs("1beat", t, from) + 20,
          from,
          t,
          "beat",
        );
        expect(msToDurationString(beat, t, from), what).toBe("1beat");
        const measure = snapDurationToGrid(
          durationStringToMs("1measure", t, from) + 20,
          from,
          t,
          "measure",
        );
        expect(msToDurationString(measure, t, from), what).toBe("1measure");
        const two = snapDurationToGrid(
          durationStringToMs("2beats", t, from) - 20,
          from,
          t,
          "beat",
        );
        expect(msToDurationString(two, t, from), what).toBe("2beats");
      }
    }
  }
});

test("a length is named in bars, beats or time", () => {
  for (const sig of [
    [4, 4],
    [3, 4],
    [6, 8],
    [7, 8],
  ] as [number, number][]) {
    const t = tempo(120, sig, [at(3, 1, { bpm: 180, transition: "2m" })]);
    const bar3 = bar(t, 3);
    expect(msToDurationString(bar(t, 5) - bar3, t, bar3)).toBe("2measures");
    expect(msToDurationString(bar(t, 3, 2) - bar3, t, bar3)).toBe("1beat");
  }
  const t = tempo(120, [4, 4], []);
  expect(msToDurationString(750, t, 0)).toBe("750ms");
  expect(msToDurationString(5000, null, 0)).toBe("5s");
  expect(msToDurationString(250, null, 0)).toBe("250ms");
});

test("a loop's resize counts the passes the drag reaches, under a ramp", () => {
  // 60 to 240 BPM over three bars: six one-bar passes get shorter and
  // shorter, so `(end - start) / first pass` is nowhere near six.
  const source =
    "tempo {\n    bpm: 60\n    time_signature: 4/4\n    changes: [\n        @1/1 { bpm: 240, transition: 3m }\n    ]\n}\n" +
    'sequence "s", length: 1measure {\n    @1/1\n    a: static, color: "red", duration: 1beat\n}\n' +
    'show "S" {\n    @1/1\n    sequence "s", loop: 6\n}\n';
  const laid = rules.layout({ source, includes: [], until_ms: 60000 });
  const pass = laid.shows[0].cues[0].passes[0];
  expect(pass.count).toBe(6);
  expect(pass.pass_starts).toHaveLength(6);
  // The passes laid on past the six, to the end of the timeline, each
  // ending where the next starts.
  expect(pass.pass_ends.length).toBeGreaterThan(8);
  expect(pass.pass_ends.slice(0, 5)).toEqual(pass.pass_starts.slice(1));
  const by = (ms: number) => rules.passes_by(pass.pass_ends, ms)!;
  // The block at its own width is all six passes, not the three the first
  // pass's length would divide it into.
  expect(by(pass.end_ms)).toBe(6);
  expect(Math.round((pass.end_ms - pass.start_ms) / pass.pass_ms)).toBeLessThan(
    6,
  );
  // A nudge past the end is still six; a wider drag grows the loop to
  // eight; a drag back to the fourth start holds three whole passes.
  expect(by(pass.end_ms + 1)).toBe(6);
  expect(by(pass.pass_ends[7])).toBe(8);
  expect(by(pass.pass_ends[8] - 1)).toBe(8);
  expect(by(pass.pass_starts[3])).toBe(3);
  expect(by(pass.pass_starts[3] - 1)).toBe(2);
  expect(by(pass.start_ms - 1)).toBe(1);
  // The count at every width is the passes whose end that width reaches,
  // so the ghost (drawn to the last one's end) never shows more than fit.
  for (let ms = pass.start_ms; ms < pass.pass_ends[9]; ms += 37) {
    const n = by(ms);
    expect(n === 1 || pass.pass_ends[n - 1] <= ms).toBe(true);
    expect(pass.pass_ends[n] > ms).toBe(true);
  }
});

test("a tempo start switched to another unit lays out where it was", () => {
  const laidStart = (start: string) =>
    rules.layout({
      source: `tempo {\n    start: ${start}\n    bpm: 120\n}\nshow "S" {\n    @1/1\n    a: static, color: "red", duration: 1s\n}\n`,
      includes: [],
      until_ms: 10000,
    }).tempo!.start_ms;
  for (const [value, from, to] of [
    [500, "ms", "s"],
    [1.5, "s", "ms"],
    [1.001, "s", "ms"],
    [0.1, "ms", "s"],
  ] as const) {
    const converted = rules.time_in(value, from, to)!;
    expect(laidStart(`${converted}${to}`), `${value}${from}`).toBe(
      laidStart(`${value}${from}`),
    );
    expect(rules.time_in(converted, to, from)).toBe(value);
  }
  expect(rules.time_in(500, "ms", "s")).toBe(0.5);
  expect(rules.time_in(1.5, "s", "ms")).toBe(1500);
});

test("the ruler labels and beat lines thin out as the bars narrow", () => {
  const source = tempo(120, [4, 4], []);
  const laid = rules.layout({
    source: `${(source as { dsl: string }).dsl}show "S" {\n    @1/1\n    a: static, color: "red", duration: 1s\n}\n`,
    includes: [],
    until_ms: 60000,
  });
  const grid = laid.tempo!.grid;
  // Bars of 2 s: at 0.05 px/ms a bar is 100 px, every bar labelled (bar 11
  // on the range's end), beats 25 px apart.
  const wide = getGridLines(grid, 0, 20000, 0.05);
  expect(wide.filter((l) => l.type === "measure" && l.label).length).toBe(11);
  expect(wide.filter((l) => l.type === "beat").length).toBe(30);
  // At 0.01 px/ms a bar is 20 px: a label every 8 bars, no beat lines.
  const narrow = getGridLines(grid, 0, 60000, 0.01);
  expect(narrow.filter((l) => l.type === "beat").length).toBe(0);
  expect(
    narrow.filter((l) => l.type === "measure" && l.label).map((l) => l.label),
  ).toEqual(["1", "9", "17", "25"]);
  // Only the visible range.
  const window = getGridLines(grid, 4000, 6000, 0.05);
  expect(window.map((l) => l.ms)).toEqual([4000, 4500, 5000, 5500, 6000]);
});
