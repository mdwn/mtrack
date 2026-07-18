<!-- *     * Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
     *
     * This program is free software: you can redistribute it and/or modify it under
     * the terms of the GNU General Public License as published by the Free Software
     * Foundation, version 3.
     *
     * This program is distributed in the hope that it will be useful, but WITHOUT
     * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
     * FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
     *
     * You should have received a copy of the GNU General Public License along with
     * this program. If not, see <https://www.gnu.org/licenses/>.
     *
     * -->
<script lang="ts">
  import type { TempoConfig, TempoChangeConfig } from "../../lib/api/songs";
  import type { TempoSection, TempoChange } from "../../lib/lighting/types";
  import TempoEditor from "../lighting/TempoEditor.svelte";

  interface Props {
    /** The song.yaml `tempo:` block, or null when not configured. */
    tempo: TempoConfig | null;
    onchange: (tempo: TempoConfig | null) => void;
    songName: string;
    hasBeatGrid?: boolean;
    hasMidi?: boolean;
  }

  let {
    tempo,
    onchange,
    songName,
    hasBeatGrid = false,
    hasMidi = false,
  }: Props = $props();

  function parseTimeSignature(raw: string | undefined): [number, number] {
    const match = /^\s*(\d+)\s*\/\s*(\d+)\s*$/.exec(raw ?? "");
    if (!match) return [4, 4];
    return [parseInt(match[1]), parseInt(match[2])];
  }

  /** song.yaml tempo config -> the lighting TempoEditor's data model. */
  function configToSection(config: TempoConfig): TempoSection {
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

  function parseTransition(
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

  /** The lighting TempoEditor's data model -> song.yaml tempo config. */
  function sectionToConfig(section: TempoSection): TempoConfig {
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
    const changes = section.changes
      // The config format is measure-based; the editor is locked to
      // measure/beat mode so this filter is only a safety net.
      .filter((c) => c.timestamp.type === "measure_beat")
      .map((c) => {
        const change: TempoChangeConfig = {
          measure: c.timestamp.measure ?? 1,
        };
        if (c.timestamp.beat !== undefined && c.timestamp.beat !== 1) {
          change.beat = c.timestamp.beat;
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
        return change;
      });
    if (changes.length > 0) {
      config.changes = changes;
    }
    return config;
  }

  let section = $derived(tempo ? configToSection(tempo) : undefined);
</script>

<TempoEditor
  tempo={section}
  {songName}
  {hasBeatGrid}
  {hasMidi}
  measureOnly
  onchange={(updated) => onchange(updated ? sectionToConfig(updated) : null)}
/>
