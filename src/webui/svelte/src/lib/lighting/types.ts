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

// The editor's model of a `.light` file is the player's own document model
// (`src/lighting/parser/document.rs`), read off the one grammar by
// `rules.document`, edited through `rules.effect_edit` and written back by
// the player's writer (`rules.document_write`); its types are generated
// from the Rust structs. What is here is the editor's own: the
// constructors its forms use (the option lists are `rules.vocabulary`).

import type {
  Cue,
  EffectKind,
  EffectParams,
  Timestamp,
} from "../../gen/rules/mtrack_rules";

export type {
  Cue,
  CueEffect,
  Document as LightFile,
  Param,
  DurationValue as Duration,
  EffectKind as EffectType,
  EffectParams,
  Include,
  LayerCommand,
  LightShow,
  Sequence,
  SequenceArg,
  SequenceRef,
  TempoChange,
  TempoSection,
  Timestamp,
} from "../../gen/rules/mtrack_rules";

/** Sub-lane content type for multi-lane show rendering */
export type SubLaneType =
  | "effects"
  | `effects:${string}`
  | "commands"
  | "sequences";

/** Creates an empty EffectParams of a given type */
export function emptyEffect(type: EffectKind): EffectParams {
  return { type, params: [] };
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
