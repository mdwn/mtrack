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
  import type { SongSummary, WaveformTrack } from "../../lib/api/songs";
  import SectionBar from "./SectionBar.svelte";
  import SectionRuler from "./SectionRuler.svelte";
  import SectionWaveformLane from "./SectionWaveformLane.svelte";

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
  }

  interface Props {
    song: SongSummary;
    waveformTracks: WaveformTrack[];
    sections: SectionEntry[];
    dirty?: boolean;
  }

  let {
    song,
    waveformTracks,
    sections = $bindable([]),
    dirty = $bindable(false), // eslint-disable-line no-useless-assignment -- consumed by parent via bind:dirty
  }: Props = $props();

  // Timeline state.
  const MIN_ZOOM = 0.005;
  const MAX_ZOOM = 2;
  let pixelsPerMs = $state(0.15);
  let scrollLeft = $state(0);
  let viewportWidth = $state(800);
  let scrollContainer: HTMLDivElement | undefined = $state();

  // Derived values.
  let songDurationMs = $derived(song.duration_ms);

  let measureTimesMs = $derived.by(() => {
    const grid = song.beat_grid;
    if (!grid) return [];
    return grid.measure_starts.map((beatIdx: number) => {
      return (grid.beats[beatIdx] ?? 0) * 1000;
    });
  });

  const LABEL_WIDTH = 80;
  // The content area width is the viewport minus the label column.
  let contentWidth = $derived(Math.max(0, viewportWidth - LABEL_WIDTH));
  let totalWidthPx = $derived(songDurationMs * pixelsPerMs);

  // Scroll synchronization.
  let scrollRaf = 0;
  function handleScroll() {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      if (scrollContainer) scrollLeft = scrollContainer.scrollLeft;
    });
  }

  // Viewport tracking.
  $effect(() => {
    if (!scrollContainer) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) viewportWidth = entry.contentRect.width;
    });
    ro.observe(scrollContainer);
    return () => ro.disconnect();
  });

  // Zoom with anchor point.
  async function applyZoom(
    newPxPerMs: number,
    anchorMs: number,
    anchorPx: number,
  ) {
    const { tick } = await import("svelte");
    pixelsPerMs = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, newPxPerMs));
    await tick();
    if (scrollContainer) {
      const newScroll = anchorMs * pixelsPerMs - anchorPx;
      scrollContainer.scrollLeft = Math.max(0, newScroll);
      scrollLeft = scrollContainer.scrollLeft;
    }
  }

  function zoomIn() {
    const centerMs = (scrollLeft + contentWidth / 2) / pixelsPerMs;
    applyZoom(pixelsPerMs * 1.3, centerMs, contentWidth / 2);
  }

  function zoomOut() {
    const centerMs = (scrollLeft + contentWidth / 2) / pixelsPerMs;
    applyZoom(pixelsPerMs / 1.3, centerMs, contentWidth / 2);
  }

  function fitView() {
    if (songDurationMs > 0 && contentWidth > 20) {
      pixelsPerMs = contentWidth / songDurationMs;
      if (scrollContainer) scrollContainer.scrollLeft = 0;
      scrollLeft = 0;
    }
  }

  function handleWheel(e: WheelEvent) {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    if (!scrollContainer) return;

    const rect = scrollContainer.getBoundingClientRect();
    const mouseXInViewport = e.clientX - rect.left - LABEL_WIDTH;
    const mouseMs = (scrollLeft + mouseXInViewport) / pixelsPerMs;
    const factor = e.deltaY > 0 ? 1 / 1.15 : 1.15;

    applyZoom(pixelsPerMs * factor, mouseMs, mouseXInViewport);
  }

  function handleSectionsChange(updated: SectionEntry[]) {
    sections = updated;
    dirty = true;
  }

  // Auto fit on mount: wait for the scroll container to be measured.
  let hasFitted = false;
  $effect(() => {
    if (!hasFitted && scrollContainer && songDurationMs > 0) {
      // Use the actual scroll container width, not the default.
      const actualWidth = scrollContainer.clientWidth;
      if (actualWidth > LABEL_WIDTH + 20) {
        viewportWidth = actualWidth;
        hasFitted = true;
        fitView();
      }
    }
  });
</script>

<div class="section-timeline-editor">
  <div class="toolbar">
    <span class="toolbar-title">{$t("songs.detail.sections")}</span>
    <div class="toolbar-controls">
      {#if !song.beat_grid}
        <span class="no-grid-warning"
          >No beat grid — add a click track for measure-based sections</span
        >
      {/if}
      <button class="btn btn-sm" onclick={zoomOut} title="Zoom out">−</button>
      <button class="btn btn-sm" onclick={fitView} title="Fit to view"
        >Fit</button
      >
      <button class="btn btn-sm" onclick={zoomIn} title="Zoom in">+</button>
    </div>
  </div>

  <div
    class="timeline-scroll"
    bind:this={scrollContainer}
    onscroll={handleScroll}
    onwheel={handleWheel}
  >
    <SectionBar
      {sections}
      {pixelsPerMs}
      {scrollLeft}
      {viewportWidth}
      {measureTimesMs}
      {songDurationMs}
      onsectionschange={handleSectionsChange}
    />

    <SectionRuler
      {songDurationMs}
      {pixelsPerMs}
      {scrollLeft}
      {viewportWidth}
      {measureTimesMs}
    />

    {#each waveformTracks as track (track.name)}
      <SectionWaveformLane
        name={track.name}
        peaks={track.peaks}
        {songDurationMs}
        {pixelsPerMs}
        {scrollLeft}
        {viewportWidth}
        {measureTimesMs}
      />
    {/each}

    {#if waveformTracks.length === 0}
      <div class="empty-waveform">
        <span class="muted">No waveform data available</span>
      </div>
    {/if}

    <div
      class="scroll-spacer"
      style:width="{totalWidthPx + LABEL_WIDTH}px"
      style:height="1px"
    ></div>
  </div>

  {#if sections.length > 0}
    <div class="section-list-summary">
      {#each sections as section (section.name)}
        <span class="section-chip">
          {section.name}
          <span class="chip-range"
            >m{section.start_measure}–{section.end_measure}</span
          >
        </span>
      {/each}
    </div>
  {:else if song.beat_grid}
    <div class="hint">
      Drag on the sections bar above to define a section. Sections snap to
      measure boundaries.
    </div>
  {/if}
</div>

<style>
  .section-timeline-editor {
    display: flex;
    flex-direction: column;
    gap: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg);
  }
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-raised);
  }
  .toolbar-title {
    font-weight: 600;
    font-size: 13px;
  }
  .toolbar-controls {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .no-grid-warning {
    font-size: 11px;
    color: var(--yellow);
    margin-right: 8px;
  }
  .timeline-scroll {
    overflow-x: auto;
    overflow-y: hidden;
    position: relative;
    max-height: 400px;
  }
  .scroll-spacer {
    height: 0;
    pointer-events: none;
    flex-shrink: 0;
  }
  .empty-waveform {
    padding: 24px;
    text-align: center;
  }
  .section-list-summary {
    display: flex;
    gap: 6px;
    padding: 8px 12px;
    flex-wrap: wrap;
    border-top: 1px solid var(--border);
  }
  .section-chip {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    color: var(--text);
  }
  .chip-range {
    color: var(--text-dim);
    margin-left: 4px;
    font-family: var(--mono);
  }
  .hint {
    padding: 8px 12px;
    font-size: 12px;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
  }
</style>
