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

/**
 * The 3D view's Preview mode (lighting UI design, section 12.1): a show
 * evaluated offline at a moment, in the shape the live `state` message has,
 * so the scene is fed the same way from either source.
 */

import { post } from "../api/rest";
import {
  BadAnswerError,
  arr,
  hasAny,
  isObj,
  num,
  obj,
  readJson,
  str,
  type Obj,
} from "./answer";
import type { SongSummary } from "../api/songs";
import { timeAtPosition } from "../util/beatGrid";
import { sectionColor } from "../sectionColors";
import type {
  CellChannels,
  FixtureChannels,
  FixtureMetadata,
  FixturePose,
} from "../ws/stores";

/** An effect running at the previewed moment. */
export interface PreviewEffect {
  id: string;
  /** The logical groups the cue named; null when the engine did not say. */
  groups: string[] | null;
  /** The effect's kind as the engine names it: `Static`, `Move`, ... */
  kind: string;
  layer: string;
  /** Seconds into the effect. */
  elapsed: number;
  /** The effect's authored length, seconds. */
  duration: number;
  /** The fixtures the groups resolved to. */
  fixtures: string[];
}

/** What the scene is fed: the live state message's three maps. */
export interface PreviewFrame {
  fixtures: Record<string, FixtureChannels>;
  poses: Record<string, FixturePose>;
  cells: CellChannels;
}

export interface PreviewEvaluation extends PreviewFrame {
  time: number;
  active_effects: PreviewEffect[];
}

export interface PreviewResult {
  song: string;
  evaluations: PreviewEvaluation[];
  /** Fixtures no cue in the show targets. */
  untouched: string[];
}

/** Guards `POST /api/lighting/evaluate`'s answer: arrays default to `[]`,
 *  the three state maps to `{}`. Null when the body is not an object with any
 *  of the expected fields. */
export function parsePreview(body: unknown): PreviewResult | null {
  if (!hasAny(body, ["evaluations", "untouched", "song"])) return null;
  return {
    song: str(body.song),
    evaluations: arr<Obj>(body.evaluations)
      .filter(isObj)
      .map((e) => ({
        time: num(e.time),
        fixtures: obj(e.fixtures) as unknown as PreviewFrame["fixtures"],
        poses: obj(e.poses) as unknown as PreviewFrame["poses"],
        cells: obj(e.cells) as unknown as PreviewFrame["cells"],
        active_effects: arr<PreviewEffect>(e.active_effects).filter(isObj),
      })),
    untouched: arr<string>(body.untouched),
  };
}

/** Evaluates a song's shows at the given times (seconds). Throws with the
 *  server's message when the song is unknown or its lighting does not load. */
export async function evaluatePreview(
  song: string,
  times: number[],
): Promise<PreviewResult> {
  const res = await post("/lighting/evaluate", JSON.stringify({ song, times }));
  if (!res.ok) {
    let message = `Failed to evaluate: ${res.status}`;
    try {
      const body = await res.json();
      if (typeof body?.error === "string") message = body.error;
    } catch {
      // Keep the status message.
    }
    throw new Error(message);
  }
  const result = parsePreview(await readJson(res, "the preview"));
  if (!result) throw new BadAnswerError("the preview");
  return result;
}

/** "1:05" for a time in seconds; tenths when asked, for the scrubber. */
export function clock(seconds: number, tenths = false): string {
  const whole = Math.max(0, seconds);
  const minutes = Math.floor(whole / 60);
  const rest = whole - minutes * 60;
  return tenths
    ? `${minutes}:${rest.toFixed(1).padStart(4, "0")}`
    : `${minutes}:${Math.floor(rest).toString().padStart(2, "0")}`;
}

export interface SectionSegment {
  name: string;
  /** Percent of the song's length. */
  start: number;
  width: number;
  color: string;
}

/** The song's sections as strip segments, from the beat grid the same way
 *  the dashboard's timeline places them. Empty without a grid. */
export function sectionSegments(song: SongSummary): SectionSegment[] {
  const grid = song.beat_grid;
  const total = song.duration_ms / 1000;
  if (!grid || total <= 0) return [];
  const at = (measure: number, beat: number | null | undefined) =>
    timeAtPosition(grid, measure, beat ?? 1) ?? total;
  return song.sections.map((s, i) => {
    const from = Math.min(total, at(s.start_measure, s.start_beat));
    const to = Math.min(total, at(s.end_measure, s.end_beat));
    return {
      name: s.name,
      start: (from / total) * 100,
      width: (Math.max(from, to) / total) * 100 - (from / total) * 100,
      color: sectionColor(s.color ?? undefined, i),
    };
  });
}

/** One group's effects at the moment, for the "at this moment" list. */
export interface GroupActivity {
  group: string;
  effects: {
    kind: string;
    elapsed: number;
    duration: number;
    percent: number;
  }[];
}

/** The active effects gathered under the group each one's cue named (an
 *  effect naming several groups appears under each), groups sorted. */
export function groupActivity(effects: PreviewEffect[]): GroupActivity[] {
  const byGroup = new Map<string, GroupActivity["effects"]>();
  for (const effect of effects) {
    const groups = effect.groups?.length
      ? effect.groups
      : [effect.fixtures.join(", ")];
    const percent =
      effect.duration > 0
        ? Math.min(100, Math.round((effect.elapsed / effect.duration) * 100))
        : 100;
    for (const group of groups) {
      const list = byGroup.get(group) ?? [];
      list.push({
        kind: effect.kind,
        elapsed: effect.elapsed,
        duration: effect.duration,
        percent,
      });
      byGroup.set(group, list);
    }
  }
  return [...byGroup.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([group, list]) => ({ group, effects: list }));
}

/** How many fixtures take their colour from a wheel the scene does not
 *  model (it draws them white). */
export function wheelFixtureCount(
  metadata: Record<string, FixtureMetadata>,
): number {
  return Object.values(metadata).filter((m) => {
    const caps = m.capabilities ?? [];
    return caps.includes("color_wheel") && !caps.includes("color");
  }).length;
}

/** How many beams do not land on the deck (pointing up, level, or beyond
 *  the throw): the scene draws those at a fixed length. */
export function missingBeamCount(poses: Record<string, FixturePose>): number {
  return Object.values(poses).filter((p) => p.floor == null).length;
}

/** The song's lighting editor at a time, in seconds. */
export function timelineLink(song: string, seconds: number): string {
  return `#/songs/${encodeURIComponent(song)}/lighting?t=${
    Math.round(seconds * 1000) / 1000
  }`;
}
