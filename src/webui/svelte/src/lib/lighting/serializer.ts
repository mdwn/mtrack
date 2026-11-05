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
  TempoChange,
  TempoSection,
  Cue,
  CueEffect,
  LayerCommand,
  SequenceRef,
  SequenceArg,
} from "./types";
import type {
  CueLine,
  DocumentItem,
  OffsetLine,
  Param,
  Remarks,
  TempoField,
} from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

/**
 * Serialize a LightFile structure back to DSL text: what the document
 * holds, as written (`rules.document` reads the text back to the same
 * document).
 */
export function serializeLightFile(file: LightFile): string {
  const parts: string[] = [];

  // The items in the order the file wrote them, while the lists still
  // match it (an edit that adds or removes an item writes them grouped).
  const counts = {
    include: file.includes?.length ?? 0,
    tempo: file.tempo ? 1 : 0,
    sequence: file.sequences.length,
    show: file.shows.length,
  };
  const written = file.items ?? [];
  const seen: Record<string, number> = {};
  for (const k of written) seen[k] = (seen[k] ?? 0) + 1;
  const asWritten = Object.entries(counts).every(
    ([k, n]) => (seen[k] ?? 0) === n,
  );
  const order: DocumentItem[] = asWritten
    ? written
    : [
        ...Array<DocumentItem>(counts.include).fill("include"),
        ...Array<DocumentItem>(counts.tempo).fill("tempo"),
        ...Array<DocumentItem>(counts.sequence).fill("sequence"),
        ...Array<DocumentItem>(counts.show).fill("show"),
      ];
  const next: Record<string, number> = {};
  order.forEach((kind, i) => {
    const n = next[kind] ?? 0;
    next[kind] = n + 1;
    if (kind === "include") {
      const inc = file.includes![n];
      parts.push(...own(inc.comments_before, ""));
      parts.push(line(`include "${inc.written}"`, inc.comment));
      // Includes sit together, one blank line after the run.
      if (order[i + 1] !== "include") parts.push("");
    } else if (kind === "tempo") {
      parts.push(serializeTempo(file.tempo!));
      parts.push("");
    } else if (kind === "sequence") {
      const seq = file.sequences[n];
      const params = seq.params ? serializeArgList(seq.params, "    ") : "";
      const length = seq.length ? `, length: ${seq.length}` : "";
      parts.push(...own(seq.comments_before, ""));
      parts.push(
        line(`sequence "${seq.name}"${params}${length} {`, seq.comment),
      );
      parts.push(serializeCues(seq.cues));
      parts.push(...own(seq.comments_after, "    "));
      parts.push(line("}", seq.close_comment));
      parts.push("");
    } else {
      const show = file.shows[n];
      parts.push(...own(show.comments_before, ""));
      const head =
        show.unnamed && show.name === "" ? "show {" : `show "${show.name}" {`;
      parts.push(line(head, show.comment));
      parts.push(serializeCues(show.cues));
      parts.push(...own(show.comments_after, "    "));
      parts.push(line("}", show.close_comment));
      parts.push("");
    }
  });

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

/** An element of a list (a parameter, an argument, a group) as text, with
 *  the comments written around it. */
type Listed = Remarks & { text: string };

/** A list's elements, `, `-separated on one line, each with its comments
 *  where they were: a comment after an element ends the line after its
 *  comma, and comment lines above an element or after the last are lines
 *  of their own; a line the list goes on to starts at `indent`. `space`:
 *  whether the first element follows a space. `open` is whether the text
 *  ends a line, so what closes the list starts the next one. */
function list(
  items: Listed[],
  indent: string,
  space: boolean,
): { text: string; open: boolean } {
  let text = "";
  let open = false;
  const newline = () => {
    text += `\n${indent}`;
    open = true;
  };
  items.forEach((item, i) => {
    if (item.comments_before?.length) {
      if (!open) newline();
      for (const c of item.comments_before) {
        text += c;
        newline();
      }
    } else if (!open && (i > 0 || space)) text += " ";
    text += item.text + (i < items.length - 1 ? "," : "");
    open = false;
    if (item.comment) {
      text += ` ${item.comment}`;
      newline();
    }
    for (const c of item.comments_after ?? []) {
      if (!open) newline();
      text += c;
      newline();
    }
  });
  return { text, open };
}

/** A list that nothing closes, as the end of its line: one that ended a
 *  line ends there. */
function ended(listed: { text: string; open: boolean }, indent: string) {
  return listed.open ? listed.text.slice(0, -(indent.length + 1)) : listed.text;
}

/** A parameter as written, with its comments. */
function param(p: Param): Listed {
  return { ...p, text: `${p.name}: ${p.value}` };
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
  const defaults = rules.tempo_defaults();
  const unset: Record<string, boolean> = {
    start: tempo.start.value === 0,
    bpm: tempo.bpm === defaults.bpm,
    time_signature:
      tempo.time_signature[0] === defaults.time_signature[0] &&
      tempo.time_signature[1] === defaults.time_signature[1],
    changes:
      tempo.changes.length === 0 && !tempo.changes_comments_after?.length,
  };
  for (const f of ["start", "bpm", "time_signature", "changes"] as const) {
    if (!order.includes(f) && !(tempo.fields && unset[f])) order.push(f);
  }
  const writeField = (name: TempoField) => {
    if (name === "start") {
      // As written, while the value and the unit are still the ones read.
      const { written, value, unit } = tempo.start;
      const kept =
        written && written.value === value && written.unit === unit
          ? written.text
          : undefined;
      field("start", `start: ${kept ?? `${value}${unit}`}`);
    } else if (name === "bpm") {
      const kept =
        tempo.bpm_text && parseFloat(tempo.bpm_text) === tempo.bpm
          ? tempo.bpm_text
          : undefined;
      field("bpm", `bpm: ${kept ?? tempo.bpm}`);
    } else if (name === "time_signature") {
      const [n, d] = tempo.time_signature;
      const kept =
        tempo.time_signature_text?.replace(/\s+/g, "") === `${n}/${d}`
          ? tempo.time_signature_text
          : undefined;
      field("time_signature", `time_signature: ${kept ?? `${n}/${d}`}`);
    } else if (name === "changes") writeChanges();
  };
  const writeChanges = () => {
    if (tempo.changes.length === 0 && !tempo.changes_comments_after?.length)
      return;
    field("changes", "changes: [");
    tempo.changes.forEach((change, i) => {
      const ts = formatTimestamp(change.timestamp);
      const params = list(changeParams(change), "            ", true);
      const comma = i < tempo.changes.length - 1 ? "," : "";
      lines.push(...own(change.comments_before, "        "));
      const text = `        @${ts} {${params.text}${params.open ? "}" : " }"}`;
      if (comma && change.comment && change.comment_before_comma) {
        // The comment was between the `}` and the comma: it stays there.
        lines.push(line(text, change.comment));
        lines.push("        ,");
      } else {
        lines.push(line(`${text}${comma}`, change.comment));
      }
    });
    lines.push(...own(tempo.changes_comments_after, "        "));
    lines.push(line("    ]", at("end", true).join(" ") || undefined));
  };
  for (const name of order) writeField(name);
  lines.push(...own(at("end", false), "    "));

  lines.push(line("}", tempo.close_comment));
  return lines.join("\n");
}

/** A tempo change's parameters: as written, in order, spelling and with
 *  their comments, an edited one's value replaced where it was (see
 *  `edited`). */
function changeParams(change: TempoChange): Listed[] {
  const sig = change.time_signature
    ? `${change.time_signature[0]}/${change.time_signature[1]}`
    : undefined;
  return edited(
    change.written ?? [],
    {
      bpm: change.bpm !== undefined ? `${change.bpm}` : undefined,
      time_signature: sig,
      transition: change.transition,
    },
    (p) =>
      (p.name === "bpm" && parseFloat(p.value) === change.bpm) ||
      (p.name === "time_signature" && p.value === sig) ||
      (p.name === "transition" && p.value === change.transition),
  );
}

/** Written parameters with the values now `set`: each as written while
 *  it `holds` (its value still the one set), else its value replaced in
 *  place -- the last of a name, which is the one that counts; a name set
 *  to nothing is dropped, and one newly set is added at the end. */
function edited(
  written: Param[],
  set: Record<string, string | undefined>,
  holds: (p: Param) => boolean,
): Listed[] {
  const last = (name: string) => {
    for (let i = written.length - 1; i >= 0; i--)
      if (written[i].name === name) return i;
    return -1;
  };
  const params: Listed[] = [];
  written.forEach((p, i) => {
    const value = set[p.name];
    if (value === undefined || value === "") return;
    params.push(param(i === last(p.name) && !holds(p) ? { ...p, value } : p));
  });
  for (const [name, value] of Object.entries(set)) {
    if (value && last(name) < 0) params.push({ text: `${name}: ${value}` });
  }
  return params;
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
    // The `offset` lines as written, each with its comments, while they
    // still add up; an edited sum is one line, the comments kept on it.
    const read = cue.offsets ?? [];
    const sum = read.reduce((a, l) => a + l.measures, 0);
    const offsets: OffsetLine[] =
      read.length > 0 && sum === cue.offset_measures
        ? read
        : cue.offset_measures !== undefined
          ? [
              {
                measures: cue.offset_measures,
                comments_before: read.flatMap((l, i) => [
                  ...(l.comments_before ?? []),
                  ...(l.comment && i < read.length - 1 ? [l.comment] : []),
                ]),
                comment: read[read.length - 1]?.comment,
              },
            ]
          : [];
    const kinds = {
      effect: cue.effects.length,
      command: cue.commands.length,
      reference: cue.sequences.length,
      offset: offsets.length,
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
        const s = serializeEffect(eff, "        ");
        if (!s) continue;
        lines.push(...own(eff.comments_before, "    "));
        lines.push(line(`    ${s}`, eff.comment));
      } else if (kind === "command") {
        const cmd = cue.commands[i];
        lines.push(...own(cmd.comments_before, "    "));
        lines.push(
          line(`    ${serializeLayerCommand(cmd, "        ")}`, cmd.comment),
        );
      } else if (kind === "reference") {
        const ref = cue.sequences[i];
        lines.push(...own(ref.comments_before, "    "));
        lines.push(
          line(`    ${serializeSequenceRef(ref, "        ")}`, ref.comment),
        );
      } else if (kind === "offset") {
        const offset = offsets[i];
        lines.push(...own(offset.comments_before, "    "));
        lines.push(
          line(`    offset ${offset.measures} measures`, offset.comment),
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

/** An effect line; a line it goes on to starts at `indent`. */
function serializeEffect(cueEffect: CueEffect, indent: string): string {
  const named = cueEffect.groups
    .map((g, i) => ({ ...cueEffect.group_remarks?.[i], text: g }))
    .filter((g) => g.text.length > 0);
  if (named.length === 0) return ""; // Skip effects with no group names
  const groups = list(named, indent, false);
  const effect = cueEffect.effect;
  const head = `${groups.text}: ${effect.type}`;
  // Each parameter as the document holds it, in order.
  if (effect.params.length > 0) {
    const params = list(effect.params.map(param), indent, true);
    return `${head}${effect.no_comma ? "" : ","}${ended(params, indent)}`;
  }
  return head;
}

function serializeLayerCommand(cmd: LayerCommand, indent: string): string {
  // As written, an edited value replaced where it was.
  const set: Record<string, string | undefined> = {
    layer: cmd.layer,
    intensity: cmd.intensity,
    speed: cmd.speed,
  };
  const params = edited(cmd.written ?? [], set, (p) => p.value === set[p.name]);
  return `${cmd.command}(${list(params, indent, false).text})`;
}

function serializeSequenceRef(ref: SequenceRef, indent: string): string {
  const parts: string[] = [];
  if (ref.stop) parts.push("stop ");
  parts.push(`sequence "${ref.name}"`);
  if (ref.args && !ref.stop) parts.push(serializeArgList(ref.args, indent));
  if (ref.loop && !ref.stop)
    parts.push(`${ref.loop_comma ? "," : ""} loop: ${ref.loop}`);
  return parts.join("");
}

/** `(name: value, …)`; a parameter with no default is its name alone. */
function serializeArgList(args: SequenceArg[], indent: string): string {
  const items = args.map((a) => ({
    ...a,
    text: a.value === undefined ? a.name : `${a.name}: ${a.value}`,
  }));
  return `(${list(items, indent, false).text})`;
}
