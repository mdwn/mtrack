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
  import { SECTION_COLORS, sectionColor } from "../../lib/sectionColors";

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
    /** 1-based beat within the measure, fractional allowed; absent = 1. */
    start_beat?: number;
    end_beat?: number;
    color?: string;
  }

  interface Props {
    sections: SectionEntry[];
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    /** Beat grid measure start times in ms (0-indexed) */
    measureTimesMs: number[];
    /** Time (ms) of a measure/beat position, for beat-precise boundaries. */
    beatToMs?: (measure: number, beat: number) => number;
    /** Total song duration in ms */
    songDurationMs: number;
    /** Shown centered when there are no sections. */
    emptyHint?: string;
    onsectionschange: (sections: SectionEntry[]) => void;
    /** A section was tapped (without dragging): open its editor. */
    onsectionedit?: (index: number) => void;
  }

  let {
    sections,
    pixelsPerMs,
    scrollLeft,
    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- kept for API consistency
    viewportWidth,
    measureTimesMs,
    beatToMs,
    songDurationMs,
    emptyHint = "",
    onsectionschange,
    onsectionedit,
  }: Props = $props();

  const BAR_HEIGHT = 32;
  const LABEL_WIDTH = 80;
  const HANDLE_WIDTH = 6;

  // Convert 1-indexed measure to ms using beat grid.
  function measureToMs(measure: number): number {
    const idx = measure - 1; // 1-indexed → 0-indexed
    if (idx >= 0 && idx < measureTimesMs.length) return measureTimesMs[idx];
    if (idx >= measureTimesMs.length) return songDurationMs;
    return 0;
  }

  // Snap stride: matches the label stride in SectionRuler so snap targets
  // correspond to visible measure labels. Zoom in for finer control.
  const MIN_LABEL_GAP = 32;
  let snapStride = $derived.by(() => {
    if (measureTimesMs.length < 2) return 1;
    const avgGapMs =
      (measureTimesMs[measureTimesMs.length - 1] - measureTimesMs[0]) /
      (measureTimesMs.length - 1);
    const avgGapPx = avgGapMs * pixelsPerMs;
    let s = 1;
    while (avgGapPx * s < MIN_LABEL_GAP) {
      s *= 2;
    }
    return s;
  });

  // Find nearest measure (1-indexed) at the current snap stride.
  function snapToMeasure(ms: number): number {
    let closest = 1;
    let closestDist = Infinity;
    for (let i = 0; i < measureTimesMs.length; i += snapStride) {
      const dist = Math.abs(measureTimesMs[i] - ms);
      if (dist < closestDist) {
        closestDist = dist;
        closest = i + 1; // 1-indexed
      }
    }
    return closest;
  }

  // A boundary's time: the measure line, or its beat offset within the
  // measure when one is set (beat 1 = the line itself).
  function posToMs(measure: number, beat?: number): number {
    if (beat && beat !== 1 && beatToMs) return beatToMs(measure, beat);
    return measureToMs(measure);
  }

  // Section block positions derived from sections + pixelsPerMs.
  let blocks = $derived(
    sections.map((s, i) => {
      const startMs = posToMs(s.start_measure, s.start_beat);
      const endMs = posToMs(s.end_measure, s.end_beat);
      const left = startMs * pixelsPerMs - scrollLeft;
      const width = (endMs - startMs) * pixelsPerMs;
      const color = sectionColor(s.color, i);
      return { ...s, left, width, startMs, endMs, color, index: i };
    }),
  );

  // Drag state.
  let dragState = $state<{
    index: number;
    edge: "start" | "end" | "move";
    originX: number;
    originMeasure: number;
    /** For move: width of section in measures (preserved during drag) */
    spanMeasures?: number;
    /** Has the pointer moved far enough to count as a drag (vs a tap)? */
    engaged?: boolean;
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
        dragState = {
          index: block.index,
          edge: "start",
          originX: x,
          originMeasure: block.start_measure,
          engaged: false,
        };
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        e.preventDefault();
        return;
      }
      if (x >= blockRight - HANDLE_WIDTH && x <= blockRight + HANDLE_WIDTH) {
        // Right edge drag.
        dragState = {
          index: block.index,
          edge: "end",
          originX: x,
          originMeasure: block.end_measure,
          engaged: false,
        };
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        e.preventDefault();
        return;
      }
      if (x > blockLeft && x < blockRight) {
        // Body — a drag moves the section, a tap opens its editor.
        dragState = {
          index: block.index,
          edge: "move",
          originX: x,
          originMeasure: block.start_measure,
          spanMeasures: block.end_measure - block.start_measure,
          engaged: false,
        };
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        return;
      }
    }

    // Click on empty area — start creating.
    const measure = snapToMeasure(ms);
    createState = { startMeasure: measure, currentMeasure: measure };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function handlePointerMove(e: PointerEvent) {
    updateCursor(e);
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    const ms = (scrollLeft + x) / pixelsPerMs;
    const measure = snapToMeasure(ms);

    if (dragState) {
      // Require a minimum distance before engaging, so a tap stays a tap.
      const DRAG_THRESHOLD = 4; // px
      if (!dragState.engaged) {
        if (Math.abs(x - dragState.originX) < DRAG_THRESHOLD) return;
        dragState = { ...dragState, engaged: true };
      }

      const updated = [...sections];
      const section = { ...updated[dragState.index] };
      if (dragState.edge === "start") {
        // Edge drags snap to measure lines; a beat offset set in the
        // dialog is cleared so the config matches what was dragged.
        section.start_measure = Math.min(measure, section.end_measure - 1);
        delete section.start_beat;
      } else if (dragState.edge === "end") {
        section.end_measure = Math.max(measure, section.start_measure + 1);
        delete section.end_beat;
      } else if (dragState.edge === "move" && dragState.engaged) {
        const span =
          dragState.spanMeasures ?? section.end_measure - section.start_measure;
        // Clamp so section stays within valid measure range.
        const maxStart = measureTimesMs.length - span + 1; // 1-indexed
        const newStart = Math.max(1, Math.min(measure, maxStart));
        section.start_measure = newStart;
        section.end_measure = newStart + span;
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
          // Take the rotation slot as an actual choice, so the color keeps
          // still when sections are added, removed or reordered later.
          color: SECTION_COLORS[sections.length % SECTION_COLORS.length],
        };
        onsectionschange([...sections, newSection]);
        // Straight into the editor to name it.
        onsectionedit?.(sections.length);
      }
      createState = null;
    }
    if (dragState && !dragState.engaged) {
      // A tap (no drag): open the section's editor.
      onsectionedit?.(dragState.index);
    }
    dragState = null;
  }

  // Dynamic cursor based on hover position.
  let cursorStyle = $state("crosshair");

  function updateCursor(e: PointerEvent) {
    if (dragState?.engaged) {
      cursorStyle = "grabbing";
      return;
    }
    if (dragState?.edge === "start" || dragState?.edge === "end") {
      cursorStyle = "ew-resize";
      return;
    }
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    for (const block of blocks) {
      const blockLeft = block.left;
      const blockRight = block.left + block.width;
      if (x >= blockLeft - HANDLE_WIDTH && x <= blockLeft + HANDLE_WIDTH) {
        cursorStyle = "ew-resize";
        return;
      }
      if (x >= blockRight - HANDLE_WIDTH && x <= blockRight + HANDLE_WIDTH) {
        cursorStyle = "ew-resize";
        return;
      }
      if (x > blockLeft && x < blockRight) {
        cursorStyle = "grab";
        return;
      }
    }
    cursorStyle = "crosshair";
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

<div
  class="section-bar"
  style:height="{BAR_HEIGHT}px"
  aria-label={$t("songs.detail.sections", { default: "Sections" })}
>
  <div class="lane-label" style:width="{LABEL_WIDTH}px">
    {$t("songs.detail.sections")}
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="bar-content"
    style:cursor={cursorStyle}
    onpointerdown={handlePointerDown}
    onpointermove={handlePointerMove}
    onpointerup={handlePointerUp}
  >
    {#if sections.length === 0 && emptyHint}
      <span class="bar-empty-hint">{emptyHint}</span>
    {/if}
    {#each blocks as block (block.index)}
      <div
        class="section-block"
        style:left="{block.left}px"
        style:width="{Math.max(block.width, 2)}px"
        style:background="color-mix(in srgb, {block.color} 25%, transparent)"
        style:border-color="color-mix(in srgb, {block.color} 60%, transparent)"
      >
        <span class="section-name">{block.name}</span>
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
  .section-block.creating {
    background: rgba(255, 255, 255, 0.1) !important;
    border: 1px dashed rgba(255, 255, 255, 0.3) !important;
  }
  .bar-empty-hint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    color: var(--text-dim);
    font-style: italic;
    pointer-events: none;
  }
  .section-name {
    font-size: 10px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
