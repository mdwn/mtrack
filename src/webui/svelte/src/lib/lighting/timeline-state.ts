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

import type {
  Timestamp,
  TempoSection,
  TempoChange,
  Cue,
  Sequence,
  CueEffect,
} from "./types";
import { absoluteTimestamp } from "./types";

/** Where the tempo stands from one tempo change to the next.
 *
 *  Positions are counted as the server's tempo map counts them: in quarter
 *  notes from bar 1 beat 1, so `@bar/beat` beats are quarter notes in any
 *  metre (a bar of 6/8 holds three). A ramp moves the tempo linearly in time
 *  and spans exactly the beats it is written over, as a DAW renders it. */
export interface TempoSegment {
  /** Absolute ms where this segment starts */
  startMs: number;
  /** Measure number at the start of this segment (1-based) */
  startMeasure: number;
  /** Quarter-note beat within the start measure (1-based) */
  startBeat: number;
  /** Quarter notes from bar 1 beat 1 to the start of this segment */
  startBeats: number;
  /** The tempo from here, once any ramp in progress has reached it */
  bpm: number;
  /** A ramp still in progress at the start of this segment */
  ramp?: TempoRamp;
  /** The time signature's numerator, as written */
  beatsPerMeasure: number;
  /** The time signature's denominator, as written */
  beatValue: number;
}

/** A gradual tempo change: linear in time from `fromBpm` to `toBpm`. */
export interface TempoRamp {
  fromBpm: number;
  toBpm: number;
  /** How long the whole ramp lasts */
  totalMs: number;
  /** How far into the ramp the segment it belongs to starts */
  elapsedMs: number;
}

/** A time signature, governing from the downbeat of `measure` on. */
interface MeterSpan {
  measure: number;
  /** Quarter notes from bar 1 beat 1 to that downbeat */
  startBeats: number;
  /** Quarter notes in each bar */
  quarters: number;
  /** The signature as written */
  signature: [number, number];
}

/** The tempo map the timeline converts through. */
interface TempoGrid {
  segments: TempoSegment[];
  meters: MeterSpan[];
  /** The change that starts each segment after the first (`changes[i]`
   *  starts `segments[i + 1]`), for walks that place them differently. */
  changes: ResolvedChange[];
  /** Whether any change is positioned in time rather than by bar */
  anyTimed: boolean;
}

/** A change as the tempo walk applies it. */
interface ResolvedChange {
  timed: boolean;
  bpm?: number;
  /** A ramp's length in quarter notes, as sized; none for a snap */
  rampBeats?: number;
}

/** Grid line for rendering on the ruler. */
export interface GridLine {
  ms: number;
  type: "measure" | "beat";
  label?: string;
}

const MS_PER_MINUTE = 60000;

function quartersPerMeasure([numerator, denominator]: [
  number,
  number,
]): number {
  return denominator > 0 ? (numerator * 4) / denominator : numerator;
}

/** How long a ramp from `fromBpm` to `toBpm` lasts when it spans `beats`
 *  beats: a tempo linear in time averages the two ends. */
export function rampDurationMs(
  fromBpm: number,
  toBpm: number,
  beats: number,
): number {
  const mean = (fromBpm + toBpm) / 2;
  return (beats * MS_PER_MINUTE) / (mean > 0 ? mean : fromBpm);
}

/** Beats a ramp covers from `elapsed` ms in to its end. */
function rampBeatsFrom(r: TempoRamp, elapsed: number): number {
  const t = r.totalMs;
  return (
    (r.fromBpm * (t - elapsed) +
      ((r.toBpm - r.fromBpm) * (t * t - elapsed * elapsed)) / (2 * t)) /
    MS_PER_MINUTE
  );
}

/** Beats covered `dt` ms after the start of a segment. */
function segmentBeatsIn(seg: TempoSegment, dt: number): number {
  const r = seg.ramp;
  if (!r) return (dt * seg.bpm) / MS_PER_MINUTE;
  const left = r.totalMs - r.elapsedMs;
  const remaining = rampBeatsFrom(r, r.elapsedMs);
  if (dt >= left) return remaining + ((dt - left) * seg.bpm) / MS_PER_MINUTE;
  return remaining - rampBeatsFrom(r, r.elapsedMs + dt);
}

/** Ms needed to cover `beats` beats from the start of a segment: the
 *  inverse of `segmentBeatsIn`. */
function segmentMsFor(seg: TempoSegment, beats: number): number {
  const r = seg.ramp;
  if (!r) return (beats * MS_PER_MINUTE) / seg.bpm;
  const remaining = rampBeatsFrom(r, r.elapsedMs);
  if (beats > remaining) {
    return (
      r.totalMs - r.elapsedMs + ((beats - remaining) * MS_PER_MINUTE) / seg.bpm
    );
  }
  // beats = b * dt + a * dt^2
  const a = (r.toBpm - r.fromBpm) / (2 * r.totalMs * MS_PER_MINUTE);
  const b =
    (r.fromBpm + ((r.toBpm - r.fromBpm) * r.elapsedMs) / r.totalMs) /
    MS_PER_MINUTE;
  if (Math.abs(a) < 1e-15) return beats / b;
  const disc = b * b + 4 * a * beats;
  return (-b + Math.sqrt(Math.max(0, disc))) / (2 * a);
}

