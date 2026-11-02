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

// The lighting timeline's tempo map, as plain values (no browser): the same
// positions the server's tempo map gives, ramps included. A ramp over N beats
// spans exactly N beats, linear in time.

import { test, expect } from "@playwright/test";
import {
  beatsToMs,
  getGridLines,
  getTempoAtMs,
  msToDurationString,
  msToTimestamp,
  offsetMeasuresToMs,
  rampDurationMs,
  snapToNearestGrid,
  timestampToMs,
} from "../../src/lib/lighting/timeline-state";
import type { TempoChange, TempoSection } from "../../src/lib/lighting/types";
import {
  absoluteTimestamp,
  measureBeatTimestamp,
} from "../../src/lib/lighting/types";

function tempo(
  bpm: number,
  sig: [number, number],
  changes: TempoChange[],
  startMs = 0,
): TempoSection {
  return {
    start: { value: startMs, unit: "ms" },
    bpm,
    time_signature: sig,
    changes,
  };
}

const at = (
  measure: number,
  beat: number,
  rest: Omit<TempoChange, "timestamp">,
): TempoChange => ({ timestamp: measureBeatTimestamp(measure, beat), ...rest });

const bar = (t: TempoSection, measure: number, beat = 1) =>
  timestampToMs(measureBeatTimestamp(measure, beat), t);

function close(got: number, want: number, what = "") {
  expect(
    Math.abs(got - want),
    `${what}: got ${got}, want ${want}`,
  ).toBeLessThan(1e-6);
}

test("a ramp over N beats spans exactly N beats", () => {
  for (const [from, to] of [
    [120, 180],
    [180, 120],
    [90, 140],
    [100, 100],
  ]) {
    for (const beats of [1, 4, 6.5, 8]) {
      const t = tempo(
        from,
        [4, 4],
        [at(3, 1, { bpm: to, transition: `${beats}` })],
        500,
      );
      const start = 500 + (8 * 60000) / from;
      const length = (beats * 60000) / ((from + to) / 2);
      close(rampDurationMs(from, to, beats), length);
      close(bar(t, 3), start, "ramp start");
      // Bar 3 beat 1 plus N beats is the ramp's end ...
      const whole = Math.floor(beats);
      const endBar = 3 + Math.floor(whole / 4);
      const endBeat = 1 + (beats - Math.floor(whole / 4) * 4);
      close(bar(t, endBar, endBeat), start + length, `${from}->${to}/${beats}`);
      close(beatsToMs(beats, start, t), length, "beats from the start");
      // ... the tempo is the target there, and the beats after are at it.
      close(getTempoAtMs(start + length + 1e-3, t), to);
      close(beatsToMs(4, start + length, t), (4 * 60000) / to);
      if (from !== to) {
        expect(
          Math.abs(getTempoAtMs(start + length - 1, t) - to),
        ).toBeGreaterThan(1e-3);
      }
    }
  }
});

test("the tempo moves linearly in time through a ramp", () => {
  const t = tempo(120, [4, 4], [at(3, 1, { bpm: 180, transition: "8" })]);
  // Bar 3 at 4s; 8 beats averaging 150 take 3.2s.
  close(getTempoAtMs(4000, t), 120);
  close(getTempoAtMs(5600, t), 150);
  close(getTempoAtMs(7200, t), 180);
});

test("a ramp over N measures ends on the barline N bars on", () => {
  for (const sig of [
    [4, 4],
    [3, 4],
    [6, 8],
    [7, 8],
  ] as [number, number][]) {
    const quarters = (sig[0] * 4) / sig[1];
    const t = tempo(120, sig, [at(3, 1, { bpm: 180, transition: "2m" })]);
    const bar3 = (2 * quarters * 60000) / 120;
    const bar5 = bar3 + rampDurationMs(120, 180, 2 * quarters);
    close(bar(t, 3), bar3, `${sig} bar 3`);
    close(bar(t, 5), bar5, `${sig} bar 5`);
    close(bar(t, 6), bar5 + (quarters * 60000) / 180, `${sig} bar 6`);
    close(offsetMeasuresToMs(2, bar3, t), bar5 - bar3, `${sig} 2 measures`);
    expect(msToDurationString(bar5 - bar3, t, bar3)).toBe("2measures");
  }
});

