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
// WebAssembly (`../tempo-wasm`), and the sequences a show plays are laid out
// by the player's own light show parser there. What is here is the
// timeline's: its ms, its grid lines and labels, snapping, and the offset
// markers it draws.

import type {
  Timestamp,
  TempoSection,
  Cue,
  Sequence,
  BlendMode,
  CueEffect,
  EffectType,
} from "./types";
import { absoluteTimestamp } from "./types";
import {
  CueClock,
  CueClockRestarted,
  DEFAULT_BPM,
  TempoMap,
  cachedTempoMap,
  tempoModuleGeneration,
  withTempoMap,
  layoutSequences,
  rampDuration,
  type LaidOutCall,
  type TempoMapInput,
  type TempoMapPosition,
} from "../tempo-wasm";
import { serializeLightFile } from "./serializer";

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

/** The tempo map of a tempo section, as the player resolves it (throws
 *  `TempoMapFailed` when it cannot be made). */
export function tempoMapOf(tempo: TempoSection): TempoMap {
  return cachedTempoMap(tempo, tempoKey(tempo), () => mapInput(tempo));
}

export { tempoFailures, tempoProblemOf } from "../tempo-wasm";

/** `use` with the tempo map of `tempo`; `without` when there is none --
 *  and when its map could not be made or failed while used, which is drawn
 *  as without a tempo (`tempoProblemOf` says why meanwhile), rather than failing.
 *  `failed`, if given, is told so. */
function withMap<T>(
  tempo: TempoSection | undefined,
  use: (map: TempoMap) => T,
  without: () => T,
  failed: () => T = without,
  walk = false,
): T {
  if (!tempo) return without();
  return withTempoMap(
    tempo,
    tempoKey(tempo),
    () => mapInput(tempo),
    use,
    failed,
    walk,
  );
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
  return withMap(
    tempo,
    (map) => spansOf(map),
    () => [],
  );
}