/** The tempo `dt` ms after the start of a segment. */
function segmentBpmAt(seg: TempoSegment, dt: number): number {
  const r = seg.ramp;
  if (!r) return seg.bpm;
  const t = Math.min(1, Math.max(0, (r.elapsedMs + dt) / r.totalMs));
  return r.fromBpm + (r.toBpm - r.fromBpm) * t;
}

/** The ramp still in progress `dt` ms after the start of a segment. */
function rampAfter(seg: TempoSegment, dt: number): TempoRamp | undefined {
  const r = seg.ramp;
  if (!r || r.elapsedMs + dt >= r.totalMs) return undefined;
  return { ...r, elapsedMs: r.elapsedMs + dt };
}

function meterAtMeasure(meters: MeterSpan[], measure: number): MeterSpan {
  return lastAtOrBefore(meters, measure, (m) => m.measure);
}

/** Quarter notes from bar 1 beat 1 to a bar and (quarter-note) beat. */
function beatsAtMeasureBeat(
  meters: MeterSpan[],
  measure: number,
  beat: number,
): number {
  const m = meterAtMeasure(meters, measure);
  return m.startBeats + (measure - m.measure) * m.quarters + (beat - 1);
}

/** The bar and (quarter-note) beat of a position in quarter notes; bar 1
 *  for one before it. */
function measureBeatAtBeats(
  meters: MeterSpan[],
  beats: number,
): { measure: number; beat: number } {
  const at = Math.max(0, Math.round(beats * 1e6) / 1e6);
  const m = lastAtOrBefore(meters, at, (span) => span.startBeats);
  const bars = Math.floor((at - m.startBeats) / m.quarters + 1e-9);
  return {
    measure: m.measure + bars,
    beat: 1 + Math.max(0, at - m.startBeats - bars * m.quarters),
  };
}

/** Measures elapsed (0 at bar 1 beat 1, fractional within a bar). */
function measuresAtBeats(meters: MeterSpan[], beats: number): number {
  if (beats < 0) return beats / meters[0].quarters;
  const { measure, beat } = measureBeatAtBeats(meters, beats);
  return measure - 1 + (beat - 1) / meterAtMeasure(meters, measure).quarters;
}

/** The inverse of `measuresAtBeats`. */
function beatsAtMeasures(meters: MeterSpan[], measures: number): number {
  if (measures < 0) return measures * meters[0].quarters;
  const whole = Math.floor(measures);
  const bar = whole + 1;
  const quarters = meterAtMeasure(meters, bar).quarters;
  return beatsAtMeasureBeat(meters, bar, 1 + (measures - whole) * quarters);
}

/** The last segment whose `key` is at or before `at`. */
function lastAtOrBefore<T>(items: T[], at: number, key: (t: T) => number): T {
  let lo = 0;
  let hi = items.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (key(items[mid]) <= at) lo = mid;
    else hi = mid - 1;
  }
  return items[lo];
}

function segmentAtMs(segments: TempoSegment[], ms: number): TempoSegment {
  return lastAtOrBefore(segments, ms, (s) => s.startMs);
}

function segmentAtBeats(segments: TempoSegment[], beats: number): TempoSegment {
  return lastAtOrBefore(segments, beats, (s) => s.startBeats);
}

/** Quarter notes from bar 1 beat 1 at an absolute ms position. */
function beatsAtMs(grid: TempoGrid, ms: number): number {
  const first = grid.segments[0];
  if (ms < first.startMs) {
    return ((ms - first.startMs) * first.bpm) / MS_PER_MINUTE;
  }
  const seg = segmentAtMs(grid.segments, ms);
  return seg.startBeats + segmentBeatsIn(seg, ms - seg.startMs);
}

/** The absolute ms of a position in quarter notes from bar 1 beat 1. */
function msAtBeats(grid: TempoGrid, beats: number): number {
  const first = grid.segments[0];
  if (beats < 0) return first.startMs + (beats * MS_PER_MINUTE) / first.bpm;
  const seg = segmentAtBeats(grid.segments, beats);
  return seg.startMs + segmentMsFor(seg, beats - seg.startBeats);
}

/** A transition's length in quarter notes: a bare number of beats, `Nm`
 *  measures (in `quarters` a bar), or none for a snap. */
function transitionBeats(
  transition: string | undefined,
  quarters: number,
): number | undefined {
  if (!transition) return undefined;
  const t = transition.trim();
  const measures = t.match(/^(\d+(?:\.\d+)?)m$/);
  const length = measures
    ? parseFloat(measures[1]) * quarters
    : /^\d+(?:\.\d+)?$/.test(t)
      ? parseFloat(t)
      : NaN;
  return Number.isFinite(length) && length > 0 ? length : undefined;
}

function positiveBpm(bpm: number | undefined, fallback: number): number {
  return bpm !== undefined && Number.isFinite(bpm) && bpm > 0 ? bpm : fallback;
}

/** Resolve a tempo section as the server's tempo map does: changes are taken
 *  in time order, each reached through the tempo (and any ramp) of those
 *  before it; a time signature set mid-bar governs from the next bar; a ramp
 *  over N beats spans exactly N beats, and one over N measures the beats in
 *  N bars of the signature set at its position. */
