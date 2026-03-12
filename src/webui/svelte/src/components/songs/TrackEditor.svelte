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
  import type { SongFile } from "../../lib/api/songs";
  import type { WaveformTrack } from "../../lib/api/songs";
  import Waveform from "./Waveform.svelte";

  interface TrackEntry {
    name: string;
    file: string;
    file_channel?: number;
  }

  interface Props {
    tracks: TrackEntry[];
    files: SongFile[];
    waveformTracks: WaveformTrack[];
    onchange: (tracks: TrackEntry[]) => void;
    /** Open a file browser for the given track index (or -1 for "add new"). */
    onbrowse: (trackIndex: number) => void;
  }

  let { tracks, files, waveformTracks, onchange, onbrowse }: Props = $props();

  let audioFiles = $derived(files.filter((f) => f.type === "audio"));

  function updateTrack(
    index: number,
    field: keyof TrackEntry,
    value: string | number | undefined,
  ) {
    const updated = tracks.map((t, i) =>
      i === index ? { ...t, [field]: value } : t,
    );
    onchange(updated);
  }

  function removeTrack(index: number) {
    onchange(tracks.filter((_, i) => i !== index));
  }

  function addTrack() {
    const file = audioFiles.length > 0 ? audioFiles[0].name : "";
    onchange([...tracks, { name: file.replace(/\.[^.]+$/, ""), file }]);
  }

  function peaksForTrack(name: string): number[] {
    return waveformTracks.find((t) => t.name === name)?.peaks ?? [];
  }
</script>

<div class="track-editor">
  {#if tracks.length === 0}
    <div class="empty">No tracks configured</div>
  {/if}
  {#each tracks as track, i (i)}
    <div class="track-row">
      <div class="track-fields">
        <label class="field">
          <span class="field-label">Name</span>
          <input
            class="input field-input"
            type="text"
            value={track.name}
            oninput={(e) => updateTrack(i, "name", e.currentTarget.value)}
          />
        </label>
        <div class="field">
          <span class="field-label">File</span>
          <div class="file-row">
            {#if audioFiles.length > 0 && !track.file.includes("/")}
              <select
                class="input field-input"
                value={track.file}
                onchange={(e) => updateTrack(i, "file", e.currentTarget.value)}
              >
                {#each audioFiles as f (f.name)}
                  <option value={f.name}>{f.name}</option>
                {/each}
                {#if track.file && !audioFiles.some((f) => f.name === track.file)}
                  <option value={track.file}>{track.file} (missing)</option>
                {/if}
              </select>
            {:else}
              <input
                class="input field-input"
                type="text"
                value={track.file}
                title={track.file}
                oninput={(e) => updateTrack(i, "file", e.currentTarget.value)}
              />
            {/if}
            <button
              class="btn browse-btn"
              onclick={() => onbrowse(i)}
              title="Browse filesystem">...</button
            >
          </div>
        </div>
        <label class="field field-narrow">
          <span class="field-label">Channel</span>
          <input
            class="input field-input"
            type="number"
            min="1"
            value={track.file_channel ?? 1}
            oninput={(e) => {
              const v = parseInt(e.currentTarget.value);
              updateTrack(i, "file_channel", isNaN(v) ? undefined : v);
            }}
          />
        </label>
        <button
          class="btn btn-danger remove-btn"
          onclick={() => removeTrack(i)}
          title="Remove track">&times;</button
        >
      </div>
      <div class="track-waveform">
        <Waveform peaks={peaksForTrack(track.name)} height={24} />
      </div>
    </div>
  {/each}
  <div class="add-actions">
    <button class="btn" onclick={addTrack}>+ Add Track</button>
    <button class="btn" onclick={() => onbrowse(-1)}
      >Browse Filesystem...</button
    >
  </div>
</div>

<style>
  .track-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .empty {
    font-size: 13px;
    color: var(--text-dim);
    padding: 4px 0;
  }
  .track-row {
    padding: 8px;
    border-radius: var(--radius);
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid var(--border);
  }
  .track-fields {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }
  .field {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .field-narrow {
    flex: 0 0 70px;
  }
  .field-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim);
    margin-bottom: 3px;
  }
  .field-input {
    width: 100%;
    font-size: 12px;
    padding: 4px 8px;
  }
  .file-row {
    display: flex;
    gap: 4px;
  }
  .file-row .field-input {
    flex: 1;
    min-width: 0;
  }
  .browse-btn {
    padding: 4px 8px;
    font-size: 12px;
    flex: 0 0 auto;
  }
  select.field-input {
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'%3E%3Cpath fill='%2371717a' d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 6px center;
    padding-right: 22px;
  }
  .remove-btn {
    flex: 0 0 auto;
    padding: 4px 8px;
    font-size: 16px;
    line-height: 1;
  }
  .track-waveform {
    margin-top: 6px;
  }
  .add-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  @media (max-width: 600px) {
    .track-fields {
      flex-wrap: wrap;
    }
    .field {
      flex: 1 1 100%;
    }
    .field-narrow {
      flex: 0 0 70px;
    }
  }
</style>
