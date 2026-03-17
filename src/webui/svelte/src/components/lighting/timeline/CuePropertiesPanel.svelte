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
  import type {
    Cue,
    CueEffect,
    LayerCommand,
    SequenceRef,
    Timestamp,
    SubLaneType,
  } from "../../../lib/lighting/types";
  import TimestampInput from "../TimestampInput.svelte";
  import EffectForm from "../EffectForm.svelte";
  import LayerCommandForm from "../LayerCommandForm.svelte";
  import SequenceRefForm from "../SequenceRefForm.svelte";
  import {
    formatMs,
    timestampToMs,
  } from "../../../lib/lighting/timeline-state";
  import type { TempoSection } from "../../../lib/lighting/types";

  interface Props {
    cue: Cue;
    laneName: string;
    groups: string[];
    sequenceNames: string[];
    tempo?: TempoSection;
    focusTab?: SubLaneType | null;
    onchange: (cue: Cue) => void;
    ondelete: () => void;
    onclose: () => void;
  }

  let {
    cue,
    laneName,
    groups,
    sequenceNames,
    tempo,
    focusTab,
    onchange,
    ondelete,
    onclose,
  }: Props = $props();

  // Use focusTab from the sub-lane selection; fall back to "effects" when
  // no sub-lane is set (e.g. sequence editor combined view)
  let activeTab = $derived(focusTab ?? "effects");
  let absTime = $derived(formatMs(timestampToMs(cue.timestamp, tempo)));

  const sectionLabels: Record<string, string> = {
    effects: "Effects",
    commands: "Commands",
    sequences: "Sequences",
  };

  function updateTimestamp(ts: Timestamp) {
    onchange({ ...cue, timestamp: ts });
  }

  function updateEffect(index: number, effect: CueEffect) {
    const effects = [...cue.effects];
    effects[index] = effect;
    onchange({ ...cue, effects });
  }

  function deleteEffect(index: number) {
    onchange({ ...cue, effects: cue.effects.filter((_, i) => i !== index) });
  }

  function addEffect() {
    const newEffect: CueEffect = {
      groups: ["all"],
      effect: { type: "static", colors: [], extra: {} },
    };
    onchange({ ...cue, effects: [...cue.effects, newEffect] });
  }

  function updateCommand(index: number, cmd: LayerCommand) {
    const commands = [...cue.commands];
    commands[index] = cmd;
    onchange({ ...cue, commands });
  }

  function deleteCommand(index: number) {
    onchange({ ...cue, commands: cue.commands.filter((_, i) => i !== index) });
  }

  function addCommand() {
    onchange({
      ...cue,
      commands: [...cue.commands, { command: "clear" }],
    });
  }

  function updateSequenceRef(index: number, ref: SequenceRef) {
    const sequences = [...cue.sequences];
    sequences[index] = ref;
    onchange({ ...cue, sequences });
  }

  function deleteSequenceRef(index: number) {
    onchange({
      ...cue,
      sequences: cue.sequences.filter((_, i) => i !== index),
    });
  }

  function addSequenceRef() {
    onchange({
      ...cue,
      sequences: [...cue.sequences, { name: "" }],
    });
  }
</script>

<div class="props-panel">
  <div class="props-header">
    <div class="props-info">
      <span class="props-lane">{laneName}</span>
      <span class="props-sep">&middot;</span>
      <span class="props-time">{absTime}</span>
      <TimestampInput value={cue.timestamp} onchange={updateTimestamp} />
    </div>

    <span class="props-section">{sectionLabels[activeTab]}</span>

    <div class="props-actions">
      <button
        class="btn btn-sm btn-danger"
        title="Delete cue"
        onclick={ondelete}
      >
        Delete
      </button>
      <button class="btn btn-sm" title="Close" onclick={onclose}>
        Close
      </button>
    </div>
  </div>

  <div class="props-body">
    {#if activeTab === "effects"}
      <div class="tab-content">
        <div class="tab-toolbar">
          <button class="btn btn-sm" onclick={addEffect}>+ Effect</button>
        </div>
        <div class="items-grid">
          {#each cue.effects as eff, i (i)}
            <EffectForm
              effect={eff}
              {groups}
              onchange={(e) => updateEffect(i, e)}
              ondelete={() => deleteEffect(i)}
            />
          {/each}
          {#if cue.effects.length === 0}
            <p class="empty-hint">No effects. Click "+ Effect" to add one.</p>
          {/if}
        </div>
      </div>
    {:else if activeTab === "commands"}
      <div class="tab-content">
        <div class="tab-toolbar">
          <button class="btn btn-sm" onclick={addCommand}>+ Command</button>
        </div>
        <div class="items-grid">
          {#each cue.commands as cmd, i (i)}
            <LayerCommandForm
              command={cmd}
              onchange={(c) => updateCommand(i, c)}
              ondelete={() => deleteCommand(i)}
            />
          {/each}
          {#if cue.commands.length === 0}
            <p class="empty-hint">No commands.</p>
          {/if}
        </div>
      </div>
    {:else if activeTab === "sequences"}
      <div class="tab-content">
        <div class="tab-toolbar">
          <button class="btn btn-sm" onclick={addSequenceRef}>+ Sequence</button
          >
        </div>
        <div class="items-grid">
          {#each cue.sequences as seqRef, i (i)}
            <SequenceRefForm
              ref={seqRef}
              {sequenceNames}
              onchange={(r) => updateSequenceRef(i, r)}
              ondelete={() => deleteSequenceRef(i)}
            />
          {/each}
          {#if cue.sequences.length === 0}
            <p class="empty-hint">No sequence references.</p>
          {/if}
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .props-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .props-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    flex-wrap: wrap;
  }
  .props-info {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .props-lane {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .props-sep {
    color: var(--text-dim);
  }
  .props-time {
    font-family: var(--mono);
    font-size: 13px;
    color: var(--text-muted);
  }
  .props-section {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-left: auto;
  }
  .props-actions {
    display: flex;
    gap: 4px;
  }
  .props-body {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .tab-content {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tab-toolbar {
    display: flex;
    gap: 6px;
    padding: 8px 12px 0;
  }
  .items-grid {
    padding: 4px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .empty-hint {
    color: var(--text-dim);
    font-size: 13px;
    padding: 8px 0;
  }
</style>