function buildTempoGrid(tempo: TempoSection): TempoGrid {
  const key = tempoKey(tempo);
  const cached = gridCache.get(tempo);
  if (cached && sameKey(cached.key, key)) return cached.grid;
  const grid = resolveTempoGrid(tempo);
  gridCache.set(tempo, { key, grid });
  return grid;
}

/** Resolved grids by tempo section. A section can be edited in place, so
 *  each is kept with the values it was resolved from and checked against
 *  them: a walk over the changes, far cheaper than resolving them again. */
const gridCache = new WeakMap<
  TempoSection,
  { key: (number | string | undefined)[]; grid: TempoGrid }
>();

function tempoKey(tempo: TempoSection): (number | string | undefined)[] {
  const key: (number | string | undefined)[] = [
    tempo.start.value,
    tempo.start.unit,
    tempo.bpm,
    tempo.time_signature[0],
    tempo.time_signature[1],
    tempo.changes.length,
  ];
  for (const c of tempo.changes) {
    const ts = c.timestamp;
    key.push(
      ts.type,
      ts.ms,
      ts.measure,
      ts.beat,
      c.bpm,
      c.time_signature?.[0],
      c.time_signature?.[1],
      c.transition,
    );
  }
  return key;
}

function sameKey(
  a: (number | string | undefined)[],
  b: (number | string | undefined)[],
): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function resolveTempoGrid(tempo: TempoSection): TempoGrid {
  const initialSig = tempo.time_signature;
  const initialBpm = positiveBpm(tempo.bpm, 120);
  const startMs = durationToMs(tempo.start);
  const segments: TempoSegment[] = [
    {
      startMs,
      startMeasure: 1,
      startBeat: 1,
      startBeats: 0,
      bpm: initialBpm,
      beatsPerMeasure: initialSig[0],
      beatValue: initialSig[1],
    },
  ];
  const meters: MeterSpan[] = [
    {
      measure: 1,
      startBeats: 0,
      quarters: quartersPerMeasure(initialSig),
      signature: [initialSig[0], initialSig[1]],
    },
  ];
  const changes: ResolvedChange[] = [];
  const grid = {
    segments,
    meters,
    changes,
    anyTimed: tempo.changes.some((c) => c.timestamp.type === "absolute"),
  };

  const timed = tempo.changes
    .filter((c) => c.timestamp.type === "absolute")
    .sort((a, b) => (a.timestamp.ms ?? 0) - (b.timestamp.ms ?? 0));
  const barred = tempo.changes
    .filter((c) => c.timestamp.type !== "absolute")
    .sort(
      (a, b) =>
        (a.timestamp.measure ?? 1) - (b.timestamp.measure ?? 1) ||
        (a.timestamp.beat ?? 1) - (b.timestamp.beat ?? 1),
    );
  const barredBeats = (c: TempoChange) =>
    beatsAtMeasureBeat(meters, c.timestamp.measure ?? 1, c.timestamp.beat ?? 1);
  let sig: [number, number] = [initialSig[0], initialSig[1]];

  let ti = 0;
  let bi = 0;
  while (ti < timed.length || bi < barred.length) {
    const cur = segments[segments.length - 1];
    let change: TempoChange;
    let beats: number;
    let dt: number;
    const barredAt =
      bi < barred.length
        ? cur.startMs +
          segmentMsFor(cur, barredBeats(barred[bi]) - cur.startBeats)
        : Infinity;
    if (ti < timed.length && (timed[ti].timestamp.ms ?? 0) <= barredAt) {
      change = timed[ti++];
      // A change before bar 1 counts as at it.
      dt = Math.max(0, (change.timestamp.ms ?? 0) - cur.startMs);
      beats = cur.startBeats + segmentBeatsIn(cur, dt);
    } else {
      change = barred[bi++];
      beats = barredBeats(change);
      dt = barredAt - cur.startMs;
    }

    if (change.time_signature) {
      sig = change.time_signature;
      // As the server places it: a change written on a bar's beat 1
      // governs from that bar, one written anywhere else (even past the
      // bar's end) from the next; a timed change by where it falls.
      const at =
        change.timestamp.type === "absolute"
          ? measureBeatAtBeats(meters, beats)
          : {
              measure: change.timestamp.measure ?? 1,
              beat: change.timestamp.beat ?? 1,
            };
      const from = Math.abs(at.beat - 1) < 0.001 ? at.measure : at.measure + 1;
      const span = {
        measure: from,
        startBeats: beatsAtMeasureBeat(meters, from, 1),
        quarters: quartersPerMeasure(change.time_signature),
        signature: change.time_signature,
      };
      const later = meters.findIndex((m) => m.measure >= from);
      if (later >= 0) meters.splice(later);
      meters.push(span);
    }

    const at = measureBeatAtBeats(meters, beats);
    const next: TempoSegment = {
      startMs: cur.startMs + dt,
      startMeasure: at.measure,
      startBeat: at.beat,
      startBeats: beats,
      bpm: cur.bpm,
      ramp: rampAfter(cur, dt),
      beatsPerMeasure: sig[0],
      beatValue: sig[1],
    };
    if (change.bpm !== undefined && positiveBpm(change.bpm, 0) > 0) {
      // A ramp in measures counts them in the signature set at its
      // position, by its own change or another beside it of either kind,
      // listed before or after it. Those still to come are at the fronts
      // of the queues.
      let sizingSig = sig;
      if (change.transition?.trim().endsWith("m")) {
        const changeMs = next.startMs;
        for (let i = ti; i < timed.length; i++) {
          if (Math.abs((timed[i].timestamp.ms ?? 0) - changeMs) > 1e-6) break;
          const ts = timed[i].time_signature;
          if (ts) sizingSig = ts;
        }
        for (let i = bi; i < barred.length; i++) {
          if (barredBeats(barred[i]) > beats + 1e-9) break;
          const ts = barred[i].time_signature;
          if (ts) sizingSig = ts;
        }
      }
      const length = transitionBeats(
        change.transition,
        quartersPerMeasure(sizingSig),
      );
      const fromBpm = segmentBpmAt(cur, dt);
      next.ramp =
        length === undefined
          ? undefined
          : {
              fromBpm,
              toBpm: change.bpm,
              totalMs: rampDurationMs(fromBpm, change.bpm, length),
              elapsedMs: 0,
            };
      next.bpm = change.bpm;
      changes.push({
        timed: change.timestamp.type === "absolute",
        bpm: change.bpm,
        rampBeats: length,
      });
    } else {
      changes.push({ timed: change.timestamp.type === "absolute" });
    }
    segments.push(next);
  }

  return grid;
}

