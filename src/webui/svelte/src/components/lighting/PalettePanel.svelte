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
  import { get } from "svelte/store";
  import type { LightFile } from "../../lib/lighting/types";
  import type {
    PaletteEdit,
    PaletteEditError,
  } from "../../gen/rules/mtrack_rules";
  import {
    describeUse,
    editPalette,
    ownSlots,
  } from "../../lib/lighting/palette";
  import { rules } from "../../lib/rules";
  import ColorInput from "./ColorInput.svelte";

  interface Props {
    /** The file whose `palette { }` block this edits. */
    file: LightFile;
    /** The file's name, said when the song has several. */
    fileName?: string;
    /** Prefix for ids and test ids. */
    idPrefix?: string;
    /** Called with the file after an edit the player accepts. */
    onchange: (file: LightFile) => void;
  }

  let { file, fileName, idPrefix = "palette", onchange }: Props = $props();

  let slots = $derived(ownSlots(file));
  /** A slot's color as written, for the text field (its quotes aside). */
  let written = $derived(
    Object.fromEntries(
      (file.palette?.slots ?? []).map((s) => [s.name, s.value]),
    ),
  );

  let newName = $state("");
  let newColor = $state("#ffffff");
  /** The last refusal, with the slot it was about. */
  let error = $state<PaletteEditError | null>(null);

  /** Makes an edit the player decides on; the file only changes when it
   *  accepts. Returns whether it did. */
  function apply(edit: PaletteEdit): boolean {
    const result = editPalette(file, edit);
    if (result.error) {
      error = result.error;
      return false;
    }
    error = null;
    onchange(result.file);
    return true;
  }

  function add() {
    const name = newName.trim();
    if (name === "") return;
    if (apply({ op: "add", name, value: newColor })) newName = "";
  }

  function errorText(e: PaletteEditError): string {
    return get(t)(`palette.error.${e.code}`, { values: { slot: e.slot } });
  }
</script>

<div class="palette-panel" data-testid={`${idPrefix}-panel`}>
  {#if fileName}
    <div class="palette-file">
      {$t("palette.file", { values: { file: fileName } })}
    </div>
  {/if}
  {#if slots.length === 0}
    <p class="empty">{$t("palette.noSlotsFile")}</p>
  {/if}
  {#each slots as slot (slot.name)}
    <div class="slot" data-testid={`${idPrefix}-slot`} data-slot={slot.name}>
      <input
        type="text"
        class="input slot-name"
        data-testid={`${idPrefix}-name`}
        value={slot.name}
        aria-label={$t("palette.slotName", { values: { slot: slot.name } })}
        onchange={(e) => {
          const el = e.target as HTMLInputElement;
          const to = el.value.trim();
          if (to === slot.name) return;
          // A refused rename puts the old name back in the field.
          if (!apply({ op: "rename", from: slot.name, to })) {
            el.value = slot.name;
          }
        }}
      />
      <div
        class="slot-color"
        aria-label={$t("palette.slotColor", { values: { slot: slot.name } })}
        role="group"
      >
        <ColorInput
          colors={[rules.unquote_value(written[slot.name] ?? "")]}
          multi={false}
          allowSlots={false}
          onchange={(colors) =>
            apply({ op: "set", name: slot.name, value: colors[0] ?? "" })}
        />
      </div>
      <button
        class="btn-icon"
        type="button"
        data-testid={`${idPrefix}-remove`}
        title={$t("palette.removeSlot", { values: { slot: slot.name } })}
        aria-label={$t("palette.removeSlot", { values: { slot: slot.name } })}
        onclick={() => apply({ op: "remove", name: slot.name })}
      >
        &#10005;
      </button>
    </div>
  {/each}

  <div class="add-slot">
    <input
      type="text"
      class="input slot-name"
      data-testid={`${idPrefix}-new-name`}
      placeholder="accent"
      bind:value={newName}
      aria-label={$t("palette.newSlotName")}
      onkeydown={(e) => {
        if (e.key === "Enter") add();
      }}
    />
    <input
      type="color"
      class="new-color"
      bind:value={newColor}
      aria-label={$t("palette.newSlotColor")}
    />
    <button
      class="btn btn-sm"
      type="button"
      data-testid={`${idPrefix}-add`}
      disabled={newName.trim() === ""}
      onclick={add}>{$t("palette.addSlot")}</button
    >
  </div>

  {#if error}
    <div class="refusal" role="alert" data-testid={`${idPrefix}-error`}>
      <div>{errorText(error)}</div>
      {#if error.code === "in_use" && error.uses}
        <div class="uses-title">
          {$t("palette.usedBy", { values: { slot: error.slot } })}
        </div>
        <ul class="uses" data-testid={`${idPrefix}-uses`}>
          {#each error.uses as use, i (i)}
            <li>{describeUse(use, get(t))}</li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</div>

<style>
  .palette-panel {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .palette-file {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-fg-2);
  }
  .empty {
    margin: 0;
    font-size: 13px;
    color: var(--nc-fg-2);
  }
  .slot,
  .add-slot {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .slot-name {
    width: 9rem;
    font-family: var(--mono);
    font-size: 13px !important;
    padding: 4px 6px !important;
  }
  .new-color {
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: none;
    cursor: pointer;
  }
  .btn-icon {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 12px;
    padding: 2px 6px;
    border-radius: 4px;
  }
  .btn-icon:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .refusal {
    padding: 8px 10px;
    border: 1px solid var(--nc-amber-fg);
    border-radius: var(--radius);
    font-size: 13px;
    color: var(--nc-fg-1);
  }
  .uses-title {
    margin-top: 6px;
    color: var(--nc-fg-2);
  }
  .uses {
    margin: 4px 0 0;
    padding-left: 18px;
    font-family: var(--nc-font-mono);
    font-size: 12px;
  }
</style>
