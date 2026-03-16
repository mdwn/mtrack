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
  } from "../../lib/lighting/types";
  import TimestampInput from "./TimestampInput.svelte";
  import EffectForm from "./EffectForm.svelte";
  import LayerCommandForm from "./LayerCommandForm.svelte";
  import SequenceRefForm from "./SequenceRefForm.svelte";

  interface Props {
    cue: Cue;
    groups: string[];
    sequenceNames: string[];
    onchange: (cue: Cue) => void;
    ondelete: () => void;
  }

  let { cue, groups, sequenceNames, onchange, ondelete }: Props = $props();

  let expanded = $state(true);

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
      groups: [""],
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

  function updateComment(text: string) {
    onchange({ ...cue, comment: text || undefined });
  }

  let summary = $derived.by(() => {
    const parts: string[] = [];
    if (cue.effects.length > 0) {
      parts.push(
        `${cue.effects.length} effect${cue.effects.length > 1 ? "s" : ""}`,
      );
    }
    if (cue.commands.length > 0) {
      parts.push(
        `${cue.commands.length} cmd${cue.commands.length > 1 ? "s" : ""}`,
      );
    }
    if (cue.sequences.length > 0) {
      parts.push(`${cue.sequences.length} seq`);
    }
    return parts.join(", ") || "empty";
  });
</script>

<div class="cue-row" class:collapsed={!expanded}>
  <div class="cue-header">
    <button class="expand-btn" onclick={() => (expanded = !expanded)}>
      {expanded ? "\u25BC" : "\u25B6"}
    </button>
    <TimestampInput value={cue.timestamp} onchange={updateTimestamp} />
    {#if !expanded}
      <span class="cue-summary">{summary}</span>
    {/if}
    <div class="cue-actions">
      <button class="btn-icon delete-cue" title="Delete cue" onclick={ondelete}>
        &#10005;
      </button>
    </div>
  </div>

  {#if expanded}
    <div class="cue-body">
      {#if cue.comment !== undefined}
        <label class="field comment-field">
          <span class="field-label">Comment</span>
          <input
            type="text"
            class="input"
            value={cue.comment ?? ""}
            onchange={(e) =>
              updateComment((e.target as HTMLInputElement).value)}
            placeholder="Cue description..."
          />
        </label>
      {:else}
        <button
          class="btn btn-sm add-comment"
          onclick={() => updateComment("")}
        >
          + Comment
        </button>
      {/if}

      <!-- Effects -->
      <div class="section">
        <div class="section-header">
          <span class="section-label">Effects</span>
          <button class="btn btn-sm" onclick={addEffect}>+ Effect</button>
        </div>
        {#each cue.effects as eff, i (i)}
          <EffectForm
            effect={eff}
            {groups}
            onchange={(e) => updateEffect(i, e)}
            ondelete={() => deleteEffect(i)}
          />
        {/each}
      </div>

      <!-- Layer Commands -->
      <div class="section">
        <div class="section-header">
          <span class="section-label">Commands</span>
          <button class="btn btn-sm" onclick={addCommand}>+ Command</button>
        </div>
        {#each cue.commands as cmd, i (i)}
          <LayerCommandForm
            command={cmd}
            onchange={(c) => updateCommand(i, c)}
            ondelete={() => deleteCommand(i)}
          />
        {/each}
      </div>

      <!-- Sequence References -->
      {#if sequenceNames.length > 0 || cue.sequences.length > 0}
        <div class="section">
          <div class="section-header">
            <span class="section-label">Sequences</span>
            <button class="btn btn-sm" onclick={addSequenceRef}
              >+ Sequence</button
            >
          </div>
          {#each cue.sequences as seqRef, i (i)}
            <SequenceRefForm
              ref={seqRef}
              {sequenceNames}
              onchange={(r) => updateSequenceRef(i, r)}
              ondelete={() => deleteSequenceRef(i)}
            />
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .cue-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    overflow: hidden;
  }
  .cue-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: rgba(255, 255, 255, 0.02);
  }
  .expand-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 11px;
    padding: 2px 4px;
    width: 20px;
  }
  .cue-summary {
    color: var(--text-muted);
    font-size: 13px;
    flex: 1;
  }
  .cue-actions {
    margin-left: auto;
    display: flex;
    gap: 4px;
  }
  .delete-cue {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 4px 6px;
    border-radius: 4px;
  }
  .delete-cue:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .cue-body {
    padding: 8px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .section-label {
    font-size: 12px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .field-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .comment-field .input {
    font-size: 13px;
  }
  .add-comment {
    align-self: flex-start;
    font-size: 12px;
    padding: 2px 8px;
  }
</style>
