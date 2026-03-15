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

import type { Timestamp, TempoSection } from "./types";

/** A segment of constant tempo, used for measure/beat <-> ms conversion. */
export interface TempoSegment {
  /** Absolute ms where this segment starts */
  startMs: number;
  /** Measure number at the start of this segment (1-based) */
  startMeasure: number;
  /** Beat within the start measure (1-based) */
  startBeat: number;
  bpm: number;
  beatsPerMeasure: number;
  beatValue: number;
}

/** Grid line for rendering on the ruler. */
export interface GridLine {
  ms: number;
  type: "measure" | "beat";
  label?: string;
}

/** Build a sorted array of tempo segments from a TempoSection.
 *  Uses only already-built segments to resolve change timestamps,
 *  avoiding mutual recursion with timestampToMs. */
export function buildTempoSegments(tempo: TempoSection): TempoSegment[] {
  const segments: TempoSegment[] = [];
  const [beatsPerMeasure, beatValue] = tempo.time_signature;

  // First segment starts at the tempo start offset
  const startMs = durationToMs(tempo.start);
  segments.push({
    startMs,
    startMeasure: 1,
    startBeat: 1,
    bpm: tempo.bpm,
    beatsPerMeasure,
    beatValue,
  });

  for (const change of tempo.changes) {
    const prev = segments[segments.length - 1];
    // Resolve the change timestamp using segments built so far — NOT
    // timestampToMs(), which would call buildTempoSegments() and recurse.
    const changeMs = resolveTimestampWithSegments(change.timestamp, segments);
    const elapsed = changeMs - prev.startMs;
    const beatDurationMs = 60000 / prev.bpm;
    const beatsElapsed = elapsed / beatDurationMs;

    // Calculate measure/beat at the change point
    const totalBeats =
      (prev.startMeasure - 1) * prev.beatsPerMeasure +
      (prev.startBeat - 1) +
      beatsElapsed;
    const newBpm = change.bpm ?? prev.bpm;
    const newTimeSig = change.time_signature ?? [
      prev.beatsPerMeasure,
      prev.beatValue,
    ];
    const newBeatsPerMeasure = newTimeSig[0];
    const measure = Math.floor(totalBeats / newBeatsPerMeasure) + 1;
    const beat = (totalBeats % newBeatsPerMeasure) + 1;

    segments.push({
      startMs: changeMs,
      startMeasure: measure,
      startBeat: beat,
      bpm: newBpm,
      beatsPerMeasure: newBeatsPerMeasure,
      beatValue: newTimeSig[1],
    });
  }

  return segments;
}

/** Convert a timestamp to ms using an already-built segment list.
 *  This avoids the mutual recursion between buildTempoSegments and timestampToMs. */
function resolveTimestampWithSegments(
  ts: Timestamp,
  segments: TempoSegment[],
): number {
  if (ts.type === "absolute") {
    return ts.ms ?? 0;
  }
  // measure_beat: walk the segments built so far
  const targetMeasure = ts.measure ?? 1;
  const targetBeat = ts.beat ?? 1;

  let seg = segments[0];
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].startMeasure <= targetMeasure) {
      seg = segments[i];
      break;
    }
  }

  const beatDurationMs = 60000 / seg.bpm;
  const beatsFromSegStart =
    (targetMeasure - seg.startMeasure) * seg.beatsPerMeasure +
    (targetBeat - seg.startBeat);

  return seg.startMs + beatsFromSegStart * beatDurationMs;
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

/** Convert a Timestamp to absolute milliseconds. */
export function timestampToMs(ts: Timestamp, tempo?: TempoSection): number {
  if (ts.type === "absolute") {
    return ts.ms ?? 0;
  }

  // measure_beat type
  if (!tempo) return 0;

  const segments = buildTempoSegments(tempo);
  const targetMeasure = ts.measure ?? 1;
  const targetBeat = ts.beat ?? 1;

  // Find the segment that contains this measure/beat
  let seg = segments[0];
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].startMeasure <= targetMeasure) {
      seg = segments[i];
      break;
    }
  }

  const beatDurationMs = 60000 / seg.bpm;
  const beatsFromSegStart =
    (targetMeasure - seg.startMeasure) * seg.beatsPerMeasure +
    (targetBeat - seg.startBeat);

  return seg.startMs + beatsFromSegStart * beatDurationMs;
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

  const segments = buildTempoSegments(tempo);

  // Find the segment containing this ms
  let seg = segments[0];
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].startMs <= ms) {
      seg = segments[i];
      break;
    }
  }

  const beatDurationMs = 60000 / seg.bpm;
  const beatsFromSegStart = (ms - seg.startMs) / beatDurationMs;
  const totalBeatsFromStart =
    (seg.startMeasure - 1) * seg.beatsPerMeasure +
    (seg.startBeat - 1) +
    beatsFromSegStart;

  const measure = Math.floor(totalBeatsFromStart / seg.beatsPerMeasure) + 1;
  const beat = Math.round((totalBeatsFromStart % seg.beatsPerMeasure) + 1);

  return { type: "measure_beat", measure, beat };
}

/** Generate grid lines (measures and beats) for a visible time range. */
export function getGridLines(
  tempo: TempoSection,
  viewStartMs: number,
  viewEndMs: number,
): GridLine[] {
  const lines: GridLine[] = [];
  const segments = buildTempoSegments(tempo);

  for (let si = 0; si < segments.length; si++) {
    const seg = segments[si];
    const segEnd =
      si < segments.length - 1 ? segments[si + 1].startMs : viewEndMs + 10000;
    const beatDurationMs = 60000 / seg.bpm;

    // Walk beats from this segment's start
    let beatIndex = 0;
    // If segment starts before our view, skip ahead
    if (seg.startMs < viewStartMs) {
      beatIndex = Math.floor((viewStartMs - seg.startMs) / beatDurationMs);
    }

    while (true) {
      const ms = seg.startMs + beatIndex * beatDurationMs;
      if (ms > viewEndMs || ms >= segEnd) break;

      if (ms >= viewStartMs) {
        const beatInMeasure = beatIndex % seg.beatsPerMeasure;
        const measureNum =
          seg.startMeasure +
          Math.floor((beatIndex + (seg.startBeat - 1)) / seg.beatsPerMeasure);

        if (beatInMeasure === 0) {
          lines.push({ ms, type: "measure", label: `${measureNum}` });
        } else {
          lines.push({ ms, type: "beat" });
        }
      }
      beatIndex++;
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

/** Pixel <-> time conversion helpers */
export function msToPixel(ms: number, pixelsPerMs: number): number {
  return ms * pixelsPerMs;
}

export function pixelToMs(px: number, pixelsPerMs: number): number {
  return px / pixelsPerMs;
}

/** Snap a time to the nearest grid line */
export function snapToNearestGrid(
  ms: number,
  tempo: TempoSection,
  resolution: "beat" | "measure",
): number {
  const segments = buildTempoSegments(tempo);

  // Find containing segment
  let seg = segments[0];
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].startMs <= ms) {
      seg = segments[i];
      break;
    }
  }

  const beatDurationMs = 60000 / seg.bpm;
  const step =
    resolution === "measure"
      ? beatDurationMs * seg.beatsPerMeasure
      : beatDurationMs;

  const elapsed = ms - seg.startMs;
  const snapped = Math.round(elapsed / step) * step;
  return seg.startMs + snapped;
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
    default:
      return "#6b7280";
  }
}
