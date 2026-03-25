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
  TempoSection,
  TempoChange,
  Timestamp,
  Cue,
  EffectParams,
  EffectType,
  LayerCommand,
  SequenceRef,
  Layer,
  BlendMode,
} from "./types";

const EFFECT_TYPES = new Set([
  "static",
  "cycle",
  "strobe",
  "pulse",
  "chase",
  "dimmer",
  "rainbow",
]);

/**
 * Parse a raw DSL string into a LightFile structure.
 */
export function parseLightFile(dsl: string): LightFile {
  const lines = dsl.split("\n");
  const result: LightFile = { sequences: [], shows: [] };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i].trim();

    // Skip empty lines and top-level comments
    if (line === "" || line.startsWith("#") || line.startsWith("//")) {
      i++;
      continue;
    }

    // Tempo block
    if (line.startsWith("tempo")) {
      const [tempo, newI] = parseTempoBlock(lines, i);
      result.tempo = tempo;
      i = newI;
      continue;
    }

    // Sequence block
    const seqMatch = line.match(/^sequence\s+"([^"]+)"\s*\{/);
    if (seqMatch) {
      const [seq, newI] = parseCueBlock(lines, i + 1);
      result.sequences.push({ name: seqMatch[1], cues: seq });
      i = newI;
      continue;
    }

    // Show block
    const showMatch = line.match(/^show\s+"([^"]+)"\s*\{/);
    if (showMatch) {
      const [cues, newI] = parseCueBlock(lines, i + 1);
      result.shows.push({ name: showMatch[1], cues });
      i = newI;
      continue;
    }

    i++;
  }

  return result;
}

/**
 * Parse a tempo { ... } block starting at the line with "tempo".
 */
function parseTempoBlock(
  lines: string[],
  startIdx: number,
): [TempoSection, number] {
  const tempo: TempoSection = {
    start: { value: 0, unit: "s" },
    bpm: 120,
    time_signature: [4, 4],
    changes: [],
  };

  let i = startIdx + 1; // skip the "tempo {" line
  let inChanges = false;

  while (i < lines.length) {
    const line = lines[i].trim();

    if (line === "}" || (line === "]" && !inChanges)) {
      // Check if this closes the tempo block or just the changes array
      if (!inChanges) {
        return [tempo, i + 1];
      }
    }

    if (line === "}") {
      return [tempo, i + 1];
    }

    // Skip comments
    if (line.startsWith("#") || line.startsWith("//") || line === "") {
      i++;
      continue;
    }

    // start: Ns
    const startMatch = line.match(/^start:\s*(.+)/);
    if (startMatch) {
      tempo.start = parseDurationString(startMatch[1]);
      i++;
      continue;
    }

    // bpm: N
    const bpmMatch = line.match(/^bpm:\s*(\d+(?:\.\d+)?)/);
    if (bpmMatch) {
      tempo.bpm = parseFloat(bpmMatch[1]);
      i++;
      continue;
    }

    // time_signature: N/N
    const tsMatch = line.match(/^time_signature:\s*(\d+)\/(\d+)/);
    if (tsMatch) {
      tempo.time_signature = [parseInt(tsMatch[1]), parseInt(tsMatch[2])];
      i++;
      continue;
    }

    // changes: [
    if (line.startsWith("changes:")) {
      inChanges = true;
      i++;
      continue;
    }

    // Tempo change entry: @M/B { ... }
    if (inChanges) {
      if (line === "]") {
        inChanges = false;
        i++;
        continue;
      }
      const changeMatch = line.match(/^@(\d+)\/(\d+(?:\.\d+)?)\s*\{([^}]*)\}/);
      if (changeMatch) {
        const change: TempoChange = {
          timestamp: {
            type: "measure_beat",
            measure: parseInt(changeMatch[1]),
            beat: parseFloat(changeMatch[2]),
          },
        };
        const params = changeMatch[3];
        const bpmM = params.match(/bpm:\s*(\d+(?:\.\d+)?)/);
        if (bpmM) change.bpm = parseFloat(bpmM[1]);
        const tsM = params.match(/time_signature:\s*(\d+)\/(\d+)/);
        if (tsM) change.time_signature = [parseInt(tsM[1]), parseInt(tsM[2])];
        const trM = params.match(/transition:\s*(\S+)/);
        if (trM) change.transition = trM[1].replace(/,/g, "");
        tempo.changes.push(change);
      }
    }

    i++;
  }

  return [tempo, i];
}

/**
 * Parse a block of cues inside a show or sequence { ... }.
 * Returns the cues and the line index after the closing brace.
 */