/** Build a sorted array of tempo segments from a TempoSection: one per
 *  change, each carrying any ramp still in progress where it starts. */
export function buildTempoSegments(tempo: TempoSection): TempoSegment[] {
  return buildTempoGrid(tempo).segments;
}

/** A stretch of the tempo lane with one label. */
export interface TempoLaneSpan {
  startMs: number;
  /** e.g. "120 BPM, 4/4", or "132.5 → 180 BPM, 4/4" while a ramp runs */
  label: string;
}

/** The tempo lane's stretches: a new one wherever the tempo or the
 *  signature in effect changes -- at each change, where a ramp ends, and
 *  at the barline a mid-bar signature change governs from. Mid-ramp, a
 *  stretch shows the tempo where it starts and where the ramp goes. */
export function tempoLaneSpans(tempo: TempoSection): TempoLaneSpan[] {
  const grid = buildTempoGrid(tempo);
  const bounds = new Set<number>();
  for (const seg of grid.segments) {
    bounds.add(seg.startMs);
    if (seg.ramp)
      bounds.add(seg.startMs + seg.ramp.totalMs - seg.ramp.elapsedMs);
  }
  for (const m of grid.meters) bounds.add(msAtBeats(grid, m.startBeats));
  const round = (bpm: number) => Math.round(bpm * 100) / 100;
  const spans: TempoLaneSpan[] = [];
  for (const ms of [...bounds].sort((a, b) => a - b)) {
    const seg = segmentAtMs(grid.segments, ms);
    const dt = ms - seg.startMs;
    const r = seg.ramp;
    const inRamp = !!r && r.elapsedMs + dt < r.totalMs - 1e-6;
    const bpm = inRamp
      ? `${round(segmentBpmAt(seg, dt))} → ${round(r!.toBpm)}`
      : `${round(seg.bpm)}`;
    const { measure } = measureBeatAtBeats(grid.meters, beatsAtMs(grid, ms));
    const [num, den] = meterAtMeasure(grid.meters, measure).signature;
    const label = `${bpm} BPM, ${num}/${den}`;
    const last = spans[spans.length - 1];
    // A change that changes nothing yet (a signature set mid-bar) starts no
    // stretch of its own.
    if (!last || last.label !== label) spans.push({ startMs: ms, label });
  }
  return spans;
}

/** Convert a Duration to milliseconds. */
function durationToMs(d: { value: number; unit: string }): number {
  switch (d.unit) {
    case "ms":
      return d.value;
    case "s":
      return d.value * 1000;
    default:
      return 0;
  }
}

/** The ms of a beat position for a cue under an offset of `offsetMs`, as
 *  the player places it: the tempo changes positioned in time are taken
 *  `offsetMs` later (each at the beat the unshifted map has there), those
 *  positioned by bar stay on their bars. The player's own walk, step for
 *  step; with no offset, or no change positioned in time, it is the
 *  unshifted position. */
function msAtBeatsOffset(
  grid: TempoGrid,
  beats: number,
  offsetMs: number,
): number {
  if (offsetMs === 0 || !grid.anyTimed || beats < 0) {
    return msAtBeats(grid, beats);
  }
  const first = grid.segments[0];
  let cur: TempoSegment = { ...first, ramp: undefined };
  for (let i = 0; i < grid.changes.length; i++) {
    const change = grid.changes[i];
    const seg = grid.segments[i + 1];
    const changeBeats = change.timed
      ? Math.max(0, beatsAtMs(grid, seg.startMs + offsetMs))
      : seg.startBeats;
    if (changeBeats > beats) break;
    const dt = Math.max(0, segmentMsFor(cur, changeBeats - cur.startBeats));
    const bpmAtChange = segmentBpmAt(cur, dt);
    const next: TempoSegment = {
      ...cur,
      startMs: cur.startMs + dt,
      startBeats: changeBeats,
      ramp: rampAfter(cur, dt),
    };
    if (change.bpm !== undefined) {
      next.bpm = change.bpm;
      next.ramp =
        change.rampBeats === undefined
          ? undefined
          : {
              fromBpm: bpmAtChange,
              toBpm: change.bpm,
              totalMs: rampDurationMs(
                bpmAtChange,
                change.bpm,
                change.rampBeats,
              ),
              elapsedMs: 0,
            };
    }
    cur = next;
  }
  return cur.startMs + Math.max(0, segmentMsFor(cur, beats - cur.startBeats));
}

