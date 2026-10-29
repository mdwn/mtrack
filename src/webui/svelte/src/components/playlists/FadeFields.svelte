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
  import type { FadeConfig } from "../../lib/api/config";
  import { t } from "svelte-i18n";

  interface Props {
    /** Which fade: only a fade out crosses the lights. */
    kind: "fade_out" | "fade_in";
    value: FadeConfig | null;
    /** The song's tracks, to pick the ones that do not fade. */
    tracks: string[];
    disabled: boolean;
    /** Unique per row, for the inputs' ids. */
    idPrefix: string;
    onchange: (value: FadeConfig | null) => void;
  }

  let { kind, value, tracks, disabled, idPrefix, onchange }: Props = $props();

  type Full = Exclude<FadeConfig, string>;

  let full = $derived<Full | null>(
    value === null
      ? null
      : typeof value === "string"
        ? { length: value }
        : value,
  );

  // Tracks named in the file but gone from the song still show, so they can
  // be unticked.
  let trackChoices = $derived([
    ...tracks,
    ...(full?.exclude ?? []).filter((name) => !tracks.includes(name)),
  ]);

  /** The fade as written: its length alone when it has no other settings. */
  function emit(next: Full | null) {
    if (!next || !next.length) {
      onchange(null);
      return;
    }
    const settings: Full = { length: next.length };
    if (next.curve) settings.curve = next.curve;
    if (next.exclude && next.exclude.length) settings.exclude = next.exclude;
    if (kind === "fade_out" && next.lights) settings.lights = next.lights;
    onchange(Object.keys(settings).length === 1 ? settings.length : settings);
  }

  function setLength(length: string) {
    emit(length ? { ...(full ?? {}), length } : null);
  }

  function setCurve(curve: string) {
    if (!full) return;
    emit({
      ...full,
      curve: curve === "linear" ? "linear" : undefined,
    });
  }

  function setLights(lights: string) {
    if (!full) return;
    emit({ ...full, lights: lights || undefined });
  }

  function toggleExclude(track: string, on: boolean) {
    if (!full) return;
    const current = full.exclude ?? [];
    const exclude = on
      ? [...current.filter((name) => name !== track), track]
      : current.filter((name) => name !== track);
    emit({ ...full, exclude });
  }
</script>

<fieldset class="fade-fields" data-testid={`${kind}-fields`}>
  <legend>
    {kind === "fade_out" ? $t("playlists.fadeOut") : $t("playlists.fadeIn")}
  </legend>
  <span class="field-hint"
    >{kind === "fade_out"
      ? $t("playlists.fadeOutHint")
      : $t("playlists.fadeInHint")}</span
  >
  <div class="fade-row">
    <div class="field">
      <label for={`${idPrefix}-${kind}-length`}
        >{$t("playlists.fadeLength")}</label
      >
      <input
        id={`${idPrefix}-${kind}-length`}
        class="input"
        type="text"
        data-testid={`${kind}-length`}
        placeholder={$t("playlists.fadeLengthPlaceholder")}
        value={full?.length ?? ""}
        {disabled}
        onchange={(e) => setLength((e.target as HTMLInputElement).value.trim())}
      />
    </div>
    <div class="field">
      <label for={`${idPrefix}-${kind}-curve`}
        >{$t("playlists.fadeCurve")}</label
      >
      <select
        id={`${idPrefix}-${kind}-curve`}
        class="input"
        data-testid={`${kind}-curve`}
        value={full?.curve ?? "equal_power"}
        disabled={disabled || !full}
        onchange={(e) => setCurve((e.target as HTMLSelectElement).value)}
      >
        <option value="equal_power"
          >{$t("playlists.fadeCurveEqualPower")}</option
        >
        <option value="linear">{$t("playlists.fadeCurveLinear")}</option>
      </select>
    </div>
    {#if kind === "fade_out"}
      <div class="field">
        <label for={`${idPrefix}-${kind}-lights`}
          >{$t("playlists.fadeLights")}</label
        >
        <input
          id={`${idPrefix}-${kind}-lights`}
          class="input"
          type="text"
          data-testid={`${kind}-lights`}
          placeholder={$t("playlists.fadeLightsPlaceholder")}
          value={full?.lights ?? ""}
          disabled={disabled || !full}
          onchange={(e) =>
            setLights((e.target as HTMLInputElement).value.trim())}
        />
      </div>
    {/if}
  </div>
  {#if trackChoices.length > 0}
    <div class="fade-exclude">
      <span class="field-label">{$t("playlists.fadeExclude")}</span>
      {#each trackChoices as track (track)}
        <label class="checkbox-row">
          <input
            type="checkbox"
            data-testid={`${kind}-exclude`}
            value={track}
            checked={full?.exclude?.includes(track) ?? false}
            disabled={disabled || !full}
            onchange={(e) =>
              toggleExclude(track, (e.target as HTMLInputElement).checked)}
          />
          {track}
        </label>
      {/each}
      <span class="field-hint"
        >{kind === "fade_out"
          ? $t("playlists.fadeOutExcludeHint")
          : $t("playlists.fadeInExcludeHint")}</span
      >
    </div>
  {/if}
</fieldset>

<style>
  .fade-fields {
    border: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  legend {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 13px;
    color: var(--nc-fg-1);
    padding: 0;
  }
  .fade-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
    gap: 8px;
  }
  .fade-exclude {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 12px;
  }
  .fade-exclude .field-hint {
    flex-basis: 100%;
  }
  .checkbox-row {
    font-size: 13px;
  }
  .field-hint {
    font-size: 12px;
    color: var(--nc-fg-3);
  }
</style>
