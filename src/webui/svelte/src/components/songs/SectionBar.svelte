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

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
  }

  interface Props {
    sections: SectionEntry[];
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    /** Beat grid measure start times in ms (0-indexed) */
    measureTimesMs: number[];
    /** Total song duration in ms */
    songDurationMs: number;
    onsectionschange: (sections: SectionEntry[]) => void;
  }

  let {
    sections,
    pixelsPerMs,
    scrollLeft,
    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- kept for API consistency
    viewportWidth,
    measureTimesMs,
    songDurationMs,
    onsectionschange,
  }: Props = $props();

  const BAR_HEIGHT = 32;
  const LABEL_WIDTH = 80;
  const HANDLE_WIDTH = 6;

  const SECTION_COLORS = [
    { fill: "rgba(94, 202, 234, 0.25)", border: "rgba(94, 202, 234, 0.6)" },
    {
      fill: "rgba(139, 92, 246, 0.25)",
      border: "rgba(139, 92, 246, 0.6)",
    },
    { fill: "rgba(234, 179, 8, 0.25)", border: "rgba(234, 179, 8, 0.6)" },
    { fill: "rgba(239, 96, 163, 0.25)", border: "rgba(239, 96, 163, 0.6)" },
    { fill: "rgba(34, 197, 94, 0.25)", border: "rgba(34, 197, 94, 0.6)" },
    {
      fill: "rgba(249, 115, 22, 0.25)",
      border: "rgba(249, 115, 22, 0.6)",
    },
  ];

  let selectedIndex = $state<number | null>(null);
  let editingIndex = $state<number | null>(null);
  let editingName = $state("");

  // Convert 1-indexed measure to ms using beat grid.
  function measureToMs(measure: number): number {
    const idx = measure - 1; // 1-indexed → 0-indexed
    if (idx >= 0 && idx < measureTimesMs.length) return measureTimesMs[idx];
    if (idx >= measureTimesMs.length) return songDurationMs;
    return 0;
  }

  // Find nearest measure (1-indexed) for a given ms.
  function snapToMeasure(ms: number): number {
    let closest = 1;
    let closestDist = Infinity;
    for (let i = 0; i < measureTimesMs.length; i++) {
      const dist = Math.abs(measureTimesMs[i] - ms);
      if (dist < closestDist) {
        closestDist = dist;
        closest = i + 1; // 1-indexed
      }
    }
    return closest;
  }

  // Section block positions derived from sections + pixelsPerMs.
  let blocks = $derived(
    sections.map((s, i) => {
      const startMs = measureToMs(s.start_measure);
      const endMs = measureToMs(s.end_measure);
      const left = startMs * pixelsPerMs - scrollLeft;
      const width = (endMs - startMs) * pixelsPerMs;
      const color = SECTION_COLORS[i % SECTION_COLORS.length];
      return { ...s, left, width, startMs, endMs, color, index: i };
    }),
  );

  // Drag state.
  let dragState = $state<{
    index: number;
    edge: "start" | "end" | "move";
    originX: number;
    originMeasure: number;
  } | null>(null);

  // Create state: click-drag on empty area.
  let createState = $state<{
    startMeasure: number;
    currentMeasure: number;
  } | null>(null);

  function handlePointerDown(e: PointerEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    const ms = (scrollLeft + x) / pixelsPerMs;

    // Check if we clicked on an existing section's edge or body.
    for (const block of blocks) {
      const blockLeft = block.left;
      const blockRight = block.left + block.width;

      if (x >= blockLeft - HANDLE_WIDTH && x <= blockLeft + HANDLE_WIDTH) {
        // Left edge drag.
        selectedIndex = block.index;
        dragState = {
          index: block.index,
          edge: "start",
          originX: x,
          originMeasure: block.start_measure,
        };
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        e.preventDefault();
        return;
      }
      if (x >= blockRight - HANDLE_WIDTH && x <= blockRight + HANDLE_WIDTH) {
        // Right edge drag.
        selectedIndex = block.index;
        dragState = {
          index: block.index,
          edge: "end",
          originX: x,
          originMeasure: block.end_measure,
        };
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        e.preventDefault();
        return;
      }
      if (x > blockLeft && x < blockRight) {
        // Click on body — select. Don't preventDefault so dblclick can fire.
        selectedIndex = block.index;
        return;
      }
    }

    // Click on empty area — start creating.
    selectedIndex = null;
    const measure = snapToMeasure(ms);
    createState = { startMeasure: measure, currentMeasure: measure };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function handlePointerMove(e: PointerEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    const ms = (scrollLeft + x) / pixelsPerMs;
    const measure = snapToMeasure(ms);

    if (dragState) {
      const updated = [...sections];
      const section = { ...updated[dragState.index] };
      if (dragState.edge === "start") {
        section.start_measure = Math.min(measure, section.end_measure - 1);
      } else if (dragState.edge === "end") {
        section.end_measure = Math.max(measure, section.start_measure + 1);
      }
      updated[dragState.index] = section;
      onsectionschange(updated);
    } else if (createState) {
      createState = { ...createState, currentMeasure: measure };
    }
  }

  function handlePointerUp() {
    if (createState) {
      const start = Math.min(
        createState.startMeasure,
        createState.currentMeasure,
      );
      const end = Math.max(
        createState.startMeasure,
        createState.currentMeasure,
      );
      if (end > start) {
        const newSection: SectionEntry = {
          name: `section_${sections.length + 1}`,
          start_measure: start,
          end_measure: end,
        };
        onsectionschange([...sections, newSection]);
        selectedIndex = sections.length;
        editingIndex = sections.length;
        editingName = newSection.name;
      }
      createState = null;
    }
    dragState = null;
  }

  function handleDblClick(e: MouseEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    for (const block of blocks) {
      if (x > block.left && x < block.left + block.width) {
        editingIndex = block.index;
        editingName = block.name;
        return;
      }
    }
  }

  function finishRename() {
    if (editingIndex !== null && editingName.trim()) {
      const updated = [...sections];
      updated[editingIndex] = {
        ...updated[editingIndex],
        name: editingName.trim(),
      };
      onsectionschange(updated);
    }
    editingIndex = null;
  }

  function deleteSelected() {
    if (selectedIndex !== null) {
      const updated = sections.filter((_, i) => i !== selectedIndex);
      onsectionschange(updated);
      selectedIndex = null;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Delete" || e.key === "Backspace") {
      if (editingIndex !== null) return; // Don't delete while renaming.
      deleteSelected();
      e.preventDefault();
    }
    if (e.key === "Enter" && editingIndex !== null) {
      finishRename();
      e.preventDefault();
    }
    if (e.key === "Escape") {
      editingIndex = null;
      selectedIndex = null;
    }
  }

  // Create preview block.
  let createPreview = $derived.by(() => {
    if (!createState) return null;
    const start = Math.min(
      createState.startMeasure,
      createState.currentMeasure,
    );
    const end = Math.max(createState.startMeasure, createState.currentMeasure);
    if (end <= start) return null;
    const startMs = measureToMs(start);
    const endMs = measureToMs(end);
    return {
      left: startMs * pixelsPerMs - scrollLeft,
      width: (endMs - startMs) * pixelsPerMs,
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="section-bar" style:height="{BAR_HEIGHT}px">
  <div class="lane-label" style:width="{LABEL_WIDTH}px">
    {$t("songs.detail.sections")}
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="bar-content"
    onpointerdown={handlePointerDown}
    onpointermove={handlePointerMove}
    onpointerup={handlePointerUp}
    ondblclick={handleDblClick}
  >
    {#each blocks as block (block.index)}
      <div
        class="section-block"
        class:selected={selectedIndex === block.index}
        style:left="{block.left}px"
        style:width="{Math.max(block.width, 2)}px"
        style:background={block.color.fill}
        style:border-color={block.color.border}
      >
        {#if editingIndex === block.index}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="section-name-input"
            type="text"
            autofocus
            bind:value={editingName}
            onblur={finishRename}
            onkeydown={(e) => {
              if (e.key === "Enter") finishRename();
            }}
          />
        {:else}
          <span class="section-name">{block.name}</span>
        {/if}
      </div>
    {/each}

    {#if createPreview}
      <div
        class="section-block creating"
        style:left="{createPreview.left}px"
        style:width="{Math.max(createPreview.width, 2)}px"
      ></div>
    {/if}
  </div>
</div>

<style>
  .section-bar {
    display: flex;
    border-bottom: 1px solid var(--border);
    position: sticky;
    left: 0;
  }
  .lane-label {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-dim);
    border-right: 1px solid var(--border);
    font-weight: 600;
  }
  .bar-content {
    flex: 1;
    position: relative;
    overflow: hidden;
    cursor: crosshair;
    background: rgba(255, 255, 255, 0.02);
  }
  .section-block {
    position: absolute;
    top: 3px;
    bottom: 3px;
    border: 1px solid;
    border-radius: 4px;
    display: flex;
    align-items: center;
    padding: 0 6px;
    overflow: hidden;
    cursor: grab;
    transition: box-shadow 0.1s;
    pointer-events: none;
  }
  .section-block.selected {
    box-shadow: 0 0 0 1px var(--accent);
  }
  .section-block.creating {
    background: rgba(255, 255, 255, 0.1);
    border: 1px dashed rgba(255, 255, 255, 0.3);
  }
  .section-name {
    font-size: 10px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .section-name-input {
    font-size: 10px;
    font-weight: 600;
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 2px;
    color: var(--text);
    padding: 1px 4px;
    width: 100%;
    outline: none;
    pointer-events: auto;
  }
</style>
