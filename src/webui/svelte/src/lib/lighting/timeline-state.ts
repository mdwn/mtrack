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

import type { Timestamp, TempoSection, Cue } from "./types";

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
): GridLine[] {
  if (offsets.length === 0) {
    return getGridLines(tempo, viewStartMs, viewEndMs);
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

  const rawLines = getGridLines(tempo, rawStart, rawEnd);

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
  /** The cumulative offset in ms applied to this cue */
  cumulativeOffsetMs: number;
}

/** Convert N measures to ms at the tempo active at a given ms position. */
export function offsetMeasuresToMs(
  measures: number,
  atMs: number,
  tempo: TempoSection,
): number {
  const segments = buildTempoSegments(tempo);
  let seg = segments[0];
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].startMs <= atMs) {
      seg = segments[i];
      break;
    }
  }
  return measures * (60000 / seg.bpm) * seg.beatsPerMeasure;
}

/**
 * Compute adjusted cue positions for a show's cues, applying cumulative offsets.
 * Each cue's offset_measures shifts all subsequent cues forward in real time.
 */
export function computeAdjustedCuePositions(
  cues: Cue[],
  tempo?: TempoSection,
): AdjustedCuePosition[] {
  let cumulativeOffsetMs = 0;
  return cues.map((cue, index) => {
    const baseMs = timestampToMs(cue.timestamp, tempo);
    const adjustedMs = baseMs + cumulativeOffsetMs;
    const result: AdjustedCuePosition = {
      cue,
      index,
      adjustedMs,
      cumulativeOffsetMs,
    };
    if (cue.offset_measures !== undefined && cue.offset_measures > 0 && tempo) {
      cumulativeOffsetMs += offsetMeasuresToMs(
        cue.offset_measures,
        adjustedMs,
        tempo,
      );
    }
    return result;
  });
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
