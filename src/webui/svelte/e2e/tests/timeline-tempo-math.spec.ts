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
import { loadTempoMap } from "../../src/lib/tempo-wasm";
import {
  beatsToMs,
  computeAdjustedCuePositions,
  expandSequencesIntoCues,
  sequencePlay,
  showSequences,
  getAdjustedGridLines,
  offsetMarkers,
  offsetTimeline,
  snapDurationToGrid,
  offsetToMs,
  tempoLaneSpans,
  getGridLines,
  getTempoAtMs,
  msToDurationString,
  msToTimestamp,
  measuresToMs,
  rampDurationMs,
  snapToNearestGrid,
  timestampToMs,
} from "../../src/lib/lighting/timeline-state";
import type {
  Cue,
  CueEffect,
  Sequence,
  TempoChange,
  TempoSection,
} from "../../src/lib/lighting/types";
import {
  absoluteTimestamp,
  emptyCue,
  measureBeatTimestamp,
} from "../../src/lib/lighting/types";

// The tempo map runs as WebAssembly, loaded once before any of these.
test.beforeAll(loadTempoMap);

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
    close(measuresToMs(2, bar3, t), bar5 - bar3, `${sig} 2 measures`);
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

const cueAt = (
  measure: number,
  beat: number,
  extra: Partial<Cue> = {},
): Cue => ({
  ...emptyCue(measureBeatTimestamp(measure, beat)),
  ...extra,
});

test("an offset lasts its bars at the one tempo where it is issued", () => {
  // As the player sizes it: the change at bar 3 inside the offset's two
  // bars does not stretch them.
  const t = tempo(120, [4, 4], [at(3, 1, { bpm: 60 })]);
  close(offsetToMs(2, 2000, t), 4000);
  const cues = [cueAt(2, 1, { offset_measures: 2 }), cueAt(4, 1)];
  const [first, second] = computeAdjustedCuePositions(cues, t);
  close(first.offsetMs, 4000);
  close(second.rawMs, 8000);
  close(second.adjustedMs, 12000);
});

test("an offset is sized where its cue sits before any offset", () => {
  // The second offset is issued at bar 4 beat 3 (7s), at 120: shifted by
  // the first offset it would be at 11s, past the change to 60 at bar 5.
  const t = tempo(120, [4, 4], [at(5, 1, { bpm: 60 })]);
  const cues = [
    cueAt(4, 1, { offset_measures: 2 }),
    cueAt(4, 3, { offset_measures: 1 }),
    cueAt(6, 1),
  ];
  const positions = computeAdjustedCuePositions(cues, t);
  close(positions[0].offsetMs, 4000);
  close(positions[1].offsetMs, 2000);
  close(positions[2].adjustedMs, 12000 + 6000);
});

test("a reset drops the offset for the cues after it", () => {
  const t = tempo(120, [4, 4], []);
  const cues = [
    cueAt(1, 2, { offset_measures: 1 }),
    cueAt(2, 1, { reset_measures: true }),
    cueAt(3, 1),
  ];
  const positions = computeAdjustedCuePositions(cues, t);
  close(positions[1].adjustedMs, 2000 + 2000);
  close(positions[2].adjustedMs, 4000);
});

test("a signature change written past its bar's end governs from the next bar", () => {
  // Written @4/6 in 4/4: the server governs from bar 5, the bar the
  // position falls in, not the one after it.
  const t = tempo(120, [4, 4], [at(4, 6, { time_signature: [3, 4] })]);
  close(bar(t, 5), 8000);
  close(bar(t, 6), 9500);
});

test("a ramp in measures counts a signature set beside it by time or by bar", () => {
  const barredRamp = at(5, 1, { bpm: 100, transition: "2m" });
  const timedMetre: TempoChange = {
    timestamp: absoluteTimestamp(8000),
    time_signature: [3, 4],
  };
  const timedRamp: TempoChange = {
    timestamp: absoluteTimestamp(8000),
    bpm: 100,
    transition: "2m",
  };
  const barredMetre = at(5, 1, { time_signature: [3, 4] });
  for (const changes of [
    [barredRamp, timedMetre],
    [timedMetre, barredRamp],
    [timedRamp, barredMetre],
    [barredMetre, timedRamp],
  ]) {
    const t = tempo(120, [4, 4], changes);
    close(bar(t, 7), 8000 + 36000 / 11, JSON.stringify(changes));
  }
});

