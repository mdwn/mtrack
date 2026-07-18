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
  import type { MetronomeConfig, ClickSoundConfig } from "../../lib/api/songs";

  interface Props {
    /** The song.yaml `metronome:` block, or null when not configured. */
    metronome: MetronomeConfig | null;
    onchange: (metronome: MetronomeConfig | null) => void;
    /** Whether the song has a beat grid (tempo map or analyzed click). */
    hasBeatGrid?: boolean;
  }

  let { metronome, onchange, hasBeatGrid = false }: Props = $props();

  let expanded = $state(true);

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
    if (updated.sounds) {
      for (const key of ["accent", "normal"] as const) {
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

  function updateSound(
    role: "accent" | "normal",
    patch: Partial<ClickSoundConfig>,
  ) {
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

  function parseAccent(raw: string): number[] {
    return raw
      .split(/[\s,]+/)
      .map((part) => parseInt(part))
      .filter((n) => Number.isFinite(n) && n > 0);
  }

  const DEFAULTS = {
    accent: { freq: 1600, volume: 1.0 },
    normal: { freq: 1200, volume: 0.8 },
  };
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
        {#if metronome.accent?.length}
          · {metronome.accent.join("+")}
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
      <div class="metronome-fields">
        <label class="field">
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
        <label class="field">
          <span class="field-label">{$t("metronome.accentPattern")}</span>
          <input
            type="text"
            class="input"
            placeholder="e.g. 3, 2, 2"
            value={(metronome.accent ?? []).join(", ")}
            onchange={(e) =>
              update({
                accent: parseAccent((e.target as HTMLInputElement).value),
              })}
          />
        </label>
      </div>
      <div class="sounds">
        {#each ["accent", "normal"] as const as role (role)}
          {@const sound = metronome.sounds?.[role] ?? {}}
          <div class="sound-row">
            <span class="field-label sound-label"
              >{$t(`metronome.sound.${role}`)}</span
            >
            <label class="field">
              <span class="field-label">{$t("metronome.freq")}</span>
              <input
                type="number"
                class="input sm-input"
                min="20"
                max="20000"
                placeholder={String(DEFAULTS[role].freq)}
                value={sound.freq ?? ""}
                onchange={(e) => {
                  const v = (e.target as HTMLInputElement).value;
                  updateSound(role, { freq: v ? parseFloat(v) : undefined });
                }}
              />
            </label>
            <label class="field">
              <span class="field-label">{$t("metronome.volume")}</span>
              <input
                type="number"
                class="input sm-input"
                min="0"
                max="2"
                step="0.05"
                placeholder={String(DEFAULTS[role].volume)}
                value={sound.volume ?? ""}
                onchange={(e) => {
                  const v = (e.target as HTMLInputElement).value;
                  updateSound(role, { volume: v ? parseFloat(v) : undefined });
                }}
              />
            </label>
            <label class="field sound-file">
              <span class="field-label">{$t("metronome.file")}</span>
              <input
                type="text"
                class="input"
                placeholder={$t("metronome.filePlaceholder")}
                value={sound.file ?? ""}
                onchange={(e) => {
                  const v = (e.target as HTMLInputElement).value.trim();
                  updateSound(role, { file: v || undefined });
                }}
              />
            </label>
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
    gap: 10px;
  }
  .metronome-fields {
    display: flex;
    gap: 12px;
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
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sound-row {
    display: flex;
    align-items: flex-end;
    gap: 12px;
    flex-wrap: wrap;
  }
  .sound-label {
    min-width: 60px;
    padding-bottom: 8px;
    font-weight: 600;
  }
  .sm-input {
    width: 90px;
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
