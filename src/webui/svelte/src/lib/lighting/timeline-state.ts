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

// The lighting timeline's layout. Every position in it -- bars and beats
// through tempo changes and ramps, offsets, sequences played from a cue -- is
// computed by the player's own tempo map and cue clock, compiled to
// WebAssembly (`../tempo-wasm`). What is here is the timeline's: its ms, its
// grid lines and labels, snapping, and the offset markers it draws.

import type {
  Timestamp,
  TempoSection,
  Cue,
  Sequence,
  CueEffect,
} from "./types";
import { absoluteTimestamp } from "./types";
import {
  CueClock,
  TempoMap,
  rampDuration,
  type TempoMapInput,
  type TempoMapPosition,
} from "../tempo-wasm";

const MS_PER_MINUTE = 60000;
/** The tempo the player times a sequence at without any tempo map. */
const DEFAULT_BPM = 120;

/** Grid line for rendering on the ruler. */
export interface GridLine {
  ms: number;
  type: "measure" | "beat";
  label?: string;
}

/** A beat position rounded to a millionth of a beat, as the server's
 *  `snap_beats` rounds one: below any float noise a time carries, far finer
 *  than anything musical. */
function snapBeats(beats: number): number {
  return Math.round(beats * 1e6) / 1e6;
}

/** How long a ramp from `fromBpm` to `toBpm` lasts when it spans `beats`
 *  beats, in ms. */
export function rampDurationMs(
  fromBpm: number,
  toBpm: number,
  beats: number,
): number {
  return rampDuration(fromBpm, toBpm, beats) * 1000;
}

/** A transition as the timeline writes it: a bare number of beats, `Nm`
 *  measures, or none (a snap). */
function parseTransition(
  transition: string | undefined,
): TempoMapInput["changes"][number]["transition"] {
  const t = transition?.trim() ?? "";
  const measures = t.match(/^(\d+(?:\.\d+)?)m$/);
  if (measures) return { measures: parseFloat(measures[1]) };
  if (/^\d+(?:\.\d+)?$/.test(t)) return { beats: parseFloat(t) };
  return undefined;
}

/** A tempo section as the tempo map's input. */
function mapInput(tempo: TempoSection): TempoMapInput {
  return {
    startSecs: durationToMs(tempo.start) / 1000,
    bpm: tempo.bpm,
    signature: tempo.time_signature,
    changes: tempo.changes.map((c) => ({
      at:
        c.timestamp.type === "absolute"
          ? { secs: (c.timestamp.ms ?? 0) / 1000 }
          : {
              measure: c.timestamp.measure ?? 1,
              beat: c.timestamp.beat ?? 1,
            },
      bpm: c.bpm,
      signature: c.time_signature,
      transition: parseTransition(c.transition),
    })),
  };
}

/** Resolved maps by tempo section. A section can be edited in place, so
 *  each is kept with the values it was resolved from and checked against
 *  them: a walk over the changes, far cheaper than resolving them again. */
const mapCache = new WeakMap<
  TempoSection,
  {
    key: (number | string | undefined)[];
    map: TempoMap;
  }
>();

function cached(tempo: TempoSection) {
  const key = tempoKey(tempo);
  const hit = mapCache.get(tempo);
  if (hit && sameKey(hit.key, key)) return hit;
  const entry = { key, map: new TempoMap(mapInput(tempo)) };
  mapCache.set(tempo, entry);
  return entry;
}

/** The tempo map of a tempo section, as the player resolves it. */
export function tempoMapOf(tempo: TempoSection): TempoMap {
  return cached(tempo).map;
}

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

/** Quarter notes from bar 1 beat 1 at an absolute ms position. */
function beatsAtMs(map: TempoMap, ms: number, offsetMs = 0): number {
  return map.beatsAt(ms / 1000, offsetMs / 1000);
}

/** The absolute ms of a position in quarter notes from bar 1 beat 1, as a
 *  cue there is placed under an offset of `offsetMs` (before the offset
 *  itself is added). */
function msAtBeats(map: TempoMap, beats: number, offsetMs = 0): number {
  return map.timeAtBeats(beats, offsetMs / 1000) * 1000;
}