test("the tempo lane labels each stretch as it is", () => {
  const t = tempo(
    120,
    [4, 4],
    [
      at(3, 1, { bpm: 180, transition: "8" }),
      at(4, 1, { time_signature: [3, 4] }),
      at(6, 3, { time_signature: [2, 4] }),
    ],
  );
  const bar4 = bar(t, 4);
  const mid = Math.round(getTempoAtMs(bar4, t) * 100) / 100;
  const spans = tempoLaneSpans(t);
  expect(spans.map((s) => s.label)).toEqual([
    "120 BPM, 4/4",
    "120 → 180 BPM, 4/4",
    `${mid} → 180 BPM, 3/4`,
    "180 BPM, 3/4",
    "180 BPM, 2/4",
  ]);
  close(spans[1].startMs, 4000);
  close(spans[2].startMs, bar4);
  close(spans[3].startMs, 7200);
  // The mid-bar change shows from the next barline.
  close(spans[4].startMs, bar(t, 7));
});

test("an edit to the tempo in place is seen at once", () => {
  const t = tempo(120, [4, 4], [at(3, 1, { bpm: 60 })]);
  close(bar(t, 4), 8000);
  t.changes[0].bpm = 120;
  close(bar(t, 4), 6000);
  t.changes.push(at(4, 1, { bpm: 60 }));
  close(bar(t, 5), 6000 + 4000);
});

test("a long show with many tempo changes lays out quickly", () => {
  const changes: TempoChange[] = [];
  for (let m = 2; m <= 301; m++) {
    changes.push(
      at(m, 1, {
        bpm: 100 + (m % 7) * 10,
        transition: m % 3 === 0 ? "1m" : m % 3 === 1 ? "2" : undefined,
        time_signature: m % 50 === 0 ? [3, 4] : undefined,
      }),
    );
  }
  const t = tempo(120, [4, 4], changes);
  const cues: Cue[] = [];
  for (let m = 1; m <= 500; m++) {
    cues.push(cueAt(m, 1, m % 25 === 0 ? { offset_measures: 1 } : {}));
  }
  const started = performance.now();
  computeAdjustedCuePositions(cues, t);
  expandSequencesIntoCues(cues, [], t);
  getGridLines(t, 0, 600000);
  // Was ~13s; a generous bound well clear of a slow machine.
  expect(performance.now() - started).toBeLessThan(3000);
});

test("under an offset, a tempo change positioned in time moves with it", () => {
  // The values the server gives (show_edge_cases_tests.rs,
  // test_offset_shifts_time_positioned_tempo_changes).
  for (const [transition, want] of [
    [undefined, 11000],
    ["4", 10014.201009],
  ] as [string | undefined, number][]) {
    const t = tempo(
      120,
      [4, 4],
      [{ timestamp: absoluteTimestamp(6000), bpm: 60, transition }],
    );
    const cues = [cueAt(1, 1, { offset_measures: 1 }), cueAt(5, 1)];
    const positions = computeAdjustedCuePositions(cues, t);
    close(positions[1].adjustedMs, want, `${transition}`);
    // Its own place in the map, for its durations, is unshifted.
    close(positions[1].rawMs, transition ? bar(t, 5) : 10000);
  }
  // A change positioned by bar stays on its bar.
  const barred = tempo(120, [4, 4], [at(4, 1, { bpm: 60 })]);
  const positions = computeAdjustedCuePositions(
    [cueAt(1, 1, { offset_measures: 1 }), cueAt(5, 1)],
    barred,
  );
  close(positions[1].adjustedMs, 10000 + 2000);
});

test("placing a cue under an offset round-trips with its layout", () => {
  // Offsets, a time-positioned ramp the offset moves, and a bar-positioned
  // ramp that stays: a cue placed at the time it is drawn at comes back
  // as the same bar and beat, and is drawn there again.
  const t = tempo(
    120,
    [4, 4],
    [
      { timestamp: absoluteTimestamp(6000), bpm: 60, transition: "4" },
      at(9, 1, { bpm: 150, transition: "1m" }),
    ],
  );
  const offsetCues = [
    cueAt(1, 1, { offset_measures: 1 }),
    cueAt(3, 3, { offset_measures: 2 }),
  ];
  for (let m = 3; m <= 12; m++) {
    for (let b = 1; b <= 4; b++) {
      const cues = [...offsetCues, cueAt(m, b)];
      const placed = computeAdjustedCuePositions(cues, t)[2];
      const offset = placed.cumulativeOffsetMs;
      const score = placed.adjustedMs - offset;
      const what = `@${m}/${b}`;
      expect(msToTimestamp(score, "measure_beat", t, offset), what).toEqual(
        measureBeatTimestamp(m, b),
      );
      // Snapping a click near it lands back on it.
      close(snapToNearestGrid(score + 15, t, "beat", offset), score, what);
      close(timestampToMs(measureBeatTimestamp(m, b), t, offset), score, what);
    }
  }
  // The offset does move the time-positioned ramp: under it, bar 5 is
  // not where it is unshifted.
  const offset = computeAdjustedCuePositions(
    [...offsetCues, cueAt(5, 1)],
    t,
  )[2];
  expect(
    Math.abs(offset.adjustedMs - offset.cumulativeOffsetMs - offset.rawMs),
  ).toBeGreaterThan(1);
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
          beatsToMs(1, from, t) + 20,
          from,
          t,
          "beat",
        );
        expect(msToDurationString(beat, t, from), what).toBe("1beat");
        const measure = snapDurationToGrid(
          measuresToMs(1, from, t) + 20,
          from,
          t,
          "measure",
        );
        expect(msToDurationString(measure, t, from), what).toBe("1measure");
        const two = snapDurationToGrid(
          beatsToMs(2, from, t) - 20,
          from,
          t,
          "beat",
        );
        expect(msToDurationString(two, t, from), what).toBe("2beats");
      }
    }
  }
});