/** Convert a Timestamp to absolute milliseconds: its place in the tempo
 *  map, or, with `offsetMs`, the score time the player gives a cue there
 *  under that offset (before the offset itself is added). */
export function timestampToMs(
  ts: Timestamp,
  tempo?: TempoSection,
  offsetMs = 0,
): number {
  if (ts.type === "absolute") {
    return ts.ms ?? 0;
  }

  // measure_beat type
  if (!tempo) return 0;

  const grid = buildTempoGrid(tempo);
  return msAtBeatsOffset(
    grid,
    beatsAtMeasureBeat(grid.meters, ts.measure ?? 1, ts.beat ?? 1),
    offsetMs,
  );
}

/**
 * Convert absolute ms to a Timestamp.
 * If preferType is 'measure_beat' and tempo is available, produces measure/beat.
 */
export function msToTimestamp(
  ms: number,
  preferType: "absolute" | "measure_beat",
  tempo?: TempoSection,
): Timestamp {
  if (preferType === "absolute" || !tempo) {
    return { type: "absolute", ms };
  }

  const grid = buildTempoGrid(tempo);
  const at = measureBeatAtBeats(grid.meters, beatsAtMs(grid, ms));
  let measure = at.measure;
  let beat = Math.round(at.beat);
  if (beat - 1 >= meterAtMeasure(grid.meters, measure).quarters) {
    measure += 1;
    beat = 1;
  }

  return { type: "measure_beat", measure, beat };
}

/** Generate grid lines (measures and beats) for a visible time range. */
export function getGridLines(
  tempo: TempoSection,
  viewStartMs: number,
  viewEndMs: number,
  pixelsPerMs?: number,
): GridLine[] {
  const lines: GridLine[] = [];
  const grid = buildTempoGrid(tempo);
  const MAX_LINES = 100000;

  let measure = measureBeatAtBeats(
    grid.meters,
    beatsAtMs(grid, viewStartMs),
  ).measure;
  while (lines.length < MAX_LINES) {
    const quarters = meterAtMeasure(grid.meters, measure).quarters;
    const barBeats = beatsAtMeasureBeat(grid.meters, measure, 1);
    const barMs = msAtBeats(grid, barBeats);
    if (!(barMs <= viewEndMs)) break;
    const measureDurationMs = msAtBeats(grid, barBeats + quarters) - barMs;

    // Determine label and beat-line density based on pixels per measure.
    // At low zoom, skip labels and beat lines to avoid overlap.
    const pxPerMeasure = pixelsPerMs
      ? pixelsPerMs * measureDurationMs
      : Infinity;

    // labelEvery: show a label every N measures (1 = every measure)
    let labelEvery = 1;
    if (pxPerMeasure < 15) labelEvery = 16;
    else if (pxPerMeasure < 25) labelEvery = 8;
    else if (pxPerMeasure < 40) labelEvery = 4;
    else if (pxPerMeasure < 60) labelEvery = 2;

    // Hide beat lines when they'd be < 6px apart
    const showBeats = pxPerMeasure / quarters >= 6;

    if (barMs >= viewStartMs) {
      const showLabel = (measure - 1) % labelEvery === 0;
      lines.push({
        ms: barMs,
        type: "measure",
        label: showLabel ? `${measure}` : undefined,
      });
    }
    if (showBeats) {
      for (let q = 1; q < quarters - 1e-9; q++) {
        const ms = msAtBeats(grid, barBeats + q);
        if (ms > viewEndMs) break;
        if (ms >= viewStartMs) lines.push({ ms, type: "beat" });
      }
    }
    measure++;
  }

  return lines;
}

/** Format milliseconds as MM:SS.mmm */
export function formatMs(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  const millis = Math.round(ms % 1000);
  return `${minutes}:${String(seconds).padStart(2, "0")}.${String(millis).padStart(3, "0")}`;
}

/**
 * Convert an effect duration string (e.g. "5s", "500ms", "2beats", "4measures")
 * to milliseconds. Requires tempo for beat/measure units; returns 0 if tempo is
 * missing and the unit requires it. An optional `atMs` parameter provides the
 * playback position for tempo-aware conversions (defaults to 0).
 */
export function durationStringToMs(
  durationStr: string | undefined,
  tempo?: TempoSection,
  atMs: number = 0,
): number {
  if (!durationStr) return 0;
  const str = durationStr.trim();
  const match = str.match(/^(\d+(?:\.\d+)?)\s*(ms|s|beats?|measures?)$/);
  if (!match) {
    const val = parseFloat(str);
    return isNaN(val) ? 0 : val * 1000; // bare number = seconds
  }
  const value = parseFloat(match[1]);
  const rawUnit = match[2];
  if (rawUnit === "ms") return value;
  if (rawUnit === "s") return value * 1000;
  if (!tempo) return 0;
  if (rawUnit === "beat" || rawUnit === "beats") {
    return beatsToMs(value, atMs, tempo);
  }
  if (rawUnit === "measure" || rawUnit === "measures") {
    return measuresToMs(value, atMs, tempo);
  }
  return value * 1000;
}

