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

// The timeline's drawing helpers. Where a cue lands, how long an effect
// runs, where the bars and beats are and what the tempo lane says all come
// from the player's own layout of the file (`rules.layout`) and its tempo
// map (`rules.tempo_query`); what is here turns
// those answers into pixels, labels and the snap steps the toolbar offers.

import type {
  GridLine,
  Layout,
  TempoSource,
  Timestamp,
  TempoSection,
} from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

export type { GridLine };

/** The tempo map the timeline asks about: the file's own `tempo` block
 *  (read through the parser, as the player's writer writes it), else the
 *  song's. */
export function tempoSourceOf(
  tempo: TempoSection | undefined,
  songTempo: TempoSource | null,
): TempoSource | null {
  return tempo ? { dsl: rules.tempo_write(tempo) ?? "" } : songTempo;
}

/** A grid line as the ruler draws it. */
export interface RulerLine {
  ms: number;
  type: "measure" | "beat";
  label?: string;
}

/** The lines of a visible range, with the labels and beat lines the zoom
 *  has room for: a label every 1, 2, 4, 8 or 16 bars as the bars get
 *  narrower, beat lines only when they are 6px apart or more. */
export function getGridLines(
  grid: GridLine[],
  viewStartMs: number,
  viewEndMs: number,
  pixelsPerMs?: number,
): RulerLine[] {
  const lines: RulerLine[] = [];
  const bars = grid.filter((l) => l.kind === "measure");
  // The last bar of the grid has no next barline: it is as long as the
  // one before it.
  let measureDurationMs = Infinity;
  for (let b = 0; b < bars.length; b++) {
    const bar = bars[b];
    if (bar.ms > viewEndMs) break;
    const nextBarMs = bars[b + 1]?.ms;
    const beats = grid.filter(
      (l) => l.measure === bar.measure && l.kind === "beat" && l.ms >= bar.ms,
    );
    const quarters = beats.length + 1;
    if (nextBarMs !== undefined) measureDurationMs = nextBarMs - bar.ms;
    if (nextBarMs !== undefined && nextBarMs < viewStartMs) continue;

    const pxPerMeasure = pixelsPerMs
      ? pixelsPerMs * measureDurationMs
      : Infinity;
    let labelEvery = 1;
    if (pxPerMeasure < 15) labelEvery = 16;
    else if (pxPerMeasure < 25) labelEvery = 8;
    else if (pxPerMeasure < 40) labelEvery = 4;
    else if (pxPerMeasure < 60) labelEvery = 2;
    const showBeats = pxPerMeasure / quarters >= 6;

    if (bar.ms >= viewStartMs) {
      const showLabel = (bar.measure - 1) % labelEvery === 0;
      lines.push({
        ms: bar.ms,
        type: "measure",
        label: showLabel ? `${bar.measure}` : undefined,
      });
    }
    if (showBeats) {
      for (const beat of beats) {
        if (beat.ms > viewEndMs) break;
        if (beat.ms >= viewStartMs) lines.push({ ms: beat.ms, type: "beat" });
      }
    }
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

/** Where a timestamp is in the tempo map, in milliseconds: its place
 *  before any offset. A bar and beat with no tempo to place it is at 0. */
export function timestampToMs(
  ts: Timestamp,
  source: TempoSource | null,
): number {
  if (ts.type === "absolute") return ts.ms ?? 0;
  if (!source) return 0;
  return (
    rules.tempo_query(source, {
      measure: ts.measure ?? 1,
      beat: ts.beat ?? 1,
    })?.ms ?? 0
  );
}

/** The bar and beat at a time, on the nearest beat, for a cue placed by
 *  bar and beat; a time with no tempo to place it is kept as a time. */
export function msToTimestamp(
  ms: number,
  source: TempoSource | null,
): Timestamp {
  const at = source
    ? rules.tempo_query(source, { ms: snapToNearestGrid(ms, source, "beat") })
    : null;
  if (!at) return { type: "absolute", ms };
  return {
    type: "measure_beat",
    measure: at.measure,
    beat: Math.round(at.beat),
  };
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

/** The step of a beat each resolution snaps to; `measure` snaps to
 *  barlines. */
export const SNAP_BEATS: Record<SnapResolution, number | undefined> = {
  measure: undefined,
  beat: 1,
  "1/2": 0.5,
  "1/4": 0.25,
  "1/8": 0.125,
  "1/16": 0.0625,
};

/** The nearest barline, or the nearest step of a beat counted from the
 *  bar's downbeat (so an odd metre's bar keeps its subdivisions), by the
 *  tempo map's own positions. */
export function snapToNearestGrid(
  ms: number,
  source: TempoSource,
  resolution: SnapResolution,
): number {
  const at = rules.tempo_query(source, { ms });
  if (!at) return ms;
  if (resolution === "measure") {
    const here = rules.tempo_query(source, { measure: at.measure, beat: 1 });
    const next = rules.tempo_query(source, {
      measure: at.measure + 1,
      beat: 1,
    });
    if (!here) return ms;
    if (!next) return here.ms;
    return ms - here.ms <= next.ms - ms ? here.ms : next.ms;
  }
  const step = SNAP_BEATS[resolution]!;
  const beat = 1 + Math.round((at.beat - 1) / step) * step;
  return rules.tempo_query(source, { measure: at.measure, beat })?.ms ?? ms;
}

/** An offset marker on the ruler: a cue's `offset` or `reset_measures`,
 *  where the cue is drawn. */
export interface OffsetMarker {
  ms: number;
  type: "offset" | "reset";
  measures: number;
  /** Which show this offset belongs to */
  showIndex: number;
  /** Which cue within the show has this offset */
  cueIndex: number;
}

/** The offset and reset markers of every show, in time order. */
export function offsetMarkers(layout: Layout): OffsetMarker[] {
  const markers: OffsetMarker[] = [];
  layout.shows.forEach((show, showIndex) => {
    for (const cue of show.cues) {
      if (cue.reset) {
        markers.push({
          ms: cue.ms,
          type: "reset",
          measures: 0,
          showIndex,
          cueIndex: cue.index,
        });
      }
      if (cue.offset_measures > 0) {
        markers.push({
          ms: cue.ms,
          type: "offset",
          measures: cue.offset_measures,
          showIndex,
          cueIndex: cue.index,
        });
      }
    }
  });
  return markers.sort((a, b) => a.ms - b.ms);
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
