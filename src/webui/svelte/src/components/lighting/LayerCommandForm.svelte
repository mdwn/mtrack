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
  import type { LayerCommand } from "../../lib/lighting/types";
  import { LAYERS } from "../../lib/lighting/types";

  const COMMANDS = [
    "clear",
    "release",
    "freeze",
    "unfreeze",
    "master",
  ] as const;

  interface Props {
    command: LayerCommand;
    onchange: (cmd: LayerCommand) => void;
    ondelete: () => void;
  }

  let { command, onchange, ondelete }: Props = $props();

  function update(key: string, value: string | undefined) {
    onchange({ ...command, [key]: value });
  }
</script>

<div class="cmd-form">
  <div class="cmd-row">
    <label class="field">
      <span class="field-label">{$t("effect.command.command")}</span>
      <select
        class="input"
        value={command.command}
        onchange={(e) =>
          update("command", (e.target as HTMLSelectElement).value)}
      >
        {#each COMMANDS as c (c)}
          <option value={c}>{c}</option>
        {/each}
      </select>
    </label>
    <label class="field">
      <span class="field-label">{$t("effect.command.layer")}</span>
      <select
        class="input"
        value={command.layer ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLSelectElement).value;
          update("layer", v || undefined);
        }}
      >
        <option value="">--</option>
        {#each LAYERS as l (l)}
          <option value={l}>{l}</option>
        {/each}
      </select>
    </label>

    {#if command.command === "release"}
      <label class="field">
        <span class="field-label">{$t("effect.command.time")}</span>
        <input
          type="text"
          class="input"
          placeholder="2s"
          value={command.time ?? ""}
          onchange={(e) => {
            const v = (e.target as HTMLInputElement).value;
            update("time", v || undefined);
          }}
        />
      </label>
    {/if}

    {#if command.command === "master"}
      <label class="field">
        <span class="field-label">{$t("effect.command.intensity")}</span>
        <input
          type="text"
          class="input"
          placeholder="100%"
          value={command.intensity ?? ""}
          onchange={(e) => {
            const v = (e.target as HTMLInputElement).value;
            update("intensity", v || undefined);
          }}
        />
      </label>
      <label class="field">
        <span class="field-label">{$t("effect.command.speed")}</span>
        <input
          type="text"
          class="input"
          placeholder="100%"
          value={command.speed ?? ""}
          onchange={(e) => {
            const v = (e.target as HTMLInputElement).value;
            update("speed", v || undefined);
          }}
        />
      </label>
    {/if}

    <button
      class="btn-icon delete-btn"
      title={$t("effect.command.removeCommand")}
      onclick={ondelete}
    >
      &#10005;
    </button>
  </div>
</div>

<style>
  .cmd-form {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
  }
  .cmd-row {
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
    width: 110px;
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