/**
 * Convert a duration in milliseconds to a duration string suitable for DSL output.
 * When tempo is available, prefers measure/beat-based units if the duration aligns
 * cleanly. Falls back to time-based representation (e.g. "5s", "500ms").
 */
export function msToDurationString(
  ms: number,
  tempo?: TempoSection,
  atMs: number = 0,
): string {
  if (tempo) {
    const grid = buildTempoGrid(tempo);
    const startBeats = beatsAtMs(grid, atMs);
    const endBeats = beatsAtMs(grid, atMs + ms);

    // Prefer whole measures
    const measures =
      measuresAtBeats(grid.meters, endBeats) -
      measuresAtBeats(grid.meters, startBeats);
    if (measures >= 1 && Math.abs(measures - Math.round(measures)) < 0.001) {
      const rounded = Math.round(measures);
      return rounded === 1 ? "1measure" : `${rounded}measures`;
    }
    // Then whole beats
    const beats = endBeats - startBeats;
    if (beats >= 1 && Math.abs(beats - Math.round(beats)) < 0.001) {
      const rounded = Math.round(beats);
      return rounded === 1 ? "1beat" : `${rounded}beats`;
    }
  }
  // Fall back to time-based
  if (ms < 1000 || ms % 1000 !== 0) {
    return `${Math.round(ms)}ms`;
  }
  return `${ms / 1000}s`;
}

/** Pixel <-> time conversion helpers */
export function msToPixel(ms: number, pixelsPerMs: number): number {
  return ms * pixelsPerMs;
}

export function pixelToMs(px: number, pixelsPerMs: number): number {
  return px / pixelsPerMs;
}

/** Snap a time to the nearest grid line */
export type SnapResolution =
  | "measure"
  | "beat"
  | "1/2"
  | "1/4"
  | "1/8"
  | "1/16";

const SNAP_BEATS: Record<Exclude<SnapResolution, "measure">, number> = {
  beat: 1,
  "1/2": 0.5,
  "1/4": 0.25,
  "1/8": 0.125,
  "1/16": 0.0625,
};

export function snapToNearestGrid(
  ms: number,
  tempo: TempoSection,
  resolution: SnapResolution,
): number {
  const grid = buildTempoGrid(tempo);
  const beats = beatsAtMs(grid, ms);
  if (resolution === "measure") {
    const measures = Math.round(measuresAtBeats(grid.meters, beats));
    return msAtBeats(grid, beatsAtMeasures(grid.meters, measures));
  }
  // Steps count from the bar's downbeat, so an odd metre's bar keeps its
  // subdivisions.
  const { measure } = measureBeatAtBeats(grid.meters, beats);
  const bar = beatsAtMeasureBeat(grid.meters, measure, 1);
  const step = SNAP_BEATS[resolution];
  return msAtBeats(grid, bar + Math.round((beats - bar) / step) * step);
}

/** Snap a duration to the nearest grid step at a given playback position. */
export function snapDurationToGrid(
  durationMs: number,
  atMs: number,
  tempo: TempoSection,
  resolution: SnapResolution,
): number {
  const grid = buildTempoGrid(tempo);
  const startBeats = beatsAtMs(grid, atMs);
  const endBeats = beatsAtMs(grid, atMs + durationMs);
  if (resolution === "measure") {
    const from = measuresAtBeats(grid.meters, startBeats);
    const span = measuresAtBeats(grid.meters, endBeats) - from;
    const measures = Math.max(1, Math.round(span));
    return (
      msAtBeats(grid, beatsAtMeasures(grid.meters, from + measures)) - atMs
    );
  }
  const step = SNAP_BEATS[resolution];
  const beats = Math.max(
    step,
    Math.round((endBeats - startBeats) / step) * step,
  );
  return msAtBeats(grid, startBeats + beats) - atMs;
}

/** An offset marker on the timeline. */
export interface OffsetMarker {
  /** Adjusted ms position (for drawing the marker) */
  ms: number;
  /** Raw ms position from the tempo grid (before offsets applied) */
  rawMs: number;
  /** Duration of this offset in ms */
  durationMs: number;
  type: "offset" | "reset";
  measures: number;
  /** Which show this offset belongs to */
  showIndex: number;
  /** Which cue within the show has this offset */
  cueIndex: number;
}

/**
 * Generate grid lines shifted by cumulative offsets, so measure/beat
 * positions align with offset-adjusted cue positions.
 */
