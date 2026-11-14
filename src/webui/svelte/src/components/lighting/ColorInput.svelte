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
  import { rules } from "../../lib/rules";
  import {
    refOf,
    resolveColor,
    slotOfRef,
    usePalette,
  } from "../../lib/lighting/palette";

  interface Props {
    colors: string[];
    onchange: (colors: string[]) => void;
    multi?: boolean;
    /** Whether a color may be a palette slot as well as a literal. The
     *  palette's own colors may not. */
    allowSlots?: boolean;
  }

  let { colors, onchange, multi = true, allowSlots = true }: Props = $props();

  /** The slots the file declares, from the editor above. */
  const paletteSlots = usePalette();
  let slots = $derived(allowSlots ? paletteSlots() : []);
  /** The choice of source is offered when the file has a palette, or a
   *  color already refers to a slot. */
  let offerSlots = $derived(
    allowSlots &&
      (slots.length > 0 || colors.some((c) => slotOfRef(c) !== null)),
  );

  /** Switches color `index` to slot `name`, or to a literal for "". */
  function pickSource(index: number, name: string) {
    if (name !== "") {
      updateColor(index, refOf(name));
      return;
    }
    // Back to a literal: the color the slot was, so nothing jumps.
    updateColor(index, resolveColor(colors[index], slots) ?? "#ffffff");
  }

  /** The colour as the picker shows it: `#rrggbb`, by the parser's own
   *  reading of a hex, `rgb()` or named colour (`rules.color_hex`). */
  function pickerColor(c: string): string {
    return rules.color_hex(c) ?? c;
  }

  function updateColor(index: number, value: string) {
    const updated = [...colors];
    updated[index] = value;
    onchange(updated);
  }

  function removeColor(index: number) {
    onchange(colors.filter((_, i) => i !== index));
  }

  function addColor() {
    onchange([...colors, "#ffffff"]);
  }
</script>

<div class="color-input">
  {#each colors as color, i (i)}
    {@const slot = slotOfRef(color)}
    <div class="color-row">
      {#if offerSlots}
        <select
          class="input color-source"
          data-testid="color-source"
          aria-label={$t("palette.source")}
          value={slot ?? ""}
          onchange={(e) => pickSource(i, (e.target as HTMLSelectElement).value)}
        >
          <option value="">{$t("palette.customColor")}</option>
          {#each slots as s (s.name)}
            <option value={s.name}
              >{$t("palette.slotOption", { values: { slot: s.name } })}</option
            >
          {/each}
          {#if slot !== null && !slots.some((s) => s.name === slot)}
            <option value={slot}
              >{$t("palette.missingSlot", { values: { slot } })}</option
            >
          {/if}
        </select>
      {/if}
      {#if slot !== null}
        <span
          class="color-swatch"
          data-testid="color-slot-swatch"
          style:background={resolveColor(color, slots) ?? "transparent"}
          title={resolveColor(color, slots) ?? ""}
        ></span>
      {:else}
        <input
          type="color"
          class="color-picker"
          value={pickerColor(color)}
          onchange={(e) => updateColor(i, (e.target as HTMLInputElement).value)}
        />
        <input
          type="text"
          class="input color-text"
          value={color}
          onchange={(e) => updateColor(i, (e.target as HTMLInputElement).value)}
          placeholder="#ffffff"
        />
      {/if}
      {#if colors.length > 1}
        <button
          class="btn-icon"
          title={$t("effect.color.removeColor")}
          onclick={() => removeColor(i)}
        >
          &#10005;
        </button>
      {/if}
    </div>
  {/each}
  {#if multi || colors.length === 0}
    <button class="btn btn-sm add-color" onclick={addColor}
      >{$t("effect.color.addColor")}</button
    >
  {/if}
</div>

<style>
  .color-input {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .color-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .color-picker {
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    background: none;
  }
  .color-swatch {
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    flex-shrink: 0;
  }
  .color-source {
    font-size: 13px !important;
    padding: 4px 6px !important;
    max-width: 150px;
  }
  .color-text {
    width: 100px;
    font-size: 13px !important;
    padding: 4px 6px !important;
    font-family: var(--mono);
  }
  .btn-icon {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 12px;
    padding: 2px 4px;
    border-radius: 4px;
  }
  .btn-icon:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  .add-color {
    align-self: flex-start;
    font-size: 12px;
    padding: 2px 8px;
  }
</style>