function parseCueBlock(lines: string[], startIdx: number): [Cue[], number] {
  const cues: Cue[] = [];
  let currentCue: Cue | null = null;
  let pendingComment: string | undefined;
  let i = startIdx;

  while (i < lines.length) {
    const line = lines[i].trim();

    // End of block
    if (line === "}") {
      if (currentCue) cues.push(currentCue);
      return [cues, i + 1];
    }

    // Empty line
    if (line === "") {
      i++;
      continue;
    }

    // Comment — buffer it for the next cue or timestamp
    if (line.startsWith("#") || line.startsWith("//")) {
      const commentText = line.startsWith("#")
        ? line.slice(1).trim()
        : line.slice(2).trim();
      pendingComment = pendingComment
        ? pendingComment + "\n" + commentText
        : commentText;
      i++;
      continue;
    }

    // Timestamp line: @MM:SS.mmm or @M/B or @S.sss
    const tsMatch = line.match(/^@(.+)/);
    if (tsMatch) {
      if (currentCue) cues.push(currentCue);
      const ts = parseTimestamp(tsMatch[1].trim());
      currentCue = {
        timestamp: ts,
        effects: [],
        commands: [],
        sequences: [],
      };
      if (pendingComment) {
        currentCue.comment = pendingComment;
        pendingComment = undefined;
      }
      i++;
      continue;
    }

    // Ensure we have a current cue to add to
    if (!currentCue) {
      i++;
      continue;
    }

    // Layer command: clear(...) / release(...) / freeze(...) / unfreeze(...) / master(...)
    const cmdMatch = line.match(
      /^(clear|release|freeze|unfreeze|master)\(([^)]*)\)/,
    );
    if (cmdMatch) {
      const cmd = parseLayerCommand(cmdMatch[1], cmdMatch[2]);
      currentCue.commands.push(cmd);
      i++;
      continue;
    }

    // Sequence reference: sequence "name"
    const seqRefMatch = line.match(
      /^(?:stop\s+)?sequence\s+"([^"]+)"(?:[,\s]+(.*))?/,
    );
    if (seqRefMatch) {
      const ref: SequenceRef = { name: seqRefMatch[1] };
      if (line.startsWith("stop")) ref.stop = true;
      if (seqRefMatch[2]) {
        const loopM = seqRefMatch[2].match(/loop:\s*(\w+)/);
        if (loopM) ref.loop = loopM[1];
      }
      currentCue.sequences.push(ref);
      i++;
      continue;
    }

    // Offset measures: offset N measures
    const offsetMatch = line.match(/^offset\s+(\d+)\s+measures?$/);
    if (offsetMatch) {
      currentCue.offset_measures = parseInt(offsetMatch[1]);
      i++;
      continue;
    }

    // Reset measures
    if (line === "reset_measures") {
      currentCue.reset_measures = true;
      i++;
      continue;
    }

    // Effect line: groups: type, params...
    // or: groups: type params...  (dimmer uses space instead of comma after type)
    const effectMatch = line.match(/^([^:]+):\s*(.+)/);
    if (effectMatch) {
      const groupsStr = effectMatch[1].trim();
      const groups = groupsStr.split(",").map((g) => g.trim());
      const rest = effectMatch[2].trim();
      const effect = parseEffectLine(rest);
      if (effect) {
        currentCue.effects.push({ groups, effect });
      }
    }

    i++;
  }

  if (currentCue) cues.push(currentCue);
  return [cues, i];
}

/**
 * Parse a timestamp string. Handles:
 * - MM:SS.mmm (absolute)
 * - S.sss (absolute seconds)
 * - M/B (measure/beat)
 */
export function parseTimestamp(str: string): Timestamp {
  // Measure/beat: M/B or M/B.f
  const mbMatch = str.match(/^(\d+)\/(\d+(?:\.\d+)?)$/);
  if (mbMatch) {
    return {
      type: "measure_beat",
      measure: parseInt(mbMatch[1]),
      beat: parseFloat(mbMatch[2]),
    };
  }

  // Absolute MM:SS.mmm
  const absMatch = str.match(/^(\d+):(\d+)\.(\d+)$/);
  if (absMatch) {
    const mins = parseInt(absMatch[1]);
    const secs = parseInt(absMatch[2]);
    const msStr = absMatch[3].padEnd(3, "0").slice(0, 3);
    const ms = mins * 60000 + secs * 1000 + parseInt(msStr);
    return { type: "absolute", ms };
  }

  // Absolute MM:SS (no milliseconds)
  const absNoMs = str.match(/^(\d+):(\d+)$/);
  if (absNoMs) {
    const ms = parseInt(absNoMs[1]) * 60000 + parseInt(absNoMs[2]) * 1000;
    return { type: "absolute", ms };
  }

  // Seconds only: S.sss
  const secMatch = str.match(/^(\d+(?:\.\d+)?)$/);
  if (secMatch) {
    const ms = Math.round(parseFloat(secMatch[1]) * 1000);
    return { type: "absolute", ms };
  }

  // Fallback
  return { type: "absolute", ms: 0 };
}

/**
 * Parse a duration string like "2s", "500ms", "1beat", "2measures"
 */
function parseDurationString(str: string): {
  value: number;
  unit: "ms" | "s" | "beats" | "measures";
} {
  str = str.trim();
  const match = str.match(/^(\d+(?:\.\d+)?)\s*(ms|s|beats?|measures?)$/);
  if (match) {
    let unit: "ms" | "s" | "beats" | "measures" = "s";
    const rawUnit = match[2];
    if (rawUnit === "ms") unit = "ms";
    else if (rawUnit === "s") unit = "s";
    else if (rawUnit === "beat" || rawUnit === "beats") unit = "beats";
    else if (rawUnit === "measure" || rawUnit === "measures") unit = "measures";
    return { value: parseFloat(match[1]), unit };
  }
  return { value: parseFloat(str) || 0, unit: "s" };
}

