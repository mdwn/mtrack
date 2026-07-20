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
  import type {
    MetronomeConfig,
    ClickSoundConfig,
    SongFile,
  } from "../../lib/api/songs";
  import SliderStepper from "../SliderStepper.svelte";
  import { previewClick } from "../../lib/clickPreview";
  import SongFileField from "./SongFileField.svelte";
  import {
    METRONOME_PRESETS,
    SOUND_DEFAULTS,
  } from "../../lib/metronomePresets";

  interface Props {
    /** The song.yaml `metronome:` block, or null when not configured. */
    metronome: MetronomeConfig | null;
    onchange: (metronome: MetronomeConfig | null) => void;
    /** Whether the song has a beat grid (tempo map or analyzed click). */
    hasBeatGrid?: boolean;
    songName?: string;
    /** The song directory listing, for the sound-file pickers. */
    songFiles?: SongFile[];
    /** Opens the filesystem browser for a sound's file. */
    onbrowsesound?: (role: "accent" | "half" | "normal" | "sub") => void;
  }

  let {
    metronome,
    onchange,
    hasBeatGrid = false,
    songName = "",
    songFiles = [],
    onbrowsesound,
  }: Props = $props();

  let expanded = $state(true);

  const ROLES = ["accent", "half", "normal", "sub"] as const;
  type Role = (typeof ROLES)[number];

  const DEFAULTS = SOUND_DEFAULTS;
  const PRESETS = METRONOME_PRESETS;

  function enable() {
    onchange({});
  }

  function disable() {
    onchange(null);
  }

  function update(patch: Partial<MetronomeConfig>) {
    if (!metronome) return;
    const updated = { ...metronome, ...patch };
    // Drop empty/default keys to keep the YAML tidy.
    if (updated.track === "metronome" || updated.track === "") {
      delete updated.track;
    }
    if (updated.accent && updated.accent.length === 0) {
      delete updated.accent;
    }
    if (updated.accents && updated.accents.length === 0) {
      delete updated.accents;
    }
    if (updated.subdivision === 1 || updated.subdivision === undefined) {
      delete updated.subdivision;
    }
    if (updated.volume === 1 || updated.volume === undefined) {
      delete updated.volume;
    }
    if (updated.sounds) {
      for (const key of ROLES) {
        const sound = updated.sounds[key];
        if (sound && Object.keys(sound).length === 0) {
          delete updated.sounds[key];
        }
      }
      if (Object.keys(updated.sounds).length === 0) {
        delete updated.sounds;
      }
    }
    onchange(updated);
  }

  function updateSound(role: Role, patch: Partial<ClickSoundConfig>) {
    if (!metronome) return;
    const sounds = { ...(metronome.sounds ?? {}) };
    const merged: ClickSoundConfig = { ...(sounds[role] ?? {}), ...patch };
    for (const key of ["file", "freq", "volume"] as const) {
      if (merged[key] === undefined || merged[key] === null) {
        delete merged[key];
      }
    }
    sounds[role] = merged;
    update({ sounds });
  }

  /** On = the song overrides this sound; off = player defaults apply. */
  function toggleSound(role: Role, on: boolean) {
    if (!metronome) return;
    if (on) {
      updateSound(role, DEFAULTS[role]);
    } else {
      const sounds = { ...(metronome.sounds ?? {}) };
      delete sounds[role];
      update({ sounds });
    }
  }

  function applyPreset(sounds: NonNullable<MetronomeConfig["sounds"]>) {
    update({ sounds: structuredClone(sounds) });
  }
</script>

