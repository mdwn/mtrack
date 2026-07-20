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
<script lang="ts" module>
  export type MetronomeSoundRole = "accent" | "half" | "normal" | "sub";
  export interface MetronomeDefaultSounds {
    accent?: { file?: string; freq?: number; volume?: number };
    half?: { file?: string; freq?: number; volume?: number };
    normal?: { file?: string; freq?: number; volume?: number };
    sub?: { file?: string; freq?: number; volume?: number };
  }
</script>

<script lang="ts">
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { uploadSampleFile } from "../../lib/api/config";
  import NumberStepper from "../NumberStepper.svelte";
  import FileUpload from "../songs/FileUpload.svelte";

  interface Props {
    /** The player-wide `metronome.sounds` block, or null when unset. */
    sounds: MetronomeDefaultSounds | null;
    onchange: () => void;
    onbrowse: (role: MetronomeSoundRole) => void;
  }

  let { sounds = $bindable(), onchange, onbrowse }: Props = $props();

  const ROLES: MetronomeSoundRole[] = ["accent", "half", "normal", "sub"];

  /** Built-in synthesized defaults, used to seed a fresh override. */
  const DEFAULTS: Record<MetronomeSoundRole, { freq: number; volume: number }> =
    {
      accent: { freq: 1600, volume: 1.0 },
      half: { freq: 1400, volume: 0.9 },
      normal: { freq: 1200, volume: 0.8 },
      sub: { freq: 1000, volume: 0.45 },
    };

  let uploading = $state(false);
  let uploadMsg = $state("");

  function updateSound(
    role: MetronomeSoundRole,
    patch: { file?: string; freq?: number; volume?: number },
  ) {
    const next = { ...(sounds ?? {}) };
    const merged = { ...(next[role] ?? {}), ...patch };
    for (const key of ["file", "freq", "volume"] as const) {
      if (merged[key] === undefined || merged[key] === null) {
        delete merged[key];
      }
    }
    next[role] = merged;
    sounds = next;
    onchange();
  }

  /** On = the config overrides this sound; off = built-in synth defaults. */
  function toggleSound(role: MetronomeSoundRole, on: boolean) {
    if (on) {
      updateSound(role, DEFAULTS[role]);
      return;
    }
    const next = { ...(sounds ?? {}) };
    delete next[role];
    sounds = Object.keys(next).length > 0 ? next : null;
    onchange();
  }

  /** Uploads into the global samples directory, like the samples editor. */
  async function handleUpload(role: MetronomeSoundRole, files: File[]) {
    if (files.length === 0) return;
    uploading = true;
    uploadMsg = "";
    try {
      const result = await uploadSampleFile(files[0]);
      updateSound(role, { file: result.path });
      uploadMsg = get(t)("songFile.uploaded", {
        values: { name: files[0].name },
      });
      setTimeout(() => (uploadMsg = ""), 3000);
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : String(e);
    } finally {
      uploading = false;
    }
  }

  export function applyBrowseResult(role: MetronomeSoundRole, path: string) {
    updateSound(role, { file: path });
  }
</script>

<div class="metronome-defaults">
  <p class="muted hint-text">{$t("config.metronomeHint")}</p>
  {#each ROLES as role (role)}
    {@const sound = sounds?.[role]}
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
          <span class="inherited-note">{$t("config.metronomeBuiltin")}</span>
        {/if}
      </label>
      {#if sound}
        <div class="sound-controls">
          <div class="field">
            <span class="field-label">{$t("metronome.volume")}</span>
            <NumberStepper
              value={sound.volume ?? DEFAULTS[role].volume}
              min={0}
              max={2}
              step={0.05}
              decimals={2}
              ariaLabel={$t("metronome.volume")}
              onchange={(v) =>
                updateSound(role, { volume: Math.round(v * 100) / 100 })}
            />
          </div>
          <div class="field">
            <span class="field-label">{$t("metronome.freq")}</span>
            <NumberStepper
              value={sound.freq ?? DEFAULTS[role].freq}
              min={20}
              max={20000}
              step={25}
              suffix="Hz"
              ariaLabel={$t("metronome.freq")}
              onchange={(v) => updateSound(role, { freq: v })}
            />
          </div>
          <div class="field sound-file">
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
            <div class="file-actions">
              <button
                type="button"
                class="btn btn-sm"
                onclick={() => onbrowse(role)}
                >{$t("samples.browseFilesystem")}</button
              >
            </div>
            <FileUpload
              accept=".wav,.flac,.mp3,.ogg,.aac,.m4a,.mp4,.aiff,.aif"
              label={uploading
                ? $t("common.uploading")
                : $t("samples.dropAudio")}
              onupload={(files) => handleUpload(role, files)}
            />
          </div>
        </div>
      {/if}
    </div>
  {/each}
  {#if uploadMsg}
    <div class="upload-msg">{uploadMsg}</div>
  {/if}
</div>

<style>
  .metronome-defaults {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 8px;
    align-items: start;
  }
  .metronome-defaults > .hint-text,
  .metronome-defaults > .upload-msg {
    grid-column: 1 / -1;
  }
  .hint-text {
    font-size: 12px;
    margin: 0 0 4px;
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
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field-label {
    font-size: 11px;
    color: var(--text-muted);
  }
  .sound-file {
    flex: 1;
    min-width: 220px;
    gap: 6px;
  }
  .sound-file .input {
    min-height: 44px;
  }
  .file-actions {
    display: flex;
    gap: 6px;
  }
  .upload-msg {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
