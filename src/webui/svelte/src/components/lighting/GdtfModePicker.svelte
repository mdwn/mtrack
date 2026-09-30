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
  /**
   * The mode picker for a GDTF import (lighting UI design, section 12.2):
   * a filterable list of the archive's modes and, for the one selected,
   * what a show can do in it in plain words, where the fixture sits in the
   * patch, the name and file it will be saved under, and the channel map
   * behind a disclosure.
   */
  import { untrack } from "svelte";
  import { t } from "svelte-i18n";
  import type { GdtfInspection, GdtfMode } from "../../lib/api/config";
  import {
    abilities,
    channelNames,
    extras,
    fileStem,
    filterModes,
    isRefused,
    keepNameChars,
    missingHere,
    safeTypeName,
  } from "../../lib/lighting/modes";

  interface Props {
    inspection: GdtfInspection;
    busy: boolean;
    oncancel: () => void;
    /** Import the chosen mode under this (already safe) type name. */
    onimport: (mode: string, name: string) => void;
  }

  let { inspection, busy, oncancel, onimport }: Props = $props();

  const firstOffered = (modes: GdtfMode[]) =>
    modes.find((m) => !isRefused(m))?.name ?? "";

  let filter = $state("");
  // Seeded once per archive: the panel remounts for the next upload.
  let selected = $state(untrack(() => firstOffered(inspection.modes)));
  let typeName = $state(
    untrack(
      () => inspection.suggested_name ?? safeTypeName(inspection.fixture),
    ),
  );

  const visible = $derived(filterModes(inspection.modes, filter));
  const offered = $derived(visible.filter((m) => !isRefused(m)));
  const mode = $derived(
    inspection.modes.find((m) => m.name === selected && !isRefused(m)) ?? null,
  );
  const words = $derived(mode ? abilities(mode) : []);
  const more = $derived(mode ? extras(mode) : []);
  const missing = $derived(mode ? missingHere(mode, inspection.modes) : []);
  const channels = $derived(mode ? channelNames(mode) : []);
  const saveName = $derived(safeTypeName(typeName));
  const file = $derived(
    `${inspection.fixture_types_dir ?? "lighting/fixture_types"}/${fileStem(saveName)}.fixture`,
  );

  // A filter that hides the selection moves it to the first mode left.
  $effect(() => {
    if (!offered.some((m) => m.name === selected)) {
      selected = offered[0]?.name ?? "";
    }
  });

  function move(delta: number) {
    if (offered.length === 0) return;
    const at = offered.findIndex((m) => m.name === selected);
    const next = Math.min(offered.length - 1, Math.max(0, at + delta));
    selected = offered[next].name;
  }

  function onListKey(e: KeyboardEvent) {
    switch (e.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Home":
        selected = offered[0]?.name ?? selected;
        break;
      case "End":
        selected = offered[offered.length - 1]?.name ?? selected;
        break;
      default:
        return;
    }
    e.preventDefault();
    // Keep the chosen option in view as the arrows walk a long list.
    queueMicrotask(() =>
      document
        .getElementById(optionId(selected))
        ?.scrollIntoView({ block: "nearest" }),
    );
  }

  function onListClick(e: MouseEvent) {
    const option = (e.target as HTMLElement).closest<HTMLElement>(
      "[data-mode]",
    );
    const chosen = inspection.modes.find(
      (m) => m.name === option?.dataset.mode,
    );
    if (chosen && !isRefused(chosen)) selected = chosen.name;
  }

  const optionId = (name: string) =>
    `gdtf-mode-${inspection.modes.findIndex((m) => m.name === name)}`;

  const nameOf = (what: string) => $t(`lighting.gdtf.missing.${what}`);
</script>

