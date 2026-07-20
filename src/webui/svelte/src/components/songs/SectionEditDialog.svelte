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
  import { SECTION_COLORS, sectionColor } from "../../lib/sectionColors";
  import MarkerDialog from "./MarkerDialog.svelte";

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
    color?: string;
  }

  interface Props {
    section: SectionEntry;
    /** The section's index, for the automatic palette color. */
    index: number;
    onchange: (patch: Partial<SectionEntry>) => void;
    ondelete: () => void;
    onclose: () => void;
  }

  let { section, index, onchange, ondelete, onclose }: Props = $props();

  let autoColor = $derived(sectionColor(undefined, index));
</script>

<MarkerDialog title={$t("sections.dialog.title")} {onclose}>
  <div class="dialog-section">
    <span class="section-label">
      {$t("sections.dialog.name")}
      <span class="range-note"
        >m{section.start_measure}–{section.end_measure}</span
      >
    </span>
    <input
      type="text"
      class="input name-input"
      value={section.name}
      onchange={(e) =>
        onchange({ name: (e.target as HTMLInputElement).value.trim() })}
    />
  </div>

  <div class="dialog-section">
    <span class="section-label">{$t("sections.dialog.color")}</span>
    <div class="swatches">
      <button
        type="button"
        class="swatch swatch--auto"
        class:active={!section.color}
        style:--swatch-color={autoColor}
        title={$t("sections.dialog.colorAuto")}
        onclick={() => onchange({ color: undefined })}
      >
        A
      </button>
      {#each SECTION_COLORS as color (color)}
        <button
          type="button"
          class="swatch"
          class:active={section.color === color}
          style:--swatch-color={color}
          aria-label={color}
          onclick={() => onchange({ color })}
        ></button>
      {/each}
    </div>
  </div>

  {#snippet actions()}
    <button type="button" class="btn btn-danger" onclick={ondelete}
      >{$t("sections.dialog.delete")}</button
    >
  {/snippet}
</MarkerDialog>

<style>
  .dialog-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .section-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    font-weight: 600;
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .range-note {
    font-family: var(--mono);
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-dim);
  }
  .name-input {
    font-size: 16px;
    min-height: 44px;
  }
  .swatches {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  .swatch {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    border: 2px solid transparent;
    background: color-mix(in srgb, var(--swatch-color) 45%, transparent);
    box-shadow: inset 0 0 0 1.5px var(--swatch-color);
    cursor: pointer;
    touch-action: manipulation;
    transition: transform 0.06s;
  }
  .swatch:active {
    transform: scale(0.92);
  }
  .swatch.active {
    border-color: var(--text);
    background: var(--swatch-color);
  }
  .swatch--auto {
    color: var(--text);
    font-size: 13px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
