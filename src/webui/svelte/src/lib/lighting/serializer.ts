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
import type { CueLine, TempoField } from "../../gen/rules/mtrack_rules";

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
  // The fields in the order the block wrote them; one it left out is
  // written after them only once it is set to something other than the
  // parser's default for it (a block with no `start:` stays without one).
  const order: TempoField[] = [...(tempo.fields ?? [])];
  const unset: Record<string, boolean> = {
    start: tempo.start.value === 0,
    bpm: tempo.bpm === 120,
    time_signature:
      tempo.time_signature[0] === 4 && tempo.time_signature[1] === 4,
    changes:
      tempo.changes.length === 0 && !tempo.changes_comments_after?.length,
  };
  for (const f of ["start", "bpm", "time_signature", "changes"] as const) {
    if (!order.includes(f) && !(tempo.fields && unset[f])) order.push(f);
  }
  const writeField = (name: TempoField) => {
    if (name === "start") {
      // As written, while the value is still the one that was read.
      const { text, value, unit } = tempo.start;
      const kept =
        text && parseFloat(text) === value && text.endsWith(unit)
          ? text
          : undefined;
      field("start", `start: ${kept ?? `${value}${unit}`}`);
    } else if (name === "bpm") field("bpm", `bpm: ${tempo.bpm}`);
    else if (name === "time_signature")
      field(
        "time_signature",
        `time_signature: ${tempo.time_signature[0]}/${tempo.time_signature[1]}`,
      );
    else if (name === "changes") writeChanges();
  };
  const writeChanges = () => {
    if (tempo.changes.length === 0 && !tempo.changes_comments_after?.length)
      return;
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
  };
  for (const name of order) writeField(name);
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

    // The cue's lines, each kind in its list's order, in the order the
    // file wrote them when the lists still match that order (an edit that
    // adds or removes a line writes the kinds grouped).
    const kinds = {
      effect: cue.effects.length,
      command: cue.commands.length,
      reference: cue.sequences.length,
      offset: cue.offset_measures !== undefined ? 1 : 0,
      reset: cue.reset_measures ? 1 : 0,
      loop: cue.loops?.length ?? 0,
    };
    const written = cue.lines ?? [];
    const counts: Record<string, number> = {};
    for (const k of written) counts[k] = (counts[k] ?? 0) + 1;
    const asWritten = Object.entries(kinds).every(
      ([k, n]) => (counts[k] ?? 0) === n,
    );
    const order: CueLine[] = asWritten
      ? written
      : [
          ...Array<CueLine>(kinds.effect).fill("effect"),
          ...Array<CueLine>(kinds.command).fill("command"),
          ...Array<CueLine>(kinds.reference).fill("reference"),
          ...Array<CueLine>(kinds.offset).fill("offset"),
          ...Array<CueLine>(kinds.reset).fill("reset"),
          ...Array<CueLine>(kinds.loop).fill("loop"),
        ];
    const next: Record<string, number> = {};
    for (const kind of order) {
      const i = next[kind] ?? 0;
      next[kind] = i + 1;
      if (kind === "effect") {
        const eff = cue.effects[i];
        const s = serializeEffect(eff);
        if (!s) continue;
        lines.push(...own(eff.comments_before, "    "));
        lines.push(line(`    ${s}`, eff.comment));
      } else if (kind === "command") {
        const cmd = cue.commands[i];
        lines.push(...own(cmd.comments_before, "    "));
        lines.push(line(`    ${serializeLayerCommand(cmd)}`, cmd.comment));
      } else if (kind === "reference") {
        const ref = cue.sequences[i];
        lines.push(...own(ref.comments_before, "    "));
        lines.push(line(`    ${serializeSequenceRef(ref)}`, ref.comment));
      } else if (kind === "offset") {
        lines.push(...own(cue.offset_comments_before, "    "));
        lines.push(
          line(
            `    offset ${cue.offset_measures} measures`,
            cue.offset_comment,
          ),
        );
      } else if (kind === "reset") {
        lines.push(...own(cue.reset_comments_before, "    "));
        lines.push(line(`    reset_measures`, cue.reset_comment));
      } else if (kind === "loop") {
        // Inline loops, kept as the text they were read from.
        const loop = cue.loops![i];
        lines.push(...own(loop.comments_before, "    "));
        lines.push(line(`    ${loop.text}`, loop.comment));
      }
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
  // As written, when the timestamp is the one that was read.
  if (ts.text) return ts.text.replace(/^@/, "");
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
  // Each parameter as the document holds it, in order.
  const parts = effect.params.map((p) => `${p.name}: ${p.value}`);
  if (parts.length > 0) {
    const sep = effect.no_comma ? " " : ", ";
    return `${groups}: ${effect.type}${sep}${parts.join(", ")}`;
  }
  return `${groups}: ${effect.type}`;
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
  if (ref.loop && !ref.stop)
    parts.push(`${ref.loop_comma ? "," : ""} loop: ${ref.loop}`);
  return parts.join("");
}

/** `(name: value, …)`; a parameter with no default is its name alone. */
function serializeArgList(args: SequenceArg[]): string {
  const items = args.map((a) =>
    a.value === undefined ? a.name : `${a.name}: ${a.value}`,
  );
  return `(${items.join(", ")})`;
}
