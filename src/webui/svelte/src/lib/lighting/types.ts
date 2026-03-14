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

/** Timestamp — either absolute (MM:SS.mmm) or measure/beat (@M/B) */
export interface Timestamp {
  type: "absolute" | "measure_beat";
  /** For absolute: total milliseconds */
  ms?: number;
  /** For measure/beat */
  measure?: number;
  beat?: number;
}

/** Duration with unit */
export interface Duration {
  value: number;
  unit: "ms" | "s" | "beats" | "measures";
}

/** Tempo definition */
export interface TempoSection {
  start: Duration;
  bpm: number;
  time_signature: [number, number];
  changes: TempoChange[];
}

/** Tempo change at a specific point */
export interface TempoChange {
  timestamp: Timestamp;
  bpm?: number;
  time_signature?: [number, number];
  transition?: string;
}

export type EffectType =
  | "static"
  | "cycle"
  | "strobe"
  | "pulse"
  | "chase"
  | "dimmer"
  | "rainbow";

export const EFFECT_TYPES: EffectType[] = [
  "static",
  "cycle",
  "strobe",
  "pulse",
  "chase",
  "dimmer",
  "rainbow",
];

export type Layer = "background" | "midground" | "foreground";
export const LAYERS: Layer[] = ["background", "midground", "foreground"];

export type BlendMode = "replace" | "multiply" | "add" | "overlay" | "screen";
export const BLEND_MODES: BlendMode[] = [
  "replace",
  "multiply",
  "add",
  "overlay",
  "screen",
];

export const CURVES = [
  "linear",
  "exponential",
  "logarithmic",
  "sine",
  "cosine",
];
export const DIRECTIONS = [
  "forward",
  "backward",
  "left_to_right",
  "right_to_left",
];

/** Effect parameters */
export interface EffectParams {
  type: EffectType;
  colors: string[];
  intensity?: number;
  dimmer?: string;
  frequency?: string;
  speed?: string;
  direction?: string;
  transition?: string;
  duration?: string;
  up_time?: string;
  hold_time?: string;
  down_time?: string;
  start_level?: number;
  end_level?: number;
  curve?: string;
  layer?: Layer;
  blend_mode?: BlendMode;
  loop?: string;
  base_level?: number;
  pulse_amplitude?: number;
  duty_cycle?: string;
  pattern?: string;
  saturation?: number;
  brightness?: number;
  extra: Record<string, string>;
}

/** A single effect targeting one or more groups */
export interface CueEffect {
  groups: string[];
  effect: EffectParams;
}

/** Layer command (clear/release/freeze/unfreeze/master) */
export interface LayerCommand {
  command: "clear" | "release" | "freeze" | "unfreeze" | "master";
  layer?: string;
  time?: string;
  intensity?: string;
  speed?: string;
}

/** Reference to a named sequence */
export interface SequenceRef {
  name: string;
  loop?: string;
  stop?: boolean;
}

/** A cue = one timestamp + all effects/commands at that time */
export interface Cue {
  timestamp: Timestamp;
  effects: CueEffect[];
  commands: LayerCommand[];
  sequences: SequenceRef[];
  comment?: string;
}

/** A named sequence of cues */
export interface Sequence {
  name: string;
  cues: Cue[];
}

/** A named show of cues */
export interface LightShow {
  name: string;
  cues: Cue[];
}

/** Top-level parsed light file */
export interface LightFile {
  tempo?: TempoSection;
  sequences: Sequence[];
  shows: LightShow[];
}

/** Creates an empty EffectParams of a given type */
export function emptyEffect(type: EffectType): EffectParams {
  return { type, colors: [], extra: {} };
}

/** Creates an empty cue at a given timestamp */
export function emptyCue(timestamp: Timestamp): Cue {
  return { timestamp, effects: [], commands: [], sequences: [] };
}

/** Creates an absolute timestamp from MM:SS.mmm */
export function absoluteTimestamp(ms: number): Timestamp {
  return { type: "absolute", ms };
}

/** Creates a measure/beat timestamp */
export function measureBeatTimestamp(measure: number, beat: number): Timestamp {
  return { type: "measure_beat", measure, beat };
}