function spansOf(map: TempoMap): TempoLaneSpan[] {
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
  return withMap(
    tempo,
    (map) =>
      msAtBeats(
        map,
        map.beatsAtPosition(ts.measure ?? 1, ts.beat ?? 1),
        offsetMs,
      ),
    () => 0,
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
  return withMap<Timestamp>(
    tempo,
    (map) => {
      const at = map.positionAtBeats(beatsAtMs(map, ms, offsetMs));
      let measure = at.measure;
      let beat = Math.round(at.beat);
      if (beat - 1 >= quartersIn(map, measure)) {
        measure += 1;
        beat = 1;
      }
      return { type: "measure_beat", measure, beat };
    },
    () => ({ type: "absolute", ms }),
  );
}

/** Generate grid lines (measures and beats) for a visible time range. */
export function getGridLines(
  tempo: TempoSection,
  viewStartMs: number,
  viewEndMs: number,
  pixelsPerMs?: number,
): GridLine[] {
  return withMap(
    tempo,
    (map) => gridLinesOf(map, viewStartMs, viewEndMs, pixelsPerMs),
    () => [],
  );
}

function gridLinesOf(
  map: TempoMap,
  viewStartMs: number,
  viewEndMs: number,
  pixelsPerMs?: number,
): GridLine[] {
  const lines: GridLine[] = [];
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
  const musical = withMap(
    tempo,
    (map) => musicalDurationString(map, ms, atMs),
    () => undefined,
  );
  if (musical) return musical;
  // Fall back to time-based
  if (ms < 1000 || ms % 1000 !== 0) {
    return `${Math.round(ms)}ms`;
  }
  return `${ms / 1000}s`;
}

/** A span as whole measures or beats where it starts, if it is one. */
function musicalDurationString(
  map: TempoMap,
  ms: number,
  atMs: number,
): string | undefined {
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

  return undefined;
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
  return withMap(
    tempo,
    (map) => snapOnMap(map, ms, resolution, offsetMs),
    () => ms,
  );
}

function snapOnMap(
  map: TempoMap,
  ms: number,
  resolution: SnapResolution,
  offsetMs: number,
): number {
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
  return withMap(
    tempo,
    (map) => snapDurationOnMap(map, durationMs, atMs, resolution),
    () => durationMs,
  );
}

function snapDurationOnMap(
  map: TempoMap,
  durationMs: number,
  atMs: number,
  resolution: SnapResolution,
): number {
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
  return withMap(
    tempo,
    (map) => map.offsetDuration(measures, anchorMs / 1000) * 1000,
    () => 0,
  );
}

/** How long the N measures from a given ms position last, through any
 *  tempo change, ramp or signature change inside them. */
export function measuresToMs(
  measures: number,
  atMs: number,
  tempo: TempoSection,
): number {
  return withMap(
    tempo,
    (map) => map.measuresToDuration(measures, atMs / 1000) * 1000,
    () => 0,
  );
}

/** How long the N beats from a given ms position last, through any tempo
 *  change or ramp inside them. */
export function beatsToMs(
  beats: number,
  atMs: number,
  tempo: TempoSection,
): number {
  return withMap(
    tempo,
    (map) => map.beatsToDuration(beats, atMs / 1000) * 1000,
    () => 0,
  );
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
  return withMap(
    tempo,
    (map) => {
      try {
        return placeCues(cues, tempo, map);
      } catch (e) {
        // The module was started afresh part way through: the offsets so
        // far are gone, so the walk starts over in the new one -- once:
        // failing again, the cues are drawn without the tempo.
        if (!(e instanceof CueClockRestarted)) throw e;
        return placeCues(cues, tempo, tempoMapOf(tempo!));
      }
    },
    () => placeCues(cues, undefined, undefined),
    undefined,
    true,
  );
}

function placeCues(
  cues: Cue[],
  tempo: TempoSection | undefined,
  map: TempoMap | undefined,
): AdjustedCuePosition[] {
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

/** The sequences a list of cues plays, as the player's own parser and
 *  sequence expansion lay them out (`layout_sequence_calls`, through
 *  WebAssembly): the cues and the sequences they reference, written out as
 *  a show and parsed, timed through `tempo`. */
export interface ShowSequences {
  /** Each cue's calls, by the cue's index */
  calls: Map<number, LaidOutCall[]>;
  /** The cues the player could not parse, with why, by index */
  cueErrors: Map<number, string>;
  /** Why the file does not parse at all, when it does not: the calls are
   *  then the last that did */
  error?: string;
  /** Why the player would not load the file, though it lays out */
  problem?: string;
}

/** An error the player gave about the file written out for it, without the
 *  line and column, which are that file's and not the user's: its first
 *  line, the place dropped. */
function withoutPlace(message: string): string {
  return message
    .split("\n")[0]
    .replace(/\s*at line \d+, column \d+/gi, "")
    .replace(/\bline \d+(?::\d+)?[:,]?\s*/gi, "")
    .trim();
}

/** Layouts by source text, as a parse is not free. */
const layoutCache = new Map<string, ShowSequences>();
/** And by the very cues, sequences and tempo asked about, as writing the
 *  source out is not free either: every cue of a lane asks in one render. */
const lastLayout: {
  cues?: Cue[];
  sequences?: Sequence[];
  tempo?: TempoSection;
  tempoKey?: string;
  lane?: LayoutLane;
  layout?: ShowSequences;
} = {};

/** The lane asking for a layout: the file it is in (the song's, say), and
 *  the show or sequence it draws. */
export interface LayoutLane {
  file: string;
  kind: "show" | "sequence";
  name: string;
}

const NO_LANE: LayoutLane = { file: "", kind: "show", name: "" };

function sameLane(a: LayoutLane | undefined, b: LayoutLane): boolean {
  return !!a && a.file === b.file && a.kind === b.kind && a.name === b.name;
}

/** The last layout made without its tempo, its map failing: with the
 *  source and the module it was made for. */
const lastFailed: {
  sourceKey?: string;
  generation?: number;
  layout?: ShowSequences;
} = {};

/** The last layout that parsed of each lane of one file (`lastGoodFile`):
 *  drawn while the file does not (mid-edit). Another file starts afresh. */
const lastGood = new Map<string, ShowSequences>();
let lastGoodFile: string | undefined;

/** The layout of the sequences `cues` play: one parse per distinct source
 *  and tempo, however many ask. `lane` names the lane asking, so that while
 *  its cues do not parse its last good layout is drawn. */
export function showSequences(
  cues: Cue[],
  sequences: Sequence[],
  tempo?: TempoSection,
  lane: LayoutLane = NO_LANE,
): ShowSequences {
  const key = tempo ? JSON.stringify(tempoKey(tempo)) : "-";
  if (
    lastLayout.layout &&
    lastLayout.cues === cues &&
    lastLayout.sequences === sequences &&
    lastLayout.tempo === tempo &&
    lastLayout.tempoKey === key &&
    sameLane(lastLayout.lane, lane)
  ) {
    return lastLayout.layout;
  }
  const source = serializeLightFile({
    sequences,
    shows: [{ name: "layout", cues }],
  });
  const sourceKey = `${key}\n${source}`;
  let mapFailed = false;
  let layout = layoutCache.get(sourceKey);
  // Laid out without its tempo (its map failed) a moment ago: not tried
  // again until the source, the tempo or the module changes.
  if (
    !layout &&
    lastFailed.sourceKey === sourceKey &&
    lastFailed.generation === tempoModuleGeneration()
  ) {
    layout = lastFailed.layout;
    mapFailed = true;
  }
  if (!layout) {
    const laid = layoutSequences(
      source,
      withMap(
        tempo,
        (map) => map,
        () => undefined,
        () => {
          mapFailed = true;
          return undefined;
        },
      ),
    );
    const show = laid.shows[0];
    layout = {
      calls: new Map(),
      cueErrors: new Map(
        (show?.cue_errors ?? []).map(([cue, e]) => [cue, withoutPlace(e)]),
      ),
      error: laid.error && withoutPlace(laid.error),
      problem: laid.problem && withoutPlace(laid.problem),
    };
    for (const call of show?.calls ?? []) {
      const list = layout.calls.get(call.cue) ?? [];
      list.push({ ...call, error: call.error && withoutPlace(call.error) });
      layout.calls.set(call.cue, list);
    }
    // Laid out without the tempo (its map failed), it is not the tempo's.
    if (!mapFailed) {
      if (layoutCache.size > 64) layoutCache.clear();
      layoutCache.set(sourceKey, layout);
    } else {
      Object.assign(lastFailed, {
        sourceKey,
        generation: tempoModuleGeneration(),
        layout,
      });
    }
  }
  if (lane.file !== lastGoodFile) {
    lastGood.clear();
    lastGoodFile = lane.file;
  }
  const laneKey = `${lane.kind}\u0000${lane.name}`;
  if (layout.error) {
    // The calls drawn are the last that parsed; what is wrong is this
    // file's, not that one's.
    const good = lastGood.get(laneKey);
    if (good) {
      layout = { calls: good.calls, cueErrors: new Map(), error: layout.error };
    }
  } else if (!mapFailed) {
    lastGood.set(laneKey, layout);
  }
  // Laid out without the tempo (its map failed), it is not the tempo's.
  if (mapFailed) return layout;
  Object.assign(lastLayout, {
    cues,
    sequences,
    tempo,
    tempoKey: key,
    lane,
    layout,
  });
  return layout;
}

/** A laid-out effect as the timeline draws it: its parameters as the player
 *  has them, its length as it plays there. */
function drawnEffect(
  call: LaidOutCall,
  [index, secs]: [number, number],
): CueEffect {
  const e = call.effects[index];
  return {
    groups: e.groups,
    effect: {
      type: e.type as EffectType,
      colors: e.colors,
      layer: e.layer ?? undefined,
      blend_mode: (e.blend_mode ?? undefined) as BlendMode | undefined,
      duration: `${Math.round(secs * 1000)}ms`,
      extra: {},
    },
    sequenceName: call.name,
  };
}

/**
 * Expand sequence references in a show's cues into concrete effect cues, as
 * the player expands them. Returns a new cue array with sequence effects
 * inserted at the correct timestamps, each marked with `sequenceName`. The
 * original cues are preserved unchanged; expanded cues are added alongside
 * them, at their places in the tempo map before any offset (the offsets are
 * the adjusted positions' to add). A call the player cannot play adds
 * nothing (see `showSequences` for why).
 */
export function expandSequencesIntoCues(
  showCues: Cue[],
  sequences: Sequence[],
  tempo?: TempoSection,
  lane: LayoutLane = NO_LANE,
): Cue[] {
  const expanded: Cue[] = [...showCues];
  for (const calls of showSequences(
    showCues,
    sequences,
    tempo,
    lane,
  ).calls.values()) {
    for (const call of calls) {
      for (const cue of call.cues) {
        if (cue.effects.length === 0) continue;
        expanded.push({
          timestamp: absoluteTimestamp((cue.time - call.offset) * 1000),
          effects: cue.effects.map((e) => drawnEffect(call, e)),
          commands: [],
          sequences: [],
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

/** How a cue's first sequence plays, as the player plays it. */
export interface SequencePlay {
  name: string;
  /** Its first pass */
  passMs: number;
  /** All its passes (as many as were laid out) */
  totalMs: number;
  /** Whether the cap on cues drawn stopped short of them all */
  truncated: boolean;
  /** How many of its cues land past a pass's `length:` and do not play */
  dropped: number;
  /** Why this call does not play, when it does not */
  error?: string;
}

/** How the first sequence cue `index` of `cues` plays: from the layout of
 *  them all (one parse for every cue that asks). A cue the player cannot
 *  read says why; a file that does not parse at all is the show's to say,
 *  not each cue's. */
export function sequencePlay(
  cues: Cue[],
  sequences: Sequence[],
  tempo: TempoSection | undefined,
  index: number,
  lane: LayoutLane = NO_LANE,
): SequencePlay | undefined {
  const layout = showSequences(cues, sequences, tempo, lane);
  const call = layout.calls.get(index)?.[0];
  if (!call) {
    const ref = cues[index]?.sequences.find((s) => !s.stop);
    const cueError = layout.cueErrors.get(index);
    return ref && cueError
      ? {
          name: ref.name,
          passMs: 0,
          totalMs: 0,
          truncated: false,
          dropped: 0,
          error: cueError,
        }
      : undefined;
  }
  const ms = (secs: number) => Math.max(0, (secs - call.start) * 1000);
  return {
    name: call.name,
    passMs: ms(call.pass_ends[0] ?? call.start),
    totalMs: ms(call.end),
    truncated: call.truncated,
    dropped: call.dropped,
    error: call.error,
  };
}

/** Get the BPM at a given ms position (mid-ramp, the ramp's tempo there). */
export function getTempoAtMs(ms: number, tempo?: TempoSection): number {
  return withMap(
    tempo,
    (map) => map.bpmAt(ms / 1000),
    () => DEFAULT_BPM,
  );
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
