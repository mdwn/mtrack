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
  import type { SongSummary } from "../../lib/api/songs";

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
  }

  interface Props {
    song: SongSummary;
    sections: SectionEntry[];
    dirty?: boolean;
  }

  let {
    song,
    sections = $bindable([]),
    dirty = $bindable(false), // eslint-disable-line no-useless-assignment -- consumed by parent via bind:dirty
  }: Props = $props();

  function addSection() {
    const num = sections.length + 1;
    sections = [
      ...sections,
      { name: `section_${num}`, start_measure: 1, end_measure: 2 },
    ];
    dirty = true;
  }

  function removeSection(index: number) {
    sections = sections.filter((_, i) => i !== index);
    dirty = true;
  }

  function onChange() {
    dirty = true;
  }

  let measureCount = $derived(
    song.beat_grid ? song.beat_grid.measure_starts.length : 0,
  );
</script>

<div class="sections-editor">
  <div class="section-header">
    <span class="section-title">{$t("songs.detail.sections")}</span>
    <button class="btn btn-sm" onclick={addSection}
      >{$t("songs.detail.addSection")}</button
    >
  </div>

  {#if measureCount > 0}
    <div class="beat-grid-info">
      {measureCount} measures detected from click track
    </div>
  {:else}
    <div class="beat-grid-info warning">
      No beat grid detected. Add a click track named "click" for measure-based
      sections.
    </div>
  {/if}

  {#if sections.length === 0}
    <div class="empty-state">{$t("songs.detail.noSections")}</div>
  {:else}
    <div class="section-list">
      {#each sections as section, i (i)}
        <div class="section-row">
          <div class="section-fields">
            <label class="section-field">
              <span class="field-label">{$t("songs.detail.sectionName")}</span>
              <input
                type="text"
                bind:value={section.name}
                onchange={onChange}
                placeholder="e.g. verse"
              />
            </label>
            <label class="section-field narrow">
              <span class="field-label">{$t("songs.detail.sectionStart")}</span>
              <input
                type="number"
                bind:value={section.start_measure}
                onchange={onChange}
                min="1"
                max={measureCount || undefined}
              />
            </label>
            <label class="section-field narrow">
              <span class="field-label">{$t("songs.detail.sectionEnd")}</span>
              <input
                type="number"
                bind:value={section.end_measure}
                onchange={onChange}
                min="1"
                max={measureCount || undefined}
              />
            </label>
          </div>
          <button class="btn btn-sm btn-danger" onclick={() => removeSection(i)}
            >{$t("common.remove")}</button
          >
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .sections-editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .section-title {
    font-weight: 600;
    font-size: 14px;
  }
  .beat-grid-info {
    font-size: 12px;
    color: var(--text-dim);
  }
  .beat-grid-info.warning {
    color: var(--yellow);
  }
  .empty-state {
    font-size: 13px;
    color: var(--text-dim);
    padding: 12px 0;
  }
  .section-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .section-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 8px;
    background: var(--bg-raised);
    border-radius: 6px;
  }
  .section-fields {
    display: flex;
    gap: 8px;
    flex: 1;
  }
  .section-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }
  .section-field.narrow {
    max-width: 100px;
  }
  .field-label {
    font-size: 11px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .section-field input {
    padding: 4px 8px;
    font-size: 13px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--text);
  }
</style>
