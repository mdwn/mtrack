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
  interface Props {
    accept: string;
    label: string;
    multiple?: boolean;
    onupload: (files: File[]) => void;
  }

  let { accept, label, multiple = false, onupload }: Props = $props();
  let dragover = $state(false);
  let inputEl: HTMLInputElement | undefined = $state();

  function handleFiles(fileList: FileList | null) {
    if (!fileList || fileList.length === 0) return;
    onupload(Array.from(fileList));
    if (inputEl) inputEl.value = "";
  }

  function ondrop(e: DragEvent) {
    e.preventDefault();
    dragover = false;
    handleFiles(e.dataTransfer?.files ?? null);
  }

  function ondragover(e: DragEvent) {
    e.preventDefault();
    dragover = true;
  }

  function ondragleave() {
    dragover = false;
  }

  function onclick() {
    inputEl?.click();
  }
</script>

<button
  class="drop-zone"
  class:dragover
  {ondrop}
  {ondragover}
  {ondragleave}
  {onclick}
  type="button"
>
  <span class="drop-label">{label}</span>
  <input
    bind:this={inputEl}
    type="file"
    {accept}
    {multiple}
    onchange={(e) => handleFiles(e.currentTarget.files)}
    class="hidden-input"
  />
</button>

<style>
  .drop-zone {
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--border);
    border-radius: var(--radius);
    padding: 20px;
    cursor: pointer;
    transition:
      border-color 0.15s,
      background 0.15s;
    background: transparent;
    width: 100%;
    font-family: var(--sans);
  }
  .drop-zone:hover,
  .drop-zone.dragover {
    border-color: var(--accent);
    background: rgba(91, 91, 214, 0.05);
  }
  .drop-label {
    font-size: 13px;
    color: var(--text-muted);
  }
  .hidden-input {
    display: none;
  }
</style>
