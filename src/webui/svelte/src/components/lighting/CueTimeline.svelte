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
  import type { Cue, Timestamp } from "../../lib/lighting/types";
  import CueRow from "./CueRow.svelte";

  interface Props {
    cues: Cue[];
    groups: string[];
    sequenceNames: string[];
    onchange: (cues: Cue[]) => void;
  }

  let { cues, groups, sequenceNames, onchange }: Props = $props();

  function updateCue(index: number, cue: Cue) {
    const updated = [...cues];
    updated[index] = cue;
    onchange(updated);
  }

  function deleteCue(index: number) {
    onchange(cues.filter((_, i) => i !== index));
  }

  function addCue() {
    let ts: Timestamp;
    if (cues.length > 0) {
      const last = cues[cues.length - 1].timestamp;
      if (last.type === "measure_beat") {
        ts = {
          type: "measure_beat",
          measure: (last.measure ?? 1) + 1,
          beat: 1,
        };
      } else {
        ts = { type: "absolute", ms: (last.ms ?? 0) + 5000 };
      }
    } else {
      ts = { type: "absolute", ms: 0 };
    }
    onchange([
      ...cues,
      { timestamp: ts, effects: [], commands: [], sequences: [] },
    ]);
  }

  function moveCue(from: number, to: number) {
    if (to < 0 || to >= cues.length) return;
    const updated = [...cues];
    const [item] = updated.splice(from, 1);
    updated.splice(to, 0, item);
    onchange(updated);
  }
</script>

<div class="cue-timeline">
  <div class="timeline-header">
    <span class="cue-count"
      >{cues.length} cue{cues.length !== 1 ? "s" : ""}</span
    >
    <button class="btn btn-sm" onclick={addCue}>+ Cue</button>
  </div>

  <div class="cue-list">
    {#each cues as cue, i (i)}
      <div class="cue-item">
        <div class="reorder-btns">
          <button
            class="btn-icon small"
            disabled={i === 0}
            onclick={() => moveCue(i, i - 1)}
            title="Move up">&#9650;</button
          >
          <button
            class="btn-icon small"
            disabled={i === cues.length - 1}
            onclick={() => moveCue(i, i + 1)}
            title="Move down">&#9660;</button
          >
        </div>
        <div class="cue-content">
          <CueRow
            {cue}
            {groups}
            {sequenceNames}
            onchange={(c) => updateCue(i, c)}
            ondelete={() => deleteCue(i)}
          />
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .cue-timeline {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .timeline-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .cue-count {
    font-size: 12px;
    color: var(--text-muted);
  }
  .cue-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cue-item {
    display: flex;
    gap: 4px;
  }
  .reorder-btns {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding-top: 10px;
  }
  .btn-icon {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 10px;
    padding: 1px 4px;
    line-height: 1;
    border-radius: 3px;
  }
  .btn-icon:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  .btn-icon:disabled {
    opacity: 0.3;
    cursor: default;
  }
  .cue-content {
    flex: 1;
    min-width: 0;
  }
</style>
