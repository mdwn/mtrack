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
  import type {
    Cue,
    Sequence,
    TempoSection,
  } from "../../../lib/lighting/types";
  import {
    timestampToMs,
    msToPixel,
  } from "../../../lib/lighting/timeline-state";
  import TimeRuler from "./TimeRuler.svelte";
  import ShowLane from "./ShowLane.svelte";
  import CuePropertiesPanel from "./CuePropertiesPanel.svelte";

  interface Props {
    sequence: Sequence;
    groups: string[];
    sequenceNames: string[];
    tempo?: TempoSection;
    snapEnabled: boolean;
    snapResolution: "beat" | "measure";
    onchange: (sequence: Sequence) => void;
    ondelete: () => void;
    onclose: () => void;
  }

  let {
    sequence,
    groups,
    sequenceNames,
    tempo,
    snapEnabled,
    snapResolution,
    onchange,
    ondelete,
    onclose,
  }: Props = $props();

  let pixelsPerMs = $state(0.3);
  let scrollLeft = $state(0);
  let viewportWidth = $state(600);
  let selectedCueIndex = $state<number | null>(null);
  let scrollContainer: HTMLDivElement | undefined = $state();

  function getDurationMs(): number {
    let max = 0;
    for (const cue of sequence.cues) {
      max = Math.max(max, timestampToMs(cue.timestamp, tempo));
    }
    return Math.max(max + 5000, 15000);
  }

  function getTotalWidthPx(): number {
    return msToPixel(getDurationMs(), pixelsPerMs);
  }

  let scrollRaf = 0;
  function handleScroll() {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      if (scrollContainer) scrollLeft = scrollContainer.scrollLeft;
    });
  }

  function handleCueChange(cueIndex: number, cue: Cue) {
    const cues = [...sequence.cues];
    cues[cueIndex] = cue;
    onchange({ ...sequence, cues });
  }

  function handleCueDelete(cueIndex: number) {
    selectedCueIndex = null;
    onchange({
      ...sequence,
      cues: sequence.cues.filter((_, i) => i !== cueIndex),
    });
  }

  function handleCueAdd(cue: Cue) {
    const cues = [...sequence.cues, cue];
    cues.sort(
      (a, b) =>
        timestampToMs(a.timestamp, tempo) - timestampToMs(b.timestamp, tempo),
    );
    onchange({ ...sequence, cues });
    selectedCueIndex = cues.indexOf(cue);
  }

  function getSelectedCue(): { cue: Cue; laneName: string } | null {
    if (selectedCueIndex === null || selectedCueIndex >= sequence.cues.length)
      return null;
    return { cue: sequence.cues[selectedCueIndex], laneName: sequence.name };
  }

  function handleSelectedCueChange(cue: Cue) {
    if (selectedCueIndex === null) return;
    handleCueChange(selectedCueIndex, cue);
  }

  function handleSelectedCueDelete() {
    if (selectedCueIndex === null) return;
    handleCueDelete(selectedCueIndex);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }

  $effect(() => {
    if (!scrollContainer) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) viewportWidth = entry.contentRect.width;
    });
    ro.observe(scrollContainer);
    return () => ro.disconnect();
  });
</script>

<div
  class="seq-overlay"
  onkeydown={handleKeydown}
  role="dialog"
  aria-label="Sequence editor: {sequence.name}"
  tabindex="-1"
>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="seq-modal"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.stopPropagation()}
    role="document"
  >
    <div class="seq-modal-header">
      <span class="seq-modal-title">Sequence: "{sequence.name}"</span>
      <span class="seq-modal-info">{sequence.cues.length} cues</span>
      <span class="seq-modal-hint"
        >Double-click to add cues. Drag to reposition.</span
      >
      <div class="seq-modal-actions">
        <button class="btn btn-sm btn-danger" onclick={ondelete}
          >Delete Sequence</button
        >
        <button class="btn btn-sm" onclick={onclose}>Close</button>
      </div>
    </div>

    <div class="seq-modal-body">
      <div
        class="seq-scroll"
        bind:this={scrollContainer}
        onscroll={handleScroll}
        role="region"
        aria-label="Sequence cue timeline"
        tabindex="-1"
      >
        <!-- Ruler -->
        <div class="sticky-row seq-ruler-row">
          <div class="seq-label-spacer"></div>
          <div class="seq-canvas-area">
            <TimeRuler
              {pixelsPerMs}
              totalDurationMs={getDurationMs()}
              {tempo}
              {scrollLeft}
              {viewportWidth}
            />
          </div>
        </div>

        <!-- Cue lane -->
        <div class="sticky-row">
          <ShowLane
            name={sequence.name}
            cues={sequence.cues}
            laneType="sequence"
            {pixelsPerMs}
            {scrollLeft}
            {viewportWidth}
            {tempo}
            {selectedCueIndex}
            {snapEnabled}
            {snapResolution}
            onselect={(ci) => (selectedCueIndex = ci)}
            oncuechange={(ci, cue) => handleCueChange(ci, cue)}
            oncuedelete={(ci) => handleCueDelete(ci)}
            oncueadd={(cue) => handleCueAdd(cue)}
            {ondelete}
          />
        </div>

        <div class="scroll-spacer" style:width="{getTotalWidthPx()}px"></div>
      </div>
    </div>

    <div class="seq-modal-detail">
      {#if getSelectedCue()}
        {@const selCue = getSelectedCue()}
        {#if selCue}
          <CuePropertiesPanel
            cue={selCue.cue}
            laneName={selCue.laneName}
            {groups}
            {sequenceNames}
            {tempo}
            onchange={handleSelectedCueChange}
            ondelete={handleSelectedCueDelete}
            onclose={() => (selectedCueIndex = null)}
          />
        {/if}
      {:else}
        <div class="seq-detail-empty">
          Select a cue to edit, or double-click the lane to create one.
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .seq-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    padding: 32px;
  }
  .seq-modal {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    width: 100%;
    max-width: 960px;
    height: 80vh;
    max-height: 600px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .seq-modal-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .seq-modal-title {
    font-size: 14px;
    font-weight: 600;
    color: #a78bfa;
  }
  .seq-modal-info {
    font-size: 12px;
    color: var(--text-muted);
  }
  .seq-modal-hint {
    font-size: 11px;
    color: var(--text-dim);
    flex: 1;
  }
  .seq-modal-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .seq-modal-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: #111;
  }
  .seq-scroll {
    flex: 1;
    overflow-x: auto;
    overflow-y: hidden;
    position: relative;
  }
  .sticky-row {
    position: sticky;
    left: 0;
    width: 100%;
  }
  .seq-ruler-row {
    display: flex;
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
    position: sticky;
    left: 0;
    top: 0;
    z-index: 10;
  }
  .seq-label-spacer {
    width: 80px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
  }
  .seq-canvas-area {
    flex: 1;
    min-width: 0;
  }
  .scroll-spacer {
    height: 1px;
    pointer-events: none;
  }

  .seq-modal-detail {
    flex-shrink: 0;
    height: 200px;
    border-top: 1px solid var(--border);
    background: var(--bg-card);
    overflow: hidden;
  }
  .seq-detail-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-dim);
    font-size: 13px;
  }

  @media (max-width: 768px) {
    .seq-overlay {
      padding: 8px;
    }
    .seq-modal {
      max-width: 100%;
      height: 90vh;
      max-height: none;
    }
  }
</style>