/**
 * Parse a layer command like "clear(layer: foreground)"
 */
function parseLayerCommand(cmd: string, paramsStr: string): LayerCommand {
  const result: LayerCommand = {
    command: cmd as LayerCommand["command"],
  };

  const layerM = paramsStr.match(/layer:\s*(\w+)/);
  if (layerM) result.layer = layerM[1];

  const timeM = paramsStr.match(/time:\s*(\S+)/);
  if (timeM) result.time = timeM[1];

  const intensityM = paramsStr.match(/intensity:\s*(\S+)/);
  if (intensityM) result.intensity = intensityM[1].replace(/,/g, "");

  const speedM = paramsStr.match(/speed:\s*(\S+)/);
  if (speedM) result.speed = speedM[1].replace(/,/g, "");

  return result;
}

/**
 * Parse the effect portion of a line, e.g.:
 * "static, color: \"blue\", intensity: 0.6, up_time: 2s"
 * "dimmer start_level: 0.0, end_level: 1.0, duration: 0.5s"
 */
function parseEffectLine(rest: string): EffectParams | null {
  // Extract effect type — first word
  const typeMatch = rest.match(/^(\w+)[,\s]?\s*(.*)/);
  if (!typeMatch) return null;

  const typeName = typeMatch[1].toLowerCase();
  if (!EFFECT_TYPES.has(typeName)) return null;

  const effect: EffectParams = {
    type: typeName as EffectType,
    colors: [],
    extra: {},
  };

  const paramsStr = typeMatch[2];
  if (!paramsStr) return effect;

  // Parse key: value pairs from the remaining string.
  // Handle repeated keys (like multiple "color:" for cycle).
  parseEffectParams(paramsStr, effect);

  return effect;
}

/**
 * Parse comma-separated key: value pairs into an EffectParams.
 * Handles quoted strings, repeated color keys, and unquoted values.
 */
function parseEffectParams(str: string, effect: EffectParams): void {
  // Tokenize by splitting on commas that are not inside quotes
  const pairs = splitParams(str);

  for (const pair of pairs) {
    const trimmed = pair.trim();
    if (!trimmed) continue;

    const kvMatch = trimmed.match(/^(\w+):\s*(.*)/);
    if (!kvMatch) continue;

    const key = kvMatch[1].toLowerCase();
    let val = kvMatch[2].trim();
    // Strip trailing comma
    if (val.endsWith(",")) val = val.slice(0, -1).trim();
    // Strip quotes
    if (
      (val.startsWith('"') && val.endsWith('"')) ||
      (val.startsWith("'") && val.endsWith("'"))
    ) {
      val = val.slice(1, -1);
    }

    switch (key) {
      case "color":
        effect.colors.push(val);
        break;
      case "intensity":
        effect.intensity = parseFloat(val);
        break;
      case "dimmer":
        effect.dimmer = val;
        break;
      case "frequency":
        effect.frequency = val;
        break;
      case "speed":
        effect.speed = val;
        break;
      case "direction":
        effect.direction = val;
        break;
      case "transition":
        effect.transition = val;
        break;
      case "duration":
        effect.duration = val;
        break;
      case "up_time":
        effect.up_time = val;
        break;
      case "hold_time":
        effect.hold_time = val;
        break;
      case "down_time":
        effect.down_time = val;
        break;
      case "start_level":
        effect.start_level = parseFloat(val);
        break;
      case "end_level":
        effect.end_level = parseFloat(val);
        break;
      case "curve":
        effect.curve = val;
        break;
      case "layer":
        effect.layer = val as Layer;
        break;
      case "blend_mode":
        effect.blend_mode = val as BlendMode;
        break;
      case "loop":
        effect.loop = val;
        break;
      case "base_level":
        effect.base_level = parseFloat(val);
        break;
      case "pulse_amplitude":
        effect.pulse_amplitude = parseFloat(val);
        break;
      case "duty_cycle":
        effect.duty_cycle = val;
        break;
      case "pattern":
        effect.pattern = val;
        break;
      case "saturation":
        effect.saturation = parseFloat(val);
        break;
      case "brightness":
        effect.brightness = parseFloat(val);
        break;
      default:
        effect.extra[key] = val;
        break;
    }
  }
}

/**
 * Split a string by commas, respecting quoted substrings.
 */
function splitParams(str: string): string[] {
  const parts: string[] = [];
  let current = "";
  let inQuote = false;
  let quoteChar = "";

  for (let i = 0; i < str.length; i++) {
    const ch = str[i];
    if (inQuote) {
      current += ch;
      if (ch === quoteChar) inQuote = false;
    } else if (ch === '"' || ch === "'") {
      inQuote = true;
      quoteChar = ch;
      current += ch;
    } else if (ch === ",") {
      parts.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  if (current.trim()) parts.push(current);
  return parts;
}