<div class="gdtf" data-testid="gdtf-mode-picker">
  <div class="gdtf__head">
    <div>
      <h4 class="gdtf__title">
        {inspection.fixture}
        <span class="gdtf__maker">{inspection.manufacturer}</span>
      </h4>
      <p class="gdtf__count" data-testid="gdtf-mode-count">
        {$t("lighting.gdtf.modeCount", {
          values: { count: inspection.modes.length },
        })}
      </p>
    </div>
    <button class="btn" type="button" onclick={oncancel}
      >{$t("common.cancel")}</button
    >
  </div>

  <div class="gdtf__body">
    <div class="gdtf__modes">
      <label for="gdtf-filter">{$t("lighting.gdtf.filter")}</label>
      <input
        id="gdtf-filter"
        class="input"
        type="search"
        bind:value={filter}
        placeholder={$t("lighting.gdtf.filterPlaceholder")}
      />
      <!-- A listbox: one tab stop, arrows move the choice. -->
      <ul
        class="gdtf__list"
        role="listbox"
        tabindex="0"
        aria-label={$t("lighting.gdtf.modes")}
        aria-activedescendant={selected ? optionId(selected) : undefined}
        onkeydown={onListKey}
        onclick={onListClick}
        data-testid="gdtf-modes"
      >
        {#each visible as m (m.name)}
          {@const refused = isRefused(m)}
          <li
            id={optionId(m.name)}
            role="option"
            class="gdtf__mode"
            class:gdtf__mode--selected={m.name === selected}
            class:gdtf__mode--refused={refused}
            aria-selected={m.name === selected}
            aria-disabled={refused}
            data-mode={m.name}
          >
            <span class="gdtf__mode-name">{m.name}</span>
            <span class="gdtf__mode-meta">
              {$t("lighting.gdtf.addresses", {
                values: { count: m.footprint },
              })}{#if (m.cells ?? 0) > 0}
                · {$t("lighting.gdtf.cellCount", {
                  values: { count: m.cells ?? 0 },
                })}{/if}
            </span>
            {#if refused}
              <span class="gdtf__mode-reason" data-testid="gdtf-refused"
                >{$t("lighting.gdtf.refused", {
                  values: { reason: m.refused ?? "" },
                })}</span
              >
            {/if}
          </li>
        {:else}
          <li class="gdtf__none" role="presentation">
            {$t("lighting.gdtf.noMatch")}
          </li>
        {/each}
      </ul>
    </div>

    <div class="gdtf__detail" aria-live="polite">
      {#if mode}
        <h5 class="gdtf__mode-title">{mode.name}</h5>

        <h6 class="gdtf__sub">{$t("lighting.gdtf.can")}</h6>
        {#if words.length > 0}
          <ul class="gdtf__can" data-testid="gdtf-can">
            {#each words as w (w.key)}
              <li>{$t(`lighting.gdtf.can.${w.key}`, { values: w.params })}</li>
            {/each}
          </ul>
        {:else}
          <p class="gdtf__quiet" data-testid="gdtf-can">
            {$t("lighting.gdtf.canNothing")}
          </p>
        {/if}
        {#if more.length > 0}
          <p class="gdtf__quiet" data-testid="gdtf-extras">
            {$t("lighting.gdtf.extras", { values: { names: more.join(", ") } })}
          </p>
        {/if}

        {#if missing.length > 0}
          <h6 class="gdtf__sub">{$t("lighting.gdtf.notHere")}</h6>
          <ul class="gdtf__missing" data-testid="gdtf-missing">
            {#each missing as m (m.what)}
              <li>
                {$t("lighting.gdtf.missingIn", {
                  values: { what: nameOf(m.what), mode: m.mode },
                })}
              </li>
            {/each}
          </ul>
        {/if}

        <div class="gdtf__strip-wrap">
          {#if mode.footprint <= 64}
            <ol class="gdtf__strip" aria-hidden="true">
              {#each Array.from({ length: mode.footprint }, (_, n) => n) as i (i)}
                <li title={String(i + 1)}>
                  {(mode.channels ?? []).find(([o]) => o === i + 1)?.[1] ?? ""}
                </li>
              {/each}
            </ol>
          {/if}
          <p class="gdtf__strip-text" data-testid="gdtf-address">
            {$t("lighting.gdtf.occupies", {
              values: {
                count: mode.footprint,
                channels: channels.join(", "),
              },
            })}
          </p>
        </div>

        <div class="field">
          <label for="gdtf-name">{$t("lighting.gdtf.typeName")}</label>
          <input
            id="gdtf-name"
            class="input"
            value={typeName}
            oninput={(e) => {
              typeName = keepNameChars(e.currentTarget.value);
              e.currentTarget.value = typeName;
            }}
            data-testid="gdtf-type-name"
          />
          <span class="field-hint" data-testid="gdtf-file"
            >{$t("lighting.gdtf.writes", { values: { file } })}</span
          >
        </div>

        <details class="gdtf__more">
          <summary>{$t("lighting.gdtf.channelMap")}</summary>
          <ul data-testid="gdtf-channels">
            {#each mode.channels ?? [] as [offset, name] (offset)}
              <li>
                {$t("lighting.gdtf.channel", { values: { offset, name } })}
              </li>
            {/each}
          </ul>
          {#if (mode.warnings ?? []).length > 0}
            <p class="field-hint">{$t("lighting.gdtfWarnings")}</p>
            <ul class="gdtf__warnings" data-testid="gdtf-warnings">
              {#each mode.warnings ?? [] as warning (warning)}
                <li>{warning}</li>
              {/each}
            </ul>
          {/if}
        </details>

        <button
          class="btn btn-primary gdtf__add"
          type="button"
          data-testid="gdtf-import-confirm"
          disabled={busy || !saveName}
          onclick={() => onimport(mode.name, saveName)}
        >
          {busy ? $t("common.saving") : $t("lighting.gdtf.add")}
        </button>
      {:else}
        <p class="gdtf__quiet">{$t("lighting.gdtf.pickOne")}</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .gdtf {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .gdtf__head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
  }
  .gdtf__title {
    margin: 0;
    font-size: 16px;
  }
  .gdtf__maker {
    font-weight: 400;
    color: var(--text-muted);
    font-size: 13px;
    margin-left: 6px;
  }
  .gdtf__count {
    margin: 2px 0 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .gdtf__body {
    display: grid;
    grid-template-columns: minmax(200px, 300px) minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }
  .gdtf__modes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .gdtf label {
    font-size: 12px;
    color: var(--text-muted);
  }
  .gdtf__list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 320px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .gdtf__list:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .gdtf__mode {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    cursor: pointer;
    border-bottom: 1px solid var(--border);
  }
  .gdtf__mode:last-child {
    border-bottom: 0;
  }
  .gdtf__mode--selected {
    background: var(--accent-subtle);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .gdtf__mode--refused {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .gdtf__mode-name {
    font-size: 13px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .gdtf__mode-meta,
  .gdtf__mode-reason {
    font-size: 12px;
    color: var(--text-muted);
  }
  .gdtf__none {
    padding: 10px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .gdtf__detail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .gdtf__mode-title {
    margin: 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  .gdtf__sub {
    margin: 4px 0 0;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .gdtf__can,
  .gdtf__missing {
    margin: 0;
    padding-left: 18px;
    font-size: 13px;
  }
  .gdtf__missing {
    color: var(--text-muted);
  }
  .gdtf__quiet {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .gdtf__strip {
    display: flex;
    gap: 2px;
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-x: auto;
  }
  .gdtf__strip li {
    flex: 0 0 auto;
    min-width: 34px;
    padding: 3px 4px;
    border: 1px solid var(--border);
    border-radius: 3px;
    font-size: 11px;
    text-align: center;
    background: var(--bg-card);
  }
  .gdtf__strip-text {
    margin: 4px 0 0;
    font-size: 13px;
  }
  .gdtf__more summary {
    cursor: pointer;
    font-size: 13px;
  }
  .gdtf__more ul {
    margin: 6px 0;
    padding-left: 18px;
    font-size: 12px;
  }
  .gdtf__add {
    align-self: flex-start;
  }
  @media (max-width: 640px) {
    .gdtf__body {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
