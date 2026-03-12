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

  interface Props {
    files: SongFile[];
    fileType: "audio" | "midi" | "lighting";
    label: string;
    onpick: (filename: string) => void;
  }

  let { files, fileType, label, onpick }: Props = $props();

  let filtered = $derived(files.filter((f) => f.type === fileType));
</script>

{#if filtered.length > 0}
  <div class="file-picker">
    <span class="picker-label">{label}</span>
    <div class="file-list">
      {#each filtered as file (file.name)}
        <button class="file-btn" onclick={() => onpick(file.name)}>
          {file.name}
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .file-picker {
    margin-top: 8px;
  }
  .picker-label {
    display: block;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim);
    margin-bottom: 6px;
  }
  .file-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .file-btn {
    padding: 4px 10px;
    font-size: 12px;
    font-family: var(--mono);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-muted);
    cursor: pointer;
    transition:
      background 0.15s,
      color 0.15s,
      border-color 0.15s;
  }
  .file-btn:hover {
    background: var(--bg-card-hover);
    color: var(--text);
    border-color: var(--text-dim);
  }
</style>
