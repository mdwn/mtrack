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
  SequenceArg,
} from "./types";
import type { TempoField } from "../../gen/rules/mtrack_rules";

/**
 * Serialize a LightFile structure back to DSL text: what the document
 * holds, as written (`rules.document` reads the text back to the same
 * document).
 */
export function serializeLightFile(file: LightFile): string {
  const parts: string[] = [];

  if (file.includes?.length) {
    for (const inc of file.includes) {
      parts.push(...own(inc.comments_before, ""));
      parts.push(line(`include "${inc.written}"`, inc.comment));
    }
    parts.push("");
  }

  // Tempo block
  if (file.tempo) {
    parts.push(serializeTempo(file.tempo));
    parts.push("");
  }

  // Sequences
  for (const seq of file.sequences) {
    const params = seq.params ? serializeArgList(seq.params) : "";
    const length = seq.length ? `, length: ${seq.length}` : "";
    parts.push(...own(seq.comments_before, ""));
    parts.push(line(`sequence "${seq.name}"${params}${length} {`, seq.comment));
    parts.push(serializeCues(seq.cues));
    parts.push(...own(seq.comments_after, "    "));
    parts.push(line("}", seq.close_comment));
    parts.push("");
  }

  // Shows
  for (const show of file.shows) {
    parts.push(...own(show.comments_before, ""));
    parts.push(line(`show "${show.name}" {`, show.comment));
    parts.push(serializeCues(show.cues));
    parts.push(...own(show.comments_after, "    "));
    parts.push(line("}", show.close_comment));
    parts.push("");
  }

  parts.push(...own(file.comments_after, ""));
  if (file.comments_after?.length) parts.push("");

  return parts.join("\n");
}

/** A line with the comment that was on it, if one was. */
function line(text: string, comment: string | undefined): string {
  return comment ? `${text} ${comment}` : text;
}

/** Comment lines of their own, as written, at an indent. */
function own(comments: string[] | undefined, indent: string): string[] {
  return (comments ?? []).map((c) => `${indent}${c}`);
}

/** The `tempo { }` block alone: what `rules.tempo_query` reads a section
 *  through (`{ dsl }`), so the editor's tempo questions go to the parser's
 *  own reading of the block it will write. */
export function serializeTempo(tempo: TempoSection): string {
  const lines: string[] = [];
  const at = (field: TempoField, inline: boolean) =>
    (tempo.comments ?? [])
      .filter((c) => c.at === field && c.inline === inline)
      .map((c) => c.text);
  const field = (name: TempoField, text: string) => {
    lines.push(...own(at(name, false), "    "));
    lines.push(line(`    ${text}`, at(name, true).join(" ") || undefined));
  };
  lines.push(...own(tempo.comments_before, ""));
  lines.push(line("tempo {", tempo.comment));
  field("start", `start: ${tempo.start.value}${tempo.start.unit}`);
  field("bpm", `bpm: ${tempo.bpm}`);
  field(
    "time_signature",
    `time_signature: ${tempo.time_signature[0]}/${tempo.time_signature[1]}`,
  );

  if (tempo.changes.length > 0 || tempo.changes_comments_after?.length) {
    field("changes", "changes: [");
    tempo.changes.forEach((change, i) => {
      const ts = formatTimestamp(change.timestamp);
      const params: string[] = [];
      if (change.bpm !== undefined) params.push(`bpm: ${change.bpm}`);
      if (change.time_signature)
        params.push(
          `time_signature: ${change.time_signature[0]}/${change.time_signature[1]}`,
        );
      if (change.transition) params.push(`transition: ${change.transition}`);
      const comma = i < tempo.changes.length - 1 ? "," : "";
      lines.push(...own(change.comments_before, "        "));
      lines.push(
        line(`        @${ts} { ${params.join(", ")} }${comma}`, change.comment),
      );
    });
    lines.push(...own(tempo.changes_comments_after, "        "));
    lines.push(line("    ]", at("end", true).join(" ") || undefined));
  }
  lines.push(...own(at("end", false), "    "));

  lines.push(line("}", tempo.close_comment));
  return lines.join("\n");
}

