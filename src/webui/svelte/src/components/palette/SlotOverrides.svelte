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
  import { overrideColor } from "../../lib/palette/resolve";

  /** A slot and the colour it plays in without this layer's override. */
  interface BaseSlot {
    name: string;
    /** `#rrggbb`. */
    color: string;
  }

  interface Props {
    /** The slots to offer, each with the colour it has without an override
     *  from this list: the show's default for a song, the song's colour for
     *  a playlist entry. */
    slots: BaseSlot[];
    /** This layer's overrides, slot to colour text. */
    overrides: Record<string, string>;
    /** Says where the colour shown without an override comes from. */
    baseLabel: string;
    /** Prefix for ids and test ids. */
    idPrefix: string;
    disabled?: boolean;
    onchange: (overrides: Record<string, string>) => void;
  }

  let {
    slots,
    overrides,
    baseLabel,
    idPrefix,
    disabled = false,
    onchange,
  }: Props = $props();

  let declared = $derived(new Set(slots.map((s) => s.name)));
  /** Overrides naming a slot none of the song's shows declares: kept as
   *  written, with a warning, until removed. */
  let unknown = $derived(
    Object.keys(overrides)
      .filter((slot) => !declared.has(slot))
      .sort(),
  );

  function set(slot: string, color: string) {
    onchange({ ...overrides, [slot]: color });
  }

  function clear(slot: string) {
    const next = { ...overrides };
    delete next[slot];
    onchange(next);
  }

  function shown(slot: BaseSlot): string {
    return overrideColor(overrides[slot.name]) ?? slot.color;
  }
</script>

<div class="slot-overrides" data-testid={`${idPrefix}-overrides`}>
  {#each slots as slot (slot.name)}
    {@const overridden = overrides[slot.name] !== undefined}
    <div class="slot-row" data-testid={`${idPrefix}-slot-${slot.name}`}>
      <label class="slot-name" for={`${idPrefix}-${slot.name}`}
        >{slot.name}</label
      >
      <span
        class="swatch"
        style:background={slot.color}
        title={`${baseLabel}: ${slot.color}`}
        role="img"
        aria-label={`${baseLabel}: ${slot.color}`}
      ></span>
      <span class="arrow" aria-hidden="true">&rarr;</span>
      <input
        id={`${idPrefix}-${slot.name}`}
        class="picker"
        class:overridden
        type="color"
        value={shown(slot)}
        {disabled}
        aria-label={$t("palette.overrideFor", { values: { slot: slot.name } })}
        onchange={(e) => set(slot.name, (e.target as HTMLInputElement).value)}
      />
      <span class="value" class:muted={!overridden}>
        {overridden ? shown(slot) : $t("palette.notOverridden")}
      </span>
      {#if overridden}
        <button
          class="btn btn-sm"
          type="button"
          {disabled}
          data-testid={`${idPrefix}-reset-${slot.name}`}
          aria-label={$t("palette.resetSlot", { values: { slot: slot.name } })}
          onclick={() => clear(slot.name)}>{$t("palette.reset")}</button
        >
      {/if}
    </div>
  {/each}
  {#each unknown as slot (slot)}
    <div
      class="slot-row slot-row--unknown"
      data-testid={`${idPrefix}-unknown-${slot}`}
    >
      <span class="slot-name">{slot}</span>
      <span class="warn-badge" title={$t("palette.unknownSlotHint")}
        >{$t("palette.unknownSlot")}</span
      >
      <span class="value">{overrides[slot]}</span>
      <button
        class="btn btn-sm"
        type="button"
        {disabled}
        aria-label={$t("palette.removeOverride", { values: { slot } })}
        onclick={() => clear(slot)}>{$t("common.remove")}</button
      >
    </div>
  {/each}
  {#if slots.length === 0 && unknown.length === 0}
    <p class="empty">{$t("palette.noSlots")}</p>
  {/if}
</div>

<style>
  .slot-overrides {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .slot-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .slot-name {
    min-width: 7rem;
    font-family: var(--nc-font-mono);
    font-size: 13px;
    color: var(--nc-fg-1);
  }
  .swatch {
    display: inline-block;
    width: 22px;
    height: 22px;
    border-radius: 4px;
    border: 1px solid var(--card-border);
    flex-shrink: 0;
  }
  .arrow {
    color: var(--nc-fg-3);
  }
  .picker {
    width: 34px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: none;
    cursor: pointer;
  }
  .picker.overridden {
    border-color: var(--nc-cyan-400);
    box-shadow: 0 0 0 1px var(--nc-cyan-400);
  }
  .picker:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
  .value {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-fg-1);
  }
  .value.muted,
  .empty {
    color: var(--nc-fg-2);
  }
  .empty {
    margin: 0;
    font-size: 13px;
  }
  .warn-badge {
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    padding: 2px 8px;
    border-radius: 999px;
    color: var(--nc-amber-fg);
    border: 1px solid var(--nc-amber-fg);
  }
</style>