test("a change written after a ramp lands on its own bar", () => {
  const t = tempo(
    120,
    [4, 4],
    [at(3, 1, { bpm: 180, transition: "8" }), at(7, 1, { bpm: 90 })],
  );
  // Bar 5 at 7.2s, two bars at 180 put bar 7 at 9.8667s, then 90.
  const bar7 = 7200 + (8 * 60000) / 180;
  close(bar(t, 5), 7200);
  close(bar(t, 7), bar7);
  close(bar(t, 8), bar7 + (4 * 60000) / 90);
  expect(msToTimestamp(bar7, "measure_beat", t)).toEqual(
    measureBeatTimestamp(7, 1),
  );
  // The grid draws its barlines there too.
  const measures = getGridLines(t, 0, 12000)
    .filter((l) => l.type === "measure")
    .map((l) => l.ms);
  // Bar 4 is 4 beats into the ramp.
  const bar4 = 4000 + beatsToMs(4, 4000, t);
  const want = [0, 2000, 4000, bar4, 7200, 7200 + 4000 / 3, bar7];
  want.forEach((ms, i) => close(measures[i], ms, `barline ${i + 1}`));
});

test("a time-positioned change after a ramp counts through the ramp", () => {
  const t = tempo(
    120,
    [4, 4],
    [
      at(1, 1, { bpm: 60, transition: "4" }),
      { timestamp: absoluteTimestamp(10000), bpm: 100 },
      at(20, 1, { bpm: 80 }),
    ],
  );
  // As the server has it: 4 beats in 8/3s, 22/3 beats at 60 to 10s, then
  // 194/3 beats at 100 to bar 20.
  close(bar(t, 20), 48800);
});

test("a ramp in measures counts a signature set beside it, either order", () => {
  const ramp = at(5, 1, { bpm: 100, transition: "2m" });
  const metre = at(5, 1, { time_signature: [3, 4] });
  for (const changes of [
    [ramp, metre],
    [metre, ramp],
  ]) {
    const t = tempo(120, [4, 4], changes);
    // Bar 5 at 8s; 2 bars of 3/4 averaging 110 take 36/11s.
    close(bar(t, 7), 8000 + 36000 / 11);
  }
});

test("a mid-bar signature change governs from the next bar", () => {
  const t = tempo(120, [4, 4], [at(2, 3, { time_signature: [3, 4] })]);
  close(bar(t, 3), 4000);
  close(bar(t, 4), 5500);
});

test("snaps and steady tempos time as before", () => {
  const t = tempo(
    120,
    [4, 4],
    [
      at(3, 1, { bpm: 150 }),
      at(5, 1, { bpm: 100, time_signature: [3, 4] }),
      { timestamp: absoluteTimestamp(20000), bpm: 60 },
    ],
    1000,
  );
  close(bar(t, 1), 1000);
  close(bar(t, 3), 5000);
  close(bar(t, 5), 8200);
  close(bar(t, 7), 11800);
  // 20s is bar 11 beat 2 2/3; bar 12 is 1 1/3 beats on at 60.
  close(bar(t, 12), 20000 + 4000 / 3);
  const steady = tempo(120, [4, 4], [at(2, 1, { bpm: 120, transition: "2m" })]);
  for (let m = 1; m < 10; m++) close(bar(steady, m), (m - 1) * 2000);
});

test("snapping lands on the grid through a ramp", () => {
  const t = tempo(120, [4, 4], [at(3, 1, { bpm: 180, transition: "8" })]);
  for (let beat = 1; beat <= 4; beat++) {
    const ms = bar(t, 4, beat);
    close(snapToNearestGrid(ms + 20, t, "beat"), ms, `beat ${beat}`);
  }
  close(snapToNearestGrid(bar(t, 5) - 30, t, "measure"), bar(t, 5));
});
