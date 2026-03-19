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
  import { t } from "svelte-i18n";
  import type { SequenceRef } from "../../lib/lighting/types";

  interface Props {
    ref: SequenceRef;
    sequenceNames: string[];
    onchange: (ref: SequenceRef) => void;
    ondelete: () => void;
  }

  let { ref, sequenceNames, onchange, ondelete }: Props = $props();

  function update(key: string, value: unknown) {
    onchange({ ...ref, [key]: value });
  }
</script>

<div class="seq-ref-form">
  <div class="seq-row">
    <label class="field">
      <span class="field-label">{$t("sequenceRef.sequence")}</span>
      {#if sequenceNames.length > 0}
        <select
          class="input"
          value={ref.name}
          onchange={(e) =>
            update("name", (e.target as HTMLSelectElement).value)}
        >
          <option value="">{$t("sequenceRef.select")}</option>
          {#each sequenceNames as name (name)}
            <option value={name}>{name}</option>
          {/each}
        </select>
      {:else}
        <input
          type="text"
          class="input"
          value={ref.name}
          onchange={(e) => update("name", (e.target as HTMLInputElement).value)}
          placeholder={$t("sequenceRef.namePlaceholder")}
        />
      {/if}
    </label>
    <label class="field">
      <span class="field-label">{$t("sequenceRef.loop")}</span>
      <select
        class="input"
        value={ref.loop ?? ""}
        disabled={ref.stop ?? false}
        onchange={(e) => {
          const v = (e.target as HTMLSelectElement).value;
          update("loop", v || undefined);
        }}
      >
        <option value="">--</option>
        <option value="once">{$t("effect.once")}</option>
        <option value="loop">{$t("effect.loopOption")}</option>
        <option value="pingpong">{$t("effect.pingpong")}</option>
        <option value="random">{$t("effect.random")}</option>
      </select>
    </label>
    <div class="field checkbox-field">
      <label>
        <input
          type="checkbox"
          checked={ref.stop ?? false}
          onchange={(e) =>
            update("stop", (e.target as HTMLInputElement).checked || undefined)}
        />
        {$t("sequenceRef.stop")}
      </label>
    </div>
    <button
      class="btn-icon delete-btn"
      title={$t("sequenceRef.remove")}
      onclick={ondelete}
    >
      &#10005;
    </button>
  </div>
</div>

<style>
  .seq-ref-form {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
  }
  .seq-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    flex-wrap: wrap;
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
  .field .input {
    font-size: 13px;
    padding: 4px 6px;
    width: 140px;
  }
  .checkbox-field {
    flex-direction: row;
    align-items: center;
    gap: 4px;
    padding-bottom: 4px;
  }
  .checkbox-field label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    text-transform: none;
    color: var(--text);
  }
  .delete-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 4px 6px;
    border-radius: 4px;
  }
  .delete-btn:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
</style>
