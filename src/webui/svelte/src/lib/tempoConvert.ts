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

// Conversions between the song.yaml `tempo:` config (measure-anchored) and
// the lighting editor's TempoSection model (measure- or time-anchored).

import type { TempoConfig, TempoChangeConfig } from "./api/songs";
import type { TempoSection, TempoChange } from "./lighting/types";

export function parseTimeSignature(raw: string | undefined): [number, number] {
  const match = /^\s*(\d+)\s*\/\s*(\d+)\s*$/.exec(raw ?? "");
  if (!match) return [4, 4];
  return [parseInt(match[1]), parseInt(match[2])];
}

/** song.yaml tempo config -> the lighting TempoEditor's data model. */
export function configToSection(config: TempoConfig): TempoSection {
  return {
    start: { value: config.start ?? 0, unit: "s" },
    bpm: config.bpm,
    time_signature: parseTimeSignature(config.time_signature),
    changes: (config.changes ?? []).map((c) => {
      const change: TempoChange = {
        timestamp: {
          type: "measure_beat",
          measure: c.measure,
          beat: c.beat ?? 1,
        },
        bpm: c.bpm,
        time_signature: c.time_signature
          ? parseTimeSignature(c.time_signature)
          : undefined,
      };
      if (c.transition) {
        change.transition =
          "measures" in c.transition
            ? `${c.transition.measures}m`
            : `${c.transition.beats}`;
      }
      return change;
    }),
  };
}

export function parseTransition(
  raw: string | undefined,
): TempoChangeConfig["transition"] {
  if (!raw) return undefined;
  const trimmed = raw.trim();
  if (trimmed === "" || trimmed === "snap") return undefined;
  const measures = /^(\d+(?:\.\d+)?)\s*m$/.exec(trimmed);
  if (measures) return { measures: parseFloat(measures[1]) };
  const beats = /^(\d+(?:\.\d+)?)$/.exec(trimmed);
  if (beats) return { beats: parseFloat(beats[1]) };
  return undefined;
}

interface BeatGrid {
  beats: number[];
  measure_starts: number[];
}

/** Nearest measure/beat for a time in seconds, or null without a grid. */
function timeToMeasureBeat(
  grid: BeatGrid | null | undefined,
  secs: number,
): { measure: number; beat: number } | null {
  if (!grid || grid.beats.length === 0 || grid.measure_starts.length === 0) {
    return null;
  }
  let nearest = 0;
  for (let i = 1; i < grid.beats.length; i++) {
    if (Math.abs(grid.beats[i] - secs) < Math.abs(grid.beats[nearest] - secs)) {
      nearest = i;
    }
  }
  let measure = 0;
  while (
    measure + 1 < grid.measure_starts.length &&
    grid.measure_starts[measure + 1] <= nearest
  ) {
    measure++;
  }
  return {
    measure: measure + 1,
    beat: nearest - grid.measure_starts[measure] + 1,
  };
}

export interface SnappedImport {
  config: TempoConfig;
  /** Time-anchored changes snapped to the nearest measure/beat. */
  snapped: number;
  /** Time-anchored changes dropped for lack of a beat grid. */
  dropped: number;
}

/** The lighting TempoEditor's data model -> song.yaml tempo config.
 * Measure-anchored changes convert directly; time-anchored ones snap to
 * the nearest measure/beat on the grid (the song format is measure-based). */
export function sectionToConfigSnapped(
  section: TempoSection,
  beatGrid: BeatGrid | null | undefined,
): SnappedImport {
  const startSecs =
    section.start.unit === "ms"
      ? section.start.value / 1000
      : section.start.value;
  const config: TempoConfig = {
    bpm: section.bpm,
    time_signature: `${section.time_signature[0]}/${section.time_signature[1]}`,
  };
  if (startSecs > 0) {
    config.start = startSecs;
  }

  let snapped = 0;
  let dropped = 0;
  const changes: TempoChangeConfig[] = [];
  for (const c of section.changes) {
    let measure: number;
    let beat: number;
    if (c.timestamp.type === "measure_beat") {
      measure = c.timestamp.measure ?? 1;
      beat = c.timestamp.beat ?? 1;
    } else {
      const at = timeToMeasureBeat(beatGrid, (c.timestamp.ms ?? 0) / 1000);
      if (!at) {
        dropped++;
        continue;
      }
      measure = at.measure;
      beat = at.beat;
      snapped++;
    }
    const change: TempoChangeConfig = { measure };
    if (beat !== 1) {
      change.beat = beat;
    }
    if (c.bpm !== undefined) {
      change.bpm = c.bpm;
    }
    if (c.time_signature) {
      change.time_signature = `${c.time_signature[0]}/${c.time_signature[1]}`;
    }
    const transition = parseTransition(c.transition);
    if (transition) {
      change.transition = transition;
    }
    changes.push(change);
  }
  if (changes.length > 0) {
    config.changes = changes;
  }
  return { config, snapped, dropped };
}
