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

import type { BeatGrid, TempoConfig, TempoChangeConfig } from "./api/songs";
import type { TempoSection, TempoChange } from "./lighting/types";
import { rules } from "./rules";

/** `[numerator, denominator]` of a `4/4`-style string, by the player's own
 * parser; 4/4 stands in for a string it refuses, since the editor's model
 * has to show something. */
function sigOf(raw: string | undefined): [number, number] {
  return rules.signature_parse(raw ?? "") ?? [4, 4];
}

/** song.yaml tempo config -> the lighting TempoEditor's data model. */
export function configToSection(config: TempoConfig): TempoSection {
  return {
    start: { value: config.start ?? 0, unit: "s" },
    bpm: config.bpm,
    time_signature: sigOf(config.time_signature),
    changes: (config.changes ?? []).map((c) => {
      const change: TempoChange = {
        timestamp: {
          type: "measure_beat",
          measure: c.measure,
          beat: c.beat ?? 1,
        },
        bpm: c.bpm,
        time_signature: c.time_signature ? sigOf(c.time_signature) : undefined,
      };
      if (c.transition) {
        change.transition = rules.transition_text(c.transition);
      }
      return change;
    }),
  };
}

/** A light show change's `transition:` as a song change's, as the
 *  player reads it (`rules.transition_read`); none for `snap`. */
export function transitionOf(
  written: string | undefined,
): TempoChangeConfig["transition"] {
  return written ? rules.transition_read(written) : undefined;
}

export interface SnappedImport {
  config: TempoConfig;
  /** Time-anchored changes snapped to the nearest measure/beat. */
  snapped: number;
  /** Time-anchored changes dropped for lack of a beat grid. */
  dropped: number;
  /** Changes dropped for landing on a position another change already held. */
  duplicates: number;
}

/** The lighting TempoEditor's data model -> song.yaml tempo config.
 * Measure-anchored changes convert directly; time-anchored ones snap to
 * the nearest measure/beat on the grid (the song format is measure-based).
 *
 * The result is ordered and free of repeated positions, because a light show
 * is under no obligation to be either: its changes are sorted at playback,
 * and two of them may share a position (or arrive at one by snapping). The
 * song format is stricter — `to_tempo_map` refuses both — so the result goes
 * through the player's `tempo_normalize` before it can reach a save. */
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
      const at = beatGrid
        ? rules.grid_query(beatGrid, {
            secs: (c.timestamp.ms ?? 0) / 1000,
            snap: { step: 1 },
          })
        : undefined;
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
    const transition = transitionOf(c.transition);
    if (transition) {
      change.transition = transition;
    }
    changes.push(change);
  }
  if (changes.length > 0) {
    config.changes = changes;
  }
  const ordered = rules.tempo_normalize(config);
  return {
    config: ordered?.config ?? config,
    snapped,
    dropped,
    duplicates: ordered?.duplicates ?? 0,
  };
}