test("after a reset the grid and tempo lane follow the cues", () => {
  const t = tempo(120, [4, 4], []);
  const cues = [
    cueAt(3, 1, { offset_measures: 4 }),
    cueAt(6, 1, { reset_measures: true }),
    cueAt(10, 1),
  ];
  const positions = computeAdjustedCuePositions(cues, t);
  close(positions[1].adjustedMs, 10000 + 8000);
  close(positions[2].adjustedMs, 18000);
  const markers = offsetMarkers(cues, t, 0);
  const offsetAt = offsetTimeline(markers);
  // Every cue is drawn where the timeline puts its raw position.
  for (const p of positions) {
    close(p.rawMs + offsetAt(p.rawMs), p.adjustedMs, `cue ${p.index}`);
  }
  // The ruler's bar 10 is where the cue on it is drawn.
  const lines = getAdjustedGridLines(t, 0, 30000, markers);
  const bar10 = lines.find((l) => l.type === "measure" && l.label === "10");
  close(bar10!.ms, 18000);
  const bar6 = lines.filter((l) => l.type === "measure" && l.label === "6");
  close(bar6[0].ms, 18000);
});

test("a sequence is laid out in beats where it plays, as the player has it", () => {
  // The values the server gives (sequences_tests.rs,
  // a_sequence_counts_its_bars_at_one_tempo_whatever_the_songs_changes):
  // the song slows to 60 at bar 2; the sequence's own bar 3 is 8 beats
  // after its bar 1, so from the song's bar 1 it lands 2s + 4s in.
  const t = tempo(120, [4, 4], [at(2, 1, { bpm: 60 })]);
  const effect = (color: string): CueEffect => ({
    groups: ["front_wash"],
    effect: { type: "static", colors: [color], duration: "1beat" },
  });
  const seq: Sequence = {
    name: "s",
    cues: [
      { ...cueAt(1, 1), effects: [effect("red")] },
      { ...cueAt(3, 1), effects: [effect("blue")] },
    ],
  };
  const show = (loop?: string) => [
    { ...cueAt(1, 1), sequences: [{ name: "s", loop }] },
  ];
  const times = (cues: Cue[]) =>
    cues.filter((c) => c.effects.length).map((c) => timestampToMs(c.timestamp));
  const once = times(expandSequencesIntoCues(show(), [seq], t));
  expect(once.length).toBe(2);
  close(once[0], 0);
  close(once[1], 6000);
  // A pass is 9 beats (to the end of the last effect): the second starts
  // on beat 9 (7s) and its second cue is on beat 17 (15s).
  const twice = times(expandSequencesIntoCues(show("2"), [seq], t));
  expect(twice.length).toBe(4);
  close(twice[2], 7000);
  close(twice[3], 15000);
  // Two passes, 18 beats: 4 at 120 and 14 at 60.
  close(sequencePlay(show("2"), [seq], t, 0)!.totalMs, 2000 + 14000);
});

test("a sequence written in seconds keeps its seconds, as the player has it", () => {
  // The values the server gives (sequences_tests.rs,
  // a_sequence_in_seconds_keeps_its_seconds_across_a_tempo_change): from
  // bar 8 (14s) through the change to 60 at bar 9, half a second a cue.
  const t = tempo(120, [4, 4], [at(9, 1, { bpm: 60 })]);
  const effect: CueEffect = {
    groups: ["front_wash"],
    effect: { type: "static", colors: ["red"], duration: "500ms" },
  };
  const seq: Sequence = {
    name: "s",
    cues: [
      { ...emptyCue(absoluteTimestamp(0)), effects: [effect] },
      { ...emptyCue(absoluteTimestamp(500)), effects: [effect] },
    ],
  };
  const show = [{ ...cueAt(8, 1), sequences: [{ name: "s", loop: "4" }] }];
  const times = expandSequencesIntoCues(show, [seq], t)
    .filter((c) => c.effects.length)
    .map((c) => timestampToMs(c.timestamp));
  expect(times.length).toBe(8);
  times.forEach((ms, k) => close(ms, 14000 + 500 * k, `cue ${k}`));
  close(sequencePlay(show, [seq], t, 0)!.totalMs, 4000);
});