function serializeCues(cues: Cue[]): string {
  const lines: string[] = [];

  for (let i = 0; i < cues.length; i++) {
    const cue = cues[i];

    lines.push(...own(cue.comments_before, "    "));
    lines.push(line(`    @${formatTimestamp(cue.timestamp)}`, cue.comment));

    // Effects
    for (const eff of cue.effects) {
      const s = serializeEffect(eff);
      if (!s) continue;
      lines.push(...own(eff.comments_before, "    "));
      lines.push(line(`    ${s}`, eff.comment));
    }

    // Layer commands
    for (const cmd of cue.commands) {
      lines.push(...own(cmd.comments_before, "    "));
      lines.push(line(`    ${serializeLayerCommand(cmd)}`, cmd.comment));
    }

    // Sequence references
    for (const ref of cue.sequences) {
      lines.push(...own(ref.comments_before, "    "));
      lines.push(line(`    ${serializeSequenceRef(ref)}`, ref.comment));
    }

    // Offset/reset measures
    if (cue.offset_measures !== undefined) {
      lines.push(...own(cue.offset_comments_before, "    "));
      lines.push(
        line(`    offset ${cue.offset_measures} measures`, cue.offset_comment),
      );
    }
    if (cue.reset_measures) {
      lines.push(...own(cue.reset_comments_before, "    "));
      lines.push(line(`    reset_measures`, cue.reset_comment));
    }

    // Inline loops, kept as the text they were read from.
    for (const loop of cue.loops ?? []) {
      lines.push(...own(loop.comments_before, "    "));
      lines.push(line(`    ${loop.text}`, loop.comment));
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

  // Absolute: format as MM:SS.mmm, in whole milliseconds (the document
  // holds whole ones; an edit's fraction is rounded away).
  const totalMs = Math.max(0, Math.round(ts.ms ?? 0));
  const mins = Math.floor(totalMs / 60000);
  const secs = Math.floor((totalMs % 60000) / 1000);
  const ms = totalMs % 1000;
  return `${mins.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}.${ms.toString().padStart(3, "0")}`;
}

function serializeEffect(cueEffect: CueEffect): string {
  const groups = cueEffect.groups.filter((g) => g.length > 0).join(", ");
  if (!groups) return ""; // Skip effects with no group names
  const effect = cueEffect.effect;
  const parts: string[] = [];

  // All effect types use comma-separated parameters, each written as the
  // document holds it: what a line said comes back out of the editor, and
  // the forms offer each type only the parameters it reads.
  for (const color of effect.colors) {
    parts.push(`color: ${quoted(color)}`);
  }
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
  if (effect.pattern !== undefined) parts.push(`pattern: ${effect.pattern}`);
  if (effect.saturation !== undefined)
    parts.push(`saturation: ${effect.saturation}`);
  if (effect.brightness !== undefined)
    parts.push(`brightness: ${effect.brightness}`);
  // `move`: a focus point or explicit angles, never both.
  if (effect.focus !== undefined && effect.focus !== "")
    parts.push(`focus: ${quoted(effect.focus)}`);
  else {
    if (effect.pan !== undefined && effect.pan !== "")
      parts.push(`pan: ${withDeg(effect.pan)}`);
    if (effect.tilt !== undefined && effect.tilt !== "")
      parts.push(`tilt: ${withDeg(effect.tilt)}`);
  }
  if (effect.from !== undefined && effect.from !== "")
    parts.push(`from: ${quoted(effect.from)}`);
  else {
    if (effect.from_pan !== undefined && effect.from_pan !== "")
      parts.push(`from_pan: ${withDeg(effect.from_pan)}`);
    if (effect.from_tilt !== undefined && effect.from_tilt !== "")
      parts.push(`from_tilt: ${withDeg(effect.from_tilt)}`);
  }
  if (effect.easing !== undefined) parts.push(`easing: ${effect.easing}`);

  // Extra params
  for (const [k, v] of Object.entries(effect.extra)) {
    parts.push(`${k}: ${v}`);
  }

  if (parts.length > 0) {
    return `${groups}: ${effect.type}, ${parts.join(", ")}`;
  }
  return `${groups}: ${effect.type}`;
}

/** An angle as the DSL wants it: `45deg`; a bare number gets the unit. */
function withDeg(value: string): string {
  const v = value.trim();
  return v.endsWith("deg") || isParam(v) ? v : `${v}deg`;
}

/** A sequence parameter, `$name`, substituted before the value is read. */
function isParam(value: string): boolean {
  return /^\$[A-Za-z_][\w-]*$/.test(value.trim());
}

/** A string value in quotes, unless it is a parameter, which must stay bare. */
function quoted(value: string): string {
  return isParam(value) ? value.trim() : `"${value}"`;
}

function serializeLayerCommand(cmd: LayerCommand): string {
  const params: string[] = [];
  if (cmd.layer) params.push(`layer: ${cmd.layer}`);
  if (cmd.intensity) params.push(`intensity: ${cmd.intensity}`);
  if (cmd.speed) params.push(`speed: ${cmd.speed}`);
  return `${cmd.command}(${params.join(", ")})`;
}

function serializeSequenceRef(ref: SequenceRef): string {
  const parts: string[] = [];
  if (ref.stop) parts.push("stop ");
  parts.push(`sequence "${ref.name}"`);
  if (ref.args && !ref.stop) parts.push(serializeArgList(ref.args));
  if (ref.loop && !ref.stop) parts.push(` loop: ${ref.loop}`);
  return parts.join("");
}

/** `(name: value, …)`; a parameter with no default is its name alone. */
function serializeArgList(args: SequenceArg[]): string {
  const items = args.map((a) =>
    a.value === undefined ? a.name : `${a.name}: ${a.value}`,
  );
  return `(${items.join(", ")})`;
}