export function getAdjustedGridLines(
  tempo: TempoSection,
  viewStartMs: number,
  viewEndMs: number,
  offsets: OffsetMarker[],
  pixelsPerMs?: number,
): GridLine[] {
  if (offsets.length === 0) {
    return getGridLines(tempo, viewStartMs, viewEndMs, pixelsPerMs);
  }

  // Sort offsets by rawMs
  const sorted = [...offsets]
    .filter((o) => o.type === "offset" && o.durationMs > 0)
    .sort((a, b) => a.rawMs - b.rawMs);

  // Compute cumulative offset durations at each offset point
  const cumulativeAt: { rawMs: number; cumulativeMs: number }[] = [];
  let cumulative = 0;
  for (const off of sorted) {
    cumulative += off.durationMs;
    cumulativeAt.push({ rawMs: off.rawMs, cumulativeMs: cumulative });
  }

  // Convert view range from adjusted back to raw to know what grid lines to generate
  // adjusted = raw + cumulativeOffset => raw = adjusted - cumulativeOffset
  // We need a generous raw range to ensure we generate all visible grid lines
  const totalOffset = cumulative;
  const rawStart = Math.max(0, viewStartMs - totalOffset);
  const rawEnd = viewEndMs;

  const rawLines = getGridLines(tempo, rawStart, rawEnd, pixelsPerMs);

  // Shift each grid line by the cumulative offset at its raw position.
  // Use >= so that a grid line coinciding with an offset appears BEFORE the gap.
  const result: GridLine[] = [];
  for (const line of rawLines) {
    let offsetMs = 0;
    for (const c of cumulativeAt) {
      if (c.rawMs >= line.ms) break;
      offsetMs = c.cumulativeMs;
    }
    const adjustedMs = line.ms + offsetMs;
    // Only include lines visible in the adjusted view range
    if (adjustedMs >= viewStartMs && adjustedMs <= viewEndMs) {
      result.push({ ...line, ms: adjustedMs });
    }
  }

  return result;
}

/** A cue with its adjusted (real playback) ms position accounting for cumulative offsets. */
export interface AdjustedCuePosition {
  cue: Cue;
  index: number;
  /** The adjusted ms position (raw timestamp + cumulative offset) */
  adjustedMs: number;
  /** The cue's own position in the tempo map, before offsets: where the
   *  player times its durations from */
  rawMs: number;
  /** The cumulative offset in ms applied to this cue */
  cumulativeOffsetMs: number;
  /** How long this cue's own `offset` lasts (0 for none) */
  offsetMs: number;
  /** The cumulative offset applied to the cues after this one */
  nextOffsetMs: number;
}

/** How long an `offset N measures` issued at `anchorMs` lasts, as the
 *  player sizes it: N bars at the one tempo and signature in effect at the
 *  anchor, not following any change or ramp inside them. */
export function offsetToMs(
  measures: number,
  anchorMs: number,
  tempo: TempoSection,
): number {
  const grid = buildTempoGrid(tempo);
  const { measure } = measureBeatAtBeats(
    grid.meters,
    beatsAtMs(grid, anchorMs),
  );
  const quarters = meterAtMeasure(grid.meters, measure).quarters;
  return (measures * quarters * MS_PER_MINUTE) / getTempoAtMs(anchorMs, tempo);
}

/** How long the N measures from a given ms position last, through any
 *  tempo change, ramp or signature change inside them. */
export function measuresToMs(
  measures: number,
  atMs: number,
  tempo: TempoSection,
): number {
  const grid = buildTempoGrid(tempo);
  const from = measuresAtBeats(grid.meters, beatsAtMs(grid, atMs));
  return msAtBeats(grid, beatsAtMeasures(grid.meters, from + measures)) - atMs;
}

/** How long the N beats from a given ms position last, through any tempo
 *  change or ramp inside them. */
export function beatsToMs(
  beats: number,
  atMs: number,
  tempo: TempoSection,
): number {
  const grid = buildTempoGrid(tempo);
  return msAtBeats(grid, beatsAtMs(grid, atMs) + beats) - atMs;
}

/**
 * Compute adjusted cue positions for a show's cues, applying cumulative offsets
 * as the player does. Each cue's `offset` shifts all subsequent cues forward in
 * real time and a `reset_measures` drops the shift; an offset is sized at the
 * tempo where its cue sits in the tempo map, before any offset (or, for a cue
 * at 0, where the previous cue sits).
 */
export function computeAdjustedCuePositions(
  cues: Cue[],
  tempo?: TempoSection,
): AdjustedCuePosition[] {
  let cumulativeOffsetMs = 0;
  let lastAbsMs: number | undefined;
  const startMs = tempo ? durationToMs(tempo.start) : 0;
  return cues.map((cue, index) => {
    const rawMs = timestampToMs(cue.timestamp, tempo);
    const applied = cumulativeOffsetMs;
    // Placed as the player places it: under an offset, the tempo changes
    // positioned in time move with it.
    const adjustedMs =
      (applied !== 0 ? timestampToMs(cue.timestamp, tempo, applied) : rawMs) +
      applied;
    let offsetMs = 0;
    const offsets =
      cue.offset_measures !== undefined && cue.offset_measures > 0;
    if (tempo && (offsets || cue.reset_measures)) {
      if (cue.reset_measures) cumulativeOffsetMs = 0;
      if (offsets) {
        const anchor =
          rawMs !== 0
            ? rawMs
            : lastAbsMs !== undefined
              ? Math.max(0, lastAbsMs - applied - startMs)
              : 0;
        offsetMs = offsetToMs(cue.offset_measures!, anchor, tempo);
        cumulativeOffsetMs += offsetMs;
      }
    }
    lastAbsMs = adjustedMs;
    return {
      cue,
      index,
      adjustedMs,
      rawMs,
      cumulativeOffsetMs: applied,
      offsetMs,
      nextOffsetMs: cumulativeOffsetMs,
    };
  });
}