/** Quarter notes in bar `measure`. */
function quartersIn(map: TempoMap, measure: number): number {
  return map.beatsAtPosition(measure + 1, 1) - map.beatsAtPosition(measure, 1);
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
  const map = tempoMapOf(tempo);
  const round = (bpm: number) => Math.round(bpm * 100) / 100;
  const spans: TempoLaneSpan[] = [];
  for (const secs of map.boundaries()) {
    const target = map.rampTargetAt(secs);
    const now = round(map.bpmAt(secs));
    const bpm = target === undefined ? `${now}` : `${now} → ${round(target)}`;
    const { measure } = map.positionAtBeats(map.beatsAt(secs));
    const [num, den] = map.signatureInMeasure(measure);
    const label = `${bpm} BPM, ${num}/${den}`;
    const last = spans[spans.length - 1];
    // A change that changes nothing yet (a signature set mid-bar) starts no
    // stretch of its own.
    if (!last || last.label !== label) {
      spans.push({ startMs: secs * 1000, label });
    }
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

  const map = tempoMapOf(tempo);
  return msAtBeats(
    map,
    map.beatsAtPosition(ts.measure ?? 1, ts.beat ?? 1),
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
  offsetMs = 0,
): Timestamp {
  if (preferType === "absolute" || !tempo) {
    return { type: "absolute", ms };
  }

  // With `offsetMs`, `ms` is a score time under that offset, as
  // `timestampToMs(ts, tempo, offsetMs)` gives it.
  const map = tempoMapOf(tempo);
  const at = map.positionAtBeats(beatsAtMs(map, ms, offsetMs));
  let measure = at.measure;
  let beat = Math.round(at.beat);
  if (beat - 1 >= quartersIn(map, measure)) {
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
  const map = tempoMapOf(tempo);
  const MAX_LINES = 100000;

  let measure = map.positionAtBeats(beatsAtMs(map, viewStartMs)).measure;
  let barBeats = map.beatsAtPosition(measure, 1);
  while (lines.length < MAX_LINES) {
    const nextBeats = map.beatsAtPosition(measure + 1, 1);
    const quarters = nextBeats - barBeats;
    const barMs = msAtBeats(map, barBeats);
    if (!(barMs <= viewEndMs)) break;
    const measureDurationMs = msAtBeats(map, nextBeats) - barMs;

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
        const ms = msAtBeats(map, barBeats + q);
        if (ms > viewEndMs) break;
        if (ms >= viewStartMs) lines.push({ ms, type: "beat" });
      }
    }
    measure++;
    barBeats = nextBeats;
  }

  return lines;
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
    const map = tempoMapOf(tempo);
    // Rounded to a millionth of a beat, as the server's snap_beats does:
    // two positions read back from times carry float noise, and a span of
    // one beat must not come out 0.9999999999.
    const startBeats = snapBeats(beatsAtMs(map, atMs));
    const endBeats = snapBeats(beatsAtMs(map, atMs + ms));

    // Prefer whole measures
    const measures = snapBeats(
      map.measuresAtBeats(endBeats) - map.measuresAtBeats(startBeats),
    );
    if (measures >= 1 && Math.abs(measures - Math.round(measures)) < 0.001) {
      const rounded = Math.round(measures);
      return rounded === 1 ? "1measure" : `${rounded}measures`;
    }
    // Then whole beats
    const beats = snapBeats(endBeats - startBeats);
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
  offsetMs = 0,
): number {
  // With `offsetMs`, `ms` is a score time under that offset, and so is
  // the grid it snaps to.
  const map = tempoMapOf(tempo);
  const beats = beatsAtMs(map, ms, offsetMs);
  if (resolution === "measure") {
    const measures = Math.round(map.measuresAtBeats(beats));
    return msAtBeats(map, map.beatsAtMeasures(measures), offsetMs);
  }
  // Steps count from the bar's downbeat, so an odd metre's bar keeps its
  // subdivisions.
  const { measure } = map.positionAtBeats(beats);
  const bar = map.beatsAtPosition(measure, 1);
  const step = SNAP_BEATS[resolution];
  return msAtBeats(
    map,
    bar + Math.round((beats - bar) / step) * step,
    offsetMs,
  );
}

/** Snap a duration to the nearest grid step at a given playback position. */
export function snapDurationToGrid(
  durationMs: number,
  atMs: number,
  tempo: TempoSection,
  resolution: SnapResolution,
): number {
  const map = tempoMapOf(tempo);
  const startBeats = beatsAtMs(map, atMs);
  const endBeats = beatsAtMs(map, atMs + durationMs);
  if (resolution === "measure") {
    const from = map.measuresAtBeats(startBeats);
    const span = map.measuresAtBeats(endBeats) - from;
    const measures = Math.max(1, Math.round(span));
    return msAtBeats(map, map.beatsAtMeasures(from + measures)) - atMs;
  }
  const step = SNAP_BEATS[resolution];
  const beats = Math.max(
    step,
    Math.round((endBeats - startBeats) / step) * step,
  );
  return msAtBeats(map, startBeats + beats) - atMs;
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

/** The offset and reset markers of one show's cues, where they are drawn. */
export function offsetMarkers(
  cues: Cue[],
  tempo: TempoSection | undefined,
  showIndex: number,
): OffsetMarker[] {
  const markers: OffsetMarker[] = [];
  for (const ap of computeAdjustedCuePositions(cues, tempo)) {
    if (ap.cue.offset_measures !== undefined) {
      markers.push({
        ms: ap.adjustedMs,
        rawMs: ap.rawMs,
        durationMs: ap.offsetMs,
        type: "offset",
        measures: ap.cue.offset_measures,
        showIndex,
        cueIndex: ap.index,
      });
    }
    if (ap.cue.reset_measures) {
      markers.push({
        ms: ap.adjustedMs,
        rawMs: ap.rawMs,
        durationMs: 0,
        type: "reset",
        measures: 0,
        showIndex,
        cueIndex: ap.index,
      });
    }
  }
  return markers;
}

/**
 * The cumulative offset at each raw (tempo-map) position, as the cues have
 * it: each offset adds its length after its cue and each reset drops what
 * there is, in the order the cues come. A position exactly at a marker is
 * before it, so a grid line on an offset's cue is drawn before the gap, as
 * the cue is. The one timeline the cues, grid, ruler and tempo lane share.
 */
export function offsetTimeline(
  offsets: OffsetMarker[],
): (rawMs: number) => number {
  const steps = offsets
    .filter((o) => o.type === "reset" || o.durationMs > 0)
    .map((o, order) => ({ o, order }))
    .sort(
      (a, b) =>
        a.o.rawMs - b.o.rawMs ||
        a.o.showIndex - b.o.showIndex ||
        a.o.cueIndex - b.o.cueIndex ||
        // A cue's reset comes before its own offset.
        (a.o.type === "reset" ? 0 : 1) - (b.o.type === "reset" ? 0 : 1) ||
        a.order - b.order,
    );
  const at: { rawMs: number; cumulativeMs: number }[] = [];
  let cumulative = 0;
  for (const { o } of steps) {
    cumulative = o.type === "reset" ? 0 : cumulative + o.durationMs;
    at.push({ rawMs: o.rawMs, cumulativeMs: cumulative });
  }
  return (rawMs: number) => {
    let offsetMs = 0;
    for (const c of at) {
      if (c.rawMs >= rawMs) break;
      offsetMs = c.cumulativeMs;
    }
    return offsetMs;
  };
}

/** The largest cumulative offset an offset timeline reaches. */
function maxOffset(offsets: OffsetMarker[]): number {
  const offsetAt = offsetTimeline(offsets);
  let max = 0;
  for (const o of offsets) {
    max = Math.max(max, offsetAt(o.rawMs + 1e-6));
  }
  return max;
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

  const offsetAt = offsetTimeline(offsets);

  // Convert view range from adjusted back to raw to know what grid lines to
  // generate: adjusted = raw + cumulativeOffset, so a generous raw range
  // starts the largest offset earlier.
  const rawStart = Math.max(0, viewStartMs - maxOffset(offsets));
  const rawEnd = viewEndMs;

  const rawLines = getGridLines(tempo, rawStart, rawEnd, pixelsPerMs);

  const result: GridLine[] = [];
  for (const line of rawLines) {
    const adjustedMs = line.ms + offsetAt(line.ms);
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
  return tempoMapOf(tempo).offsetDuration(measures, anchorMs / 1000) * 1000;
}

/** How long the N measures from a given ms position last, through any
 *  tempo change, ramp or signature change inside them. */
export function measuresToMs(
  measures: number,
  atMs: number,
  tempo: TempoSection,
): number {
  return tempoMapOf(tempo).measuresToDuration(measures, atMs / 1000) * 1000;
}

/** How long the N beats from a given ms position last, through any tempo
 *  change or ramp inside them. */
export function beatsToMs(
  beats: number,
  atMs: number,
  tempo: TempoSection,
): number {
  return tempoMapOf(tempo).beatsToDuration(beats, atMs / 1000) * 1000;
}

/** Where a cue is written, as the cue clock takes it. */
function cuePosition(ts: Timestamp): TempoMapPosition {
  return ts.type === "absolute"
    ? { secs: (ts.ms ?? 0) / 1000 }
    : { measure: ts.measure ?? 1, beat: ts.beat ?? 1 };
}

/**
 * Compute adjusted cue positions for a show's cues, placed as the player
 * places them (its `CueClock`): each cue's `offset` shifts all subsequent
 * cues forward in real time and a `reset_measures` drops the shift; an
 * offset is sized at the tempo where its cue sits in the tempo map, before
 * any offset (or, for a cue at 0, where the previous cue sits); under an
 * offset, the tempo changes positioned in time move with it.
 */
export function computeAdjustedCuePositions(
  cues: Cue[],
  tempo?: TempoSection,
): AdjustedCuePosition[] {
  const map = tempo ? tempoMapOf(tempo) : undefined;
  const clock = new CueClock();
  return cues.map((cue, index) => {
    const offsets =
      cue.offset_measures !== undefined && cue.offset_measures > 0;
    const placed = map
      ? clock.place(
          map,
          cuePosition(cue.timestamp),
          cue.offset_measures ?? 0,
          offsets,
          !!cue.reset_measures,
        )
      : undefined;
    if (!placed) {
      // No tempo map: a cue at a bar has no place (it is drawn at 0), and
      // an offset needs a tempo to be sized.
      const rawMs = timestampToMs(cue.timestamp, tempo);
      return {
        cue,
        index,
        adjustedMs: rawMs,
        rawMs,
        cumulativeOffsetMs: 0,
        offsetMs: 0,
        nextOffsetMs: 0,
      };
    }
    return {
      cue,
      index,
      adjustedMs: placed.time * 1000,
      rawMs:
        placed.unshifted !== undefined
          ? placed.unshifted * 1000
          : timestampToMs(cue.timestamp, tempo),
      cumulativeOffsetMs: placed.offset * 1000,
      offsetMs: placed.issued * 1000,
      nextOffsetMs: placed.after * 1000,
    };
  });
}

/** A sequence's cues and pass, in beats from its start, as the player
 *  counts them (`Sequence::beats_from_start`, `Sequence::pass_beats`):
 *  through the map it is timed in inside itself, which is the show's at its
 *  starting tempo and time signature. A pass lasts its declared `length:`,
 *  else until its last effect ends, else until its last cue starts. */
interface SequencePass {
  /** Each cue's start, in beats from the sequence's */
  cueBeats: number[];
  passBeats: number;
  /** The same in ms, for a show without a tempo map */
  cueMs: number[];
  passMs: number;
}

function sequencePass(seq: Sequence, tempo?: TempoSection): SequencePass {
  const steady = tempo && steadyTempo(tempo);
  const map = steady && tempoMapOf(steady);
  const cueAt = seq.cues.map((c) =>
    map
      ? msAtBeats(map, beatsAtTimestamp(map, c.timestamp))
      : (c.timestamp.ms ?? 0),
  );
  const startMs = cueAt.length ? Math.min(...cueAt) : 0;
  const beatsFromStart = (ms: number) =>
    snapBeats(
      map
        ? beatsAtMs(map, ms) - beatsAtMs(map, startMs)
        : ((ms - startMs) / MS_PER_MINUTE) * DEFAULT_BPM,
    );

  let endMs: number;
  const length = seq.length
    ? durationStringToMs(seq.length, steady, startMs)
    : 0;
  if (length > 0) {
    endMs = startMs + length;
  } else if (seq.cues.some((c) => c.effects.length > 0)) {
    endMs = startMs;
    seq.cues.forEach((c, i) => {
      for (const e of c.effects) {
        endMs = Math.max(
          endMs,
          cueAt[i] + effectDurationMs(e, steady, cueAt[i]),
        );
      }
    });
  } else {
    endMs = Math.max(startMs, ...cueAt);
  }
  return {
    cueBeats: cueAt.map(beatsFromStart),
    passBeats: beatsFromStart(endMs),
    cueMs: cueAt.map((ms) => ms - startMs),
    passMs: endMs - startMs,
  };
}

/** The beat position of a timestamp in `map`. */
function beatsAtTimestamp(map: TempoMap, ts: Timestamp): number {
  return ts.type === "absolute"
    ? beatsAtMs(map, ts.ms ?? 0)
    : map.beatsAtPosition(ts.measure ?? 1, ts.beat ?? 1);
}

/** A tempo section at its starting tempo and time signature, none of its
 *  changes: the timing inside a sequence. */
const steadyTempos = new WeakMap<TempoSection, TempoSection>();
function steadyTempo(tempo: TempoSection): TempoSection {
  let steady = steadyTempos.get(tempo);
  if (
    !steady ||
    steady.bpm !== tempo.bpm ||
    steady.start !== tempo.start ||
    steady.time_signature !== tempo.time_signature
  ) {
    steady = { ...tempo, changes: [] };
    steadyTempos.set(tempo, steady);
  }
  return steady;
}

/** How long an effect lasts, as the player has it (`Effect::total_duration`):
 *  up, then its hold (or its own duration), then down. */
function effectDurationMs(
  e: CueEffect,
  tempo: TempoSection | undefined,
  atMs: number,
): number {
  const p = e.effect;
  const ms = (d: string | undefined) => durationStringToMs(d, tempo, atMs);
  if (p.type === "dimmer") return ms(p.duration);
  return ms(p.up_time) + ms(p.hold_time ?? p.duration) + ms(p.down_time);
}

/** Where the cue at `cueBeats` in pass `pass` of a sequence played from
 *  `refMs` lands: placed in beats through the show's tempo map from there,
 *  as the player places it (`expand_sequence_call`). */
function passCueMs(
  pass: SequencePass,
  index: number,
  iteration: number,
  refMs: number,
  tempo?: TempoSection,
): number {
  if (!tempo) {
    return (
      refMs + iteration * pass.passMs + (index < 0 ? 0 : pass.cueMs[index])
    );
  }
  const beats =
    index < 0
      ? iteration * pass.passBeats
      : iteration * pass.passBeats + pass.cueBeats[index];
  return refMs + beatsToMs(snapBeats(beats), refMs, tempo);
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
      const pass = sequencePass(seq, tempo);
      // Passes of no length all start together: one is all there is to see.
      const passes = pass.passBeats > 0 ? parseLoopCount(ref.loop) : 1;

      for (let iter = 0; iter < passes; iter++) {
        seq.cues.forEach((seqCue, i) => {
          if (seqCue.effects.length === 0) return;
          const taggedEffects: CueEffect[] = seqCue.effects.map((e) => ({
            ...e,
            sequenceName: ref.name,
          }));
          expanded.push({
            timestamp: absoluteTimestamp(
              passCueMs(pass, i, iter, refMs, tempo),
            ),
            effects: taggedEffects,
            commands: [],
            sequences: [],
          });
        });
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
 * How long `passes` passes of a sequence played from `refMs` last, through
 * the show's tempo map from there.
 */
export function getSequencePlayMs(
  seq: Sequence,
  refMs: number,
  tempo?: TempoSection,
  passes = 1,
): number {
  if (seq.cues.length === 0) return 0;
  return passCueMs(sequencePass(seq, tempo), -1, passes, refMs, tempo) - refMs;
}

/** Get the BPM at a given ms position (mid-ramp, the ramp's tempo there). */
export function getTempoAtMs(ms: number, tempo?: TempoSection): number {
  if (!tempo) return 120; // default
  return tempoMapOf(tempo).bpmAt(ms / 1000);
}

/** The passes a sequence reference plays: `loop: N`, or a practical limit
 *  for display for `loop: loop`. */
export function parseLoopCount(loop?: string): number {
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
