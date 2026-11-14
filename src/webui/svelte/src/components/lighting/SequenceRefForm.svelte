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
  import { rules } from "../../lib/rules";
  import { isColorText, useSequenceParams } from "../../lib/lighting/palette";
  import ColorInput from "./ColorInput.svelte";

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

  const sequenceParams = useSequenceParams();

  /** The color arguments of the sequence this plays: each parameter whose
   *  default or bound value is a color or a palette slot, with the value
   *  the pass takes (the one bound here, else the default). */
  let colorArgs = $derived(
    (sequenceParams()[ref.name] ?? []).flatMap((param) => {
      const bound = ref.args?.find((a) => a.name === param.name)?.value;
      return isColorText(bound) || isColorText(param.value)
        ? [{ name: param.name, value: bound ?? param.value ?? "" }]
        : [];
    }),
  );

  /** Binds argument `name` to `color`, as written in the file. */
  function setColorArg(name: string, color: string) {
    const value = rules.quote_value(color);
    const args = [...(ref.args ?? [])];
    const at = args.findIndex((a) => a.name === name);
    if (at >= 0) args[at] = { ...args[at], value };
    else args.push({ name, value });
    update("args", args);
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
        {#each rules.vocabulary().loops as l (l)}
          <option value={l}>{l}</option>
        {/each}
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
  {#if colorArgs.length > 0}
    <div class="seq-args" data-testid="sequence-color-args">
      {#each colorArgs as arg (arg.name)}
        <div class="field">
          <span class="field-label">{arg.name}</span>
          <ColorInput
            colors={[rules.unquote_value(arg.value)]}
            multi={false}
            onchange={(colors) => setColorArg(arg.name, colors[0] ?? "")}
          />
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .seq-args {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
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