/**
 * Expand sequence references in a show's cues into concrete effect cues.
 * Returns a new cue array with sequence effects inserted at the correct
 * timestamps, each marked with `sequenceName`. The original cues are
 * preserved unchanged; expanded cues are added alongside them.
 */
export function expandSequencesIntoCues(
  showCues: Cue[],
  sequences: Sequence[],
  tempo?: TempoSection,
): Cue[] {
  const seqMap = new Map<string, Sequence>();
  for (const s of sequences) seqMap.set(s.name, s);

  const expanded: Cue[] = [];

  for (const cue of showCues) {
    // Always include the original cue
    expanded.push(cue);

    // Expand sequence references into additional cues
    for (const ref of cue.sequences) {
      if (ref.stop) continue; // stop commands don't produce effects
      const seq = seqMap.get(ref.name);
      if (!seq || seq.cues.length === 0) continue;

      const refMs = timestampToMs(cue.timestamp, tempo);

      const seqDurationMs = getSequenceIterationMs(seq, refMs, tempo);
      const loopCount = parseLoopCount(ref.loop);

      // BPM context for rescaling cue positions within the sequence
      const seqBaseMs = timestampToMs(seq.cues[0].timestamp, tempo);
      const seqParseBpm = getTempoAtMs(0, tempo);
      const refBpm = getTempoAtMs(refMs, tempo);
      const msPerBeatAtParse = 60000 / seqParseBpm;
      const msPerBeatAtRef = 60000 / refBpm;

      for (let iter = 0; iter < loopCount; iter++) {
        const iterOffsetMs = refMs + iter * seqDurationMs;

        for (const seqCue of seq.cues) {
          // Convert this cue's position to a beat offset, then to ms at ref tempo
          const scMs = timestampToMs(seqCue.timestamp, tempo);
          const relBeats = (scMs - seqBaseMs) / msPerBeatAtParse;
          const relMs = relBeats * msPerBeatAtRef;
          const absoluteMs = iterOffsetMs + relMs;

          if (seqCue.effects.length === 0) continue;

          const taggedEffects: CueEffect[] = seqCue.effects.map((e) => ({
            ...e,
            sequenceName: ref.name,
          }));

          expanded.push({
            timestamp: absoluteTimestamp(absoluteMs),
            effects: taggedEffects,
            commands: [],
            sequences: [],
          });
        }
      }
    }
  }

  // Sort by timestamp so cue positions compute correctly
  const at = new Map(
    expanded.map((c) => [c, timestampToMs(c.timestamp, tempo)]),
  );
  expanded.sort((a, b) => at.get(a)! - at.get(b)!);

  return expanded;
}

/**
 * Compute the duration of a single iteration of a sequence in ms,
 * rescaled to the tempo active at a given reference point.
 */
export function getSequenceIterationMs(
  seq: Sequence,
  refMs: number,
  tempo?: TempoSection,
): number {
  if (seq.cues.length === 0) return 1000;
  // A declared `length:` is the pass length, however long the effects run.
  if (seq.length) {
    const lengthMs = durationStringToMs(seq.length, tempo, refMs);
    if (lengthMs > 0) return lengthMs;
  }

  const seqBaseMs = timestampToMs(seq.cues[0].timestamp, tempo);
  const seqParseBpm = getTempoAtMs(0, tempo);
  const refBpm = getTempoAtMs(refMs, tempo);
  const msPerBeatAtParse = 60000 / seqParseBpm;
  const msPerBeatAtRef = 60000 / refBpm;

  let maxBeats = 0;
  for (const sc of seq.cues) {
    const scMs = timestampToMs(sc.timestamp, tempo);
    const relBeats = (scMs - seqBaseMs) / msPerBeatAtParse;
    for (const eff of sc.effects) {
      const durStr = eff.effect.duration ?? eff.effect.hold_time;
      const durMs = durationStringToMs(durStr, tempo, refMs);
      const durBeats = durMs / msPerBeatAtRef;
      const endBeats = relBeats + durBeats;
      if (endBeats > maxBeats) maxBeats = endBeats;
    }
    if (relBeats > maxBeats) maxBeats = relBeats;
  }
  if (maxBeats <= 0) maxBeats = 4;

  return maxBeats * msPerBeatAtRef;
}

/** Get the BPM at a given ms position (mid-ramp, the ramp's tempo there). */
export function getTempoAtMs(ms: number, tempo?: TempoSection): number {
  if (!tempo) return 120; // default
  const grid = buildTempoGrid(tempo);
  const seg = segmentAtMs(grid.segments, ms);
  return segmentBpmAt(seg, Math.max(0, ms - seg.startMs));
}

function parseLoopCount(loop?: string): number {
  if (!loop) return 1;
  const n = parseInt(loop, 10);
  if (!isNaN(n) && n > 0) return n;
  if (loop === "loop") return 100; // practical limit for display
  return 1;
}

/** Effect type color coding */
export function effectTypeColor(type: string): string {
  switch (type) {
    case "static":
      return "#3b82f6";
    case "cycle":
      return "#8b5cf6";
    case "strobe":
      return "#eab308";
    case "pulse":
      return "#22c55e";
    case "chase":
      return "#f97316";
    case "dimmer":
      return "#6b7280";
    case "rainbow":
      return "#ec4899";
    case "move":
      return "#14b8a6";
    default:
      return "#6b7280";
  }
}