<div class="metronome-editor">
  <div class="metronome-header">
    <button class="expand-btn" onclick={() => (expanded = !expanded)}>
      {expanded ? "▼" : "▶"}
    </button>
    <span class="section-title">{$t("metronome.title")}</span>
    {#if metronome}
      <span class="metronome-info">
        {metronome.track ?? "metronome"}
        {#if metronome.volume !== undefined && metronome.volume !== 1}
          · ×{metronome.volume}
        {/if}
      </span>
      <button class="btn btn-sm btn-danger" onclick={disable}
        >{$t("common.remove")}</button
      >
    {:else}
      <button class="btn btn-sm btn-primary" onclick={enable}
        >{$t("metronome.add")}</button
      >
    {/if}
    {#if metronome && !hasBeatGrid}
      <span class="no-grid-warning">{$t("metronome.noBeatGrid")}</span>
    {/if}
  </div>

  {#if metronome && expanded}
    <div class="metronome-body">
      <div class="level-row">
        <div class="field level-field">
          <span class="field-label">{$t("metronome.level")}</span>
          <SliderStepper
            value={metronome.volume ?? 1}
            min={0}
            max={2}
            step={0.05}
            decimals={2}
            suffix="×"
            defaultValue={1}
            ariaLabel={$t("metronome.level")}
            onchange={(v) => update({ volume: Math.round(v * 100) / 100 })}
          />
        </div>
        <label class="field track-field">
          <span class="field-label">{$t("metronome.track")}</span>
          <input
            type="text"
            class="input"
            placeholder="metronome"
            value={metronome.track ?? ""}
            onchange={(e) =>
              update({ track: (e.target as HTMLInputElement).value.trim() })}
          />
        </label>
      </div>

      <div class="presets">
        <span class="field-label">{$t("metronome.presets")}</span>
        {#each PRESETS as preset (preset.key)}
          <button
            class="btn btn-sm"
            onclick={() => applyPreset(preset.sounds)}
            title="{preset.sounds.accent?.freq}/{preset.sounds.normal?.freq} Hz"
          >
            {$t(`metronome.preset.${preset.key}`)}
          </button>
        {/each}
      </div>

      <div class="sounds">
        {#each ROLES as role (role)}
          {@const sound = metronome.sounds?.[role]}
          <div class="sound-row" class:sound-row--off={!sound}>
            <label class="toggle-row">
              <input
                type="checkbox"
                checked={!!sound}
                onchange={(e) =>
                  toggleSound(role, (e.target as HTMLInputElement).checked)}
              />
              <span class="sound-label">{$t(`metronome.sound.${role}`)}</span>
              {#if !sound}
                <span class="inherited-note">{$t("metronome.inherited")}</span>
              {/if}
              {#if sound && !sound.file}
                <button
                  type="button"
                  class="preview-click-btn"
                  title={$t("metronome.previewClick")}
                  onclick={(e) => {
                    e.preventDefault();
                    previewClick(
                      sound.freq ?? DEFAULTS[role].freq,
                      sound.volume ?? DEFAULTS[role].volume,
                    );
                  }}>🔊</button
                >
              {/if}
            </label>
            {#if sound}
              <div class="sound-controls">
                <div class="field">
                  <span class="field-label">{$t("metronome.volume")}</span>
                  <SliderStepper
                    value={sound.volume ?? DEFAULTS[role].volume}
                    min={0}
                    max={2}
                    step={0.05}
                    decimals={2}
                    defaultValue={DEFAULTS[role].volume}
                    ariaLabel={$t("metronome.volume")}
                    onchange={(v) =>
                      updateSound(role, { volume: Math.round(v * 100) / 100 })}
                  />
                </div>
                <div class="field">
                  <span class="field-label">{$t("metronome.freq")}</span>
                  <SliderStepper
                    value={sound.freq ?? DEFAULTS[role].freq}
                    min={20}
                    max={20000}
                    step={25}
                    suffix="Hz"
                    log
                    defaultValue={DEFAULTS[role].freq}
                    ariaLabel={$t("metronome.freq")}
                    onchange={(v) => updateSound(role, { freq: v })}
                  />
                </div>
                <div class="field sound-file">
                  <span class="field-label">{$t("metronome.file")}</span>
                  <SongFileField
                    value={sound.file ?? ""}
                    placeholder={$t("metronome.filePlaceholder")}
                    {songName}
                    files={songFiles}
                    onchange={(path) => updateSound(role, { file: path })}
                    onbrowse={onbrowsesound
                      ? () => onbrowsesound(role)
                      : undefined}
                  />
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
      <p class="muted hint-text">{$t("metronome.hint")}</p>
    </div>
  {/if}
</div>

<style>
  .metronome-editor {
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }
  .metronome-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
  }
  .expand-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 11px;
    padding: 2px;
  }
  .section-title {
    font-weight: 600;
    font-size: 13px;
  }
  .metronome-info {
    color: var(--text-muted);
    font-size: 12px;
    flex: 1;
  }
  .no-grid-warning {
    color: var(--warning, #e8a54b);
    font-size: 12px;
  }
  .metronome-body {
    padding: 0 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .level-row {
    display: flex;
    align-items: flex-end;
    gap: 20px;
    flex-wrap: wrap;
  }
  .level-field {
    flex: 1;
    min-width: 260px;
  }
  .track-field {
    min-width: 160px;
  }
  .track-field .input {
    min-height: 44px;
  }
  .presets {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field-label {
    font-size: 11px;
    color: var(--text-muted);
  }
  .sounds {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    align-items: start;
  }
  @media (max-width: 720px) {
    .sounds {
      grid-template-columns: 1fr;
    }
  }
  .sound-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .sound-row--off {
    padding: 6px 10px;
  }
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
    min-height: 24px;
  }
  .preview-click-btn {
    margin-left: auto;
    background: none;
    border: 1px solid var(--border);
    border-radius: 8px;
    min-width: 36px;
    min-height: 30px;
    cursor: pointer;
    font-size: 13px;
  }
  .preview-click-btn:active {
    background: var(--accent);
  }
  .toggle-row input[type="checkbox"] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
    margin: 0;
  }
  .sound-label {
    font-size: 12px;
    font-weight: 600;
    min-width: 60px;
  }
  .inherited-note {
    font-size: 11px;
    color: var(--text-dim);
    font-style: italic;
  }
  .sound-controls {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }
  .sound-file {
    flex: 1;
    min-width: 180px;
  }
  .hint-text {
    font-size: 12px;
    margin: 0;
  }
</style>