test("a sequence that cannot play is said so, and the rest drawn as written", () => {
  // Mid-edit: one call names a sequence that is not there. The player's
  // layout passes over it with why, and draws the others with their
  // colours.
  const t = tempo(120, [4, 4], []);
  const seq: Sequence = {
    name: "s",
    cues: [
      {
        ...cueAt(1, 1),
        effects: [
          {
            groups: ["front_wash"],
            effect: {
              type: "static",
              colors: ["#ff0000"],
              duration: "1beat",
              extra: {},
            },
          },
        ],
      },
    ],
  };
  const show = [
    { ...cueAt(1, 1), sequences: [{ name: "s" }] },
    { ...cueAt(3, 1), sequences: [{ name: "not_there" }] },
  ];
  const drawn = expandSequencesIntoCues(show, [seq], t).filter(
    (c) => c.effects.length,
  );
  expect(drawn.length).toBe(1);
  expect(drawn[0].effects[0].effect.colors).toEqual(["#ff0000"]);
  expect(drawn[0].effects[0].sequenceName).toBe("s");
  expect(sequencePlay(show, [seq], t, 0)!.error).toBeUndefined();
  expect(sequencePlay(show, [seq], t, 1)!.error).toContain("not_there");
});

test("while the file does not parse, the last layout that did is drawn", () => {
  const t = tempo(120, [4, 4], []);
  const effect = (type: string): CueEffect => ({
    groups: ["front_wash"],
    effect: { type, colors: ["red"], duration: "1beat" },
  });
  const seq: Sequence = {
    name: "s",
    cues: [{ ...cueAt(1, 1), effects: [effect("static")] }],
  };
  const show = (type: string) => [
    { ...cueAt(1, 1), sequences: [{ name: "s" }] },
    { ...cueAt(3, 1), effects: [effect(type)] },
  ];
  const lane = "parse-lane";
  const good = showSequences(show("static"), [seq], t, lane);
  expect(good.error).toBeUndefined();
  expect(good.calls.get(0)?.length).toBe(1);
  // Mid-edit: the effect's type is not one the grammar reads.
  const bad = showSequences(show("no such }"), [seq], t, lane);
  expect(bad.error).toBeTruthy();
  // Said without the written-out file's line and column.
  expect(bad.error).not.toMatch(/line \d+/i);
  expect(bad.calls.get(0)?.length).toBe(1);
  // And not on each cue.
  expect(sequencePlay(show("no such }"), [seq], t, 0, lane)!.error).toBe(
    undefined,
  );
});

test("a cue past a pass's length is counted on its call", () => {
  // At 240 a measure is a second: "s"'s cue 1.5s in falls past one pass
  // of "b" and does not play.
  const t = tempo(240, [4, 4], []);
  const red = (duration: string): CueEffect => ({
    groups: ["front_wash"],
    effect: { type: "static", colors: ["red"], duration },
  });
  const s: Sequence = {
    name: "s",
    cues: [
      { ...emptyCue(absoluteTimestamp(0)), effects: [red("200ms")] },
      { ...emptyCue(absoluteTimestamp(1500)), effects: [red("200ms")] },
    ],
  };
  const b: Sequence = {
    name: "b",
    length: "1measure",
    cues: [{ ...cueAt(1, 1), sequences: [{ name: "s" }] }],
  };
  const show = [{ ...cueAt(1, 1), sequences: [{ name: "b" }] }];
  const play = sequencePlay(show, [s, b], t, 0)!;
  expect(play.error).toBeUndefined();
  expect(play.dropped).toBe(1);
});

test("a file the player would not load says why, and still lays out", () => {
  const t = tempo(120, [4, 4], []);
  const effect = (colors: string[]): CueEffect => ({
    groups: ["front_wash"],
    effect: { type: "static", colors, duration: "1s" },
  });
  const a: Sequence = {
    name: "a",
    cues: [{ ...emptyCue(absoluteTimestamp(0)), effects: [effect(["red"])] }],
  };
  const broken: Sequence = {
    name: "broken",
    cues: [
      { ...emptyCue(absoluteTimestamp(0)), effects: [effect(["nocolor"])] },
    ],
  };
  const show = [
    { ...emptyCue(absoluteTimestamp(0)), sequences: [{ name: "a" }] },
  ];
  const layout = showSequences(show, [a, broken], t, "problem-lane");
  expect(layout.problem).toBeTruthy();
  expect(layout.calls.get(0)?.[0].cues.length).toBe(1);
});
