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
  import type { PlaylistAmbient } from "../../lib/api/config";
  import {
    ambientForMode,
    ambientModeOf,
    ambientTimings,
    withAmbientField,
    type AmbientChoice,
  } from "../../lib/playlistEntry";
  import { t } from "svelte-i18n";

  interface Props {
    /** null inherits, false is none, an object is its own. */
    value: PlaylistAmbient;
    /** Prefix for the inputs' ids: `-mode`, `-show`, `-delay`, ... */
    idPrefix: string;
    label: string;
    hint?: string;
    /** The inherited choice's option label (whose settings apply). */
    inheritLabel: string;
    /** The inherited choice's option value, as the select reads. */
    inheritValue?: string;
    /** Placeholder for each field of its own left empty. */
    inheritPlaceholder: string;
    disabled: boolean;
    onchange: (value: PlaylistAmbient) => void;
  }

  let {
    value,
    idPrefix,
    label,
    hint,
    inheritLabel,
    inheritValue = "inherit",
    inheritPlaceholder,
    disabled,
    onchange,
  }: Props = $props();

  let mode = $derived(ambientModeOf(value));

  function pick(selected: string) {
    const next: AmbientChoice =
      selected === inheritValue
        ? "inherit"
        : selected === "off"
          ? "off"
          : "custom";
    onchange(ambientForMode(next));
  }
</script>

<div class="field">
  <label for={`${idPrefix}-mode`}>{label}</label>
  <select
    id={`${idPrefix}-mode`}
    class="input"
    data-testid={`${idPrefix}-mode`}
    value={mode === "inherit" ? inheritValue : mode}
    {disabled}
    onchange={(e) => pick((e.target as HTMLSelectElement).value)}
  >
    <option value={inheritValue}>{inheritLabel}</option>
    <option value="off">{$t("playlists.ambientOff")}</option>
    <option value="custom">{$t("playlists.ambientCustom")}</option>
  </select>
  {#if hint}
    <span class="field-hint">{hint}</span>
  {/if}
</div>
{#if value}
  <div class="field">
    <label for={`${idPrefix}-show`}>{$t("lighting.ambientShow")}</label>
    <input
      id={`${idPrefix}-show`}
      class="input"
      type="text"
      placeholder={inheritPlaceholder}
      value={value.show ?? ""}
      {disabled}
      onchange={(e) =>
        onchange(
          withAmbientField(
            value,
            "show",
            (e.target as HTMLInputElement).value.trim(),
          ),
        )}
    />
  </div>
  <div class="field-row-3">
    {#each ambientTimings as timing (timing.key)}
      <div class="field">
        <label for={`${idPrefix}-${timing.key}`}>{$t(timing.label)}</label>
        <input
          id={`${idPrefix}-${timing.key}`}
          class="input"
          type="text"
          placeholder={inheritPlaceholder}
          value={value[timing.key] ?? ""}
          {disabled}
          onchange={(e) =>
            onchange(
              withAmbientField(
                value,
                timing.key,
                (e.target as HTMLInputElement).value.trim(),
              ),
            )}
        />
      </div>
    {/each}
  </div>
{/if}
