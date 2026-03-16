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

import type {
  LightFile,
  Timestamp,
  TempoSection,
  Cue,
  CueEffect,
  LayerCommand,
  SequenceRef,
} from "./types";

/**
 * Serialize a LightFile structure back to DSL text.
 */
export function serializeLightFile(file: LightFile): string {
  const parts: string[] = [];

  // Tempo block
  if (file.tempo) {
    parts.push(serializeTempo(file.tempo));
    parts.push("");
  }

  // Sequences
  for (const seq of file.sequences) {
    parts.push(`sequence "${seq.name}" {`);
    parts.push(serializeCues(seq.cues));
    parts.push("}");
    parts.push("");
  }

  // Shows
  for (const show of file.shows) {
    parts.push(`show "${show.name}" {`);
    parts.push(serializeCues(show.cues));
    parts.push("}");
    parts.push("");
  }

  return parts.join("\n");
}

function serializeTempo(tempo: TempoSection): string {
  const lines: string[] = [];
  lines.push("tempo {");
  lines.push(`    start: ${tempo.start.value}${tempo.start.unit}`);
  lines.push(`    bpm: ${tempo.bpm}`);
  lines.push(
    `    time_signature: ${tempo.time_signature[0]}/${tempo.time_signature[1]}`,
  );

  if (tempo.changes.length > 0) {
    lines.push("    changes: [");
    for (const change of tempo.changes) {
      const ts = formatTimestamp(change.timestamp);
      const params: string[] = [];
      if (change.bpm !== undefined) params.push(`bpm: ${change.bpm}`);
      if (change.time_signature)
        params.push(
          `time_signature: ${change.time_signature[0]}/${change.time_signature[1]}`,
        );
      if (change.transition) params.push(`transition: ${change.transition}`);
      lines.push(`        @${ts} { ${params.join(", ")} },`);
    }
    lines.push("    ]");
  }

  lines.push("}");
  return lines.join("\n");
}

function serializeCues(cues: Cue[]): string {
  const lines: string[] = [];

  for (let i = 0; i < cues.length; i++) {
    const cue = cues[i];

    // Comment
    if (cue.comment) {
      for (const commentLine of cue.comment.split("\n")) {
        lines.push(`    # ${commentLine}`);
      }
    }

    // Timestamp
    lines.push(`    @${formatTimestamp(cue.timestamp)}`);

    // Effects
    for (const eff of cue.effects) {
      lines.push(`    ${serializeEffect(eff)}`);
    }

    // Layer commands
    for (const cmd of cue.commands) {
      lines.push(`    ${serializeLayerCommand(cmd)}`);
    }

    // Sequence references
    for (const ref of cue.sequences) {
      lines.push(`    ${serializeSequenceRef(ref)}`);
    }

    // Offset/reset measures
    if (cue.offset_measures !== undefined) {
      lines.push(`    offset ${cue.offset_measures} measures`);
    }
    if (cue.reset_measures) {
      lines.push(`    reset_measures`);
    }

    // Blank line between cues (except after last)
    if (i < cues.length - 1) {
      lines.push("");
    }
  }

  return lines.join("\n");
}

/**
 * Format a timestamp as a DSL string.
 */
export function formatTimestamp(ts: Timestamp): string {
  if (ts.type === "measure_beat") {
    const beat = ts.beat !== undefined ? ts.beat.toString() : "1";
    return `${ts.measure ?? 1}/${beat}`;
  }

  // Absolute: format as MM:SS.mmm
  const totalMs = ts.ms ?? 0;
  const mins = Math.floor(totalMs / 60000);
  const secs = Math.floor((totalMs % 60000) / 1000);
  const ms = totalMs % 1000;
  return `${mins.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}.${ms.toString().padStart(3, "0")}`;
}

function serializeEffect(cueEffect: CueEffect): string {
  const groups = cueEffect.groups.join(", ");
  const effect = cueEffect.effect;
  const parts: string[] = [];

  // Effect type
  // Dimmer uses space separator, others use comma
  const isDimmer = effect.type === "dimmer";
  const separator = isDimmer ? " " : ", ";

  // Colors
  for (const color of effect.colors) {
    parts.push(`color: "${color}"`);
  }

  // Known parameters
  if (effect.intensity !== undefined)
    parts.push(`intensity: ${effect.intensity}`);
  if (effect.dimmer !== undefined) parts.push(`dimmer: ${effect.dimmer}`);
  if (effect.frequency !== undefined)
    parts.push(`frequency: ${effect.frequency}`);
  if (effect.speed !== undefined) parts.push(`speed: ${effect.speed}`);
  if (effect.direction !== undefined)
    parts.push(`direction: ${effect.direction}`);
  if (effect.transition !== undefined)
    parts.push(`transition: ${effect.transition}`);
  if (effect.duration !== undefined) parts.push(`duration: ${effect.duration}`);
  if (effect.up_time !== undefined) parts.push(`up_time: ${effect.up_time}`);
  if (effect.hold_time !== undefined)
    parts.push(`hold_time: ${effect.hold_time}`);
  if (effect.down_time !== undefined)
    parts.push(`down_time: ${effect.down_time}`);
  if (effect.start_level !== undefined)
    parts.push(`start_level: ${effect.start_level}`);
  if (effect.end_level !== undefined)
    parts.push(`end_level: ${effect.end_level}`);
  if (effect.curve !== undefined) parts.push(`curve: ${effect.curve}`);
  if (effect.layer !== undefined) parts.push(`layer: ${effect.layer}`);
  if (effect.blend_mode !== undefined)
    parts.push(`blend_mode: ${effect.blend_mode}`);
  if (effect.loop !== undefined) parts.push(`loop: ${effect.loop}`);
  if (effect.base_level !== undefined)
    parts.push(`base_level: ${effect.base_level}`);
  if (effect.pulse_amplitude !== undefined)
    parts.push(`pulse_amplitude: ${effect.pulse_amplitude}`);
  if (effect.duty_cycle !== undefined)
    parts.push(`duty_cycle: ${effect.duty_cycle}`);
  if (effect.pattern !== undefined) parts.push(`pattern: ${effect.pattern}`);
  if (effect.saturation !== undefined)
    parts.push(`saturation: ${effect.saturation}`);
  if (effect.brightness !== undefined)
    parts.push(`brightness: ${effect.brightness}`);

  // Extra params
  for (const [k, v] of Object.entries(effect.extra)) {
    parts.push(`${k}: ${v}`);
  }

  if (parts.length > 0) {
    return `${groups}: ${effect.type}${separator}${parts.join(", ")}`;
  }
  return `${groups}: ${effect.type}`;
}

function serializeLayerCommand(cmd: LayerCommand): string {
  const params: string[] = [];
  if (cmd.layer) params.push(`layer: ${cmd.layer}`);
  if (cmd.time) params.push(`time: ${cmd.time}`);
  if (cmd.intensity) params.push(`intensity: ${cmd.intensity}`);
  if (cmd.speed) params.push(`speed: ${cmd.speed}`);
  return `${cmd.command}(${params.join(", ")})`;
}

function serializeSequenceRef(ref: SequenceRef): string {
  const parts: string[] = [];
  if (ref.stop) parts.push("stop ");
  parts.push(`sequence "${ref.name}"`);
  if (ref.loop && !ref.stop) parts.push(` loop: ${ref.loop}`);
  return parts.join("");
}
