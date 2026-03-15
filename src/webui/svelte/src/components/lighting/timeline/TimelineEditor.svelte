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
  import type { LightFile, Cue, Sequence } from "../../../lib/lighting/types";
  import type { WaveformTrack } from "../../../lib/api/songs";
  import {
    timestampToMs,
    msToPixel,
  } from "../../../lib/lighting/timeline-state";
  import TimelineToolbar from "./TimelineToolbar.svelte";
  import TimeRuler from "./TimeRuler.svelte";
  import WaveformLane from "./WaveformLane.svelte";
  import ShowLane from "./ShowLane.svelte";
  import CuePropertiesPanel from "./CuePropertiesPanel.svelte";
  import SequenceEditorModal from "./SequenceEditorModal.svelte";

  interface Props {
    lightFile: LightFile;
    groups: string[];
    sequenceNames: string[];
    songDurationMs: number;
    waveformTracks: WaveformTrack[];
    onchange: (lightFile: LightFile) => void;
  }

  let {
    lightFile,
    groups,
    sequenceNames,
    songDurationMs,
    waveformTracks,
    onchange,
  }: Props = $props();

  // Timeline state
  let pixelsPerMs = $state(0.15);
  let scrollLeft = $state(0);
  let viewportWidth = $state(800);
  let cursorMs = $state<number | null>(null);
  let snapEnabled = $state(true);
  let snapResolution = $state<"beat" | "measure">("beat");

  // Selection
  let selectedShowIndex = $state<number | null>(null);
  let selectedCueIndex = $state<number | null>(null);

  // Sequence editing modal
  let editingSequenceIndex = $state<number | null>(null);

  let scrollContainer: HTMLDivElement | undefined = $state();

  let totalDurationMs = $derived(
    Math.max(songDurationMs || 60000, getMaxShowCueTime(lightFile) + 5000),
  );

  function getTotalWidthPx(): number {
    return msToPixel(totalDurationMs, pixelsPerMs);
  }

  // Merge waveform peaks — cached
  let cachedPeaks: number[] = [];
  let cachedPeaksTracks: WaveformTrack[] = [];
  function getMergedPeaks(): number[] {
    if (waveformTracks === cachedPeaksTracks) return cachedPeaks;
    cachedPeaksTracks = waveformTracks;
    if (waveformTracks.length === 0) {
      cachedPeaks = [];
      return cachedPeaks;
    }
    if (waveformTracks.length === 1) {
      cachedPeaks = waveformTracks[0].peaks;
      return cachedPeaks;
    }
    const len = Math.max(...waveformTracks.map((t) => t.peaks.length));
    const merged = new Array(len).fill(0);
    for (let i = 0; i < len; i++) {
      let sum = 0,
        count = 0;
      for (const track of waveformTracks) {
        if (i < track.peaks.length) {
          sum += track.peaks[i];
          count++;
        }
      }
      merged[i] = count > 0 ? sum / count : 0;
    }
    cachedPeaks = merged;
    return cachedPeaks;
  }

  function getSelectedCue(): { cue: Cue; laneName: string } | null {
    if (selectedShowIndex === null || selectedCueIndex === null) return null;
    const show = lightFile.shows[selectedShowIndex];
    if (!show || selectedCueIndex >= show.cues.length) return null;
    return { cue: show.cues[selectedCueIndex], laneName: show.name };
  }

  function getMaxShowCueTime(lf: LightFile): number {
    let max = 0;
    for (const show of lf.shows) {
      for (const cue of show.cues) {
        max = Math.max(max, timestampToMs(cue.timestamp, lf.tempo));
      }
    }
    return max;
  }

  // --- Scroll ---
  let scrollRaf = 0;
  function handleScroll() {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      if (scrollContainer) scrollLeft = scrollContainer.scrollLeft;
    });
  }

  function handleZoom(newPixelsPerMs: number) {
    const centerMs = (scrollLeft + viewportWidth / 2) / pixelsPerMs;
    pixelsPerMs = newPixelsPerMs;
    if (scrollContainer) {
      scrollContainer.scrollLeft =
        centerMs * newPixelsPerMs - viewportWidth / 2;
    }
  }

  function fitView() {
    if (totalDurationMs > 0 && viewportWidth > 100) {
      pixelsPerMs = (viewportWidth - 100) / totalDurationMs;
      if (scrollContainer) scrollContainer.scrollLeft = 0;
      scrollLeft = 0;
    }
  }

  function handleMouseMove(e: MouseEvent) {
    if (!scrollContainer) return;
    const rect = scrollContainer.getBoundingClientRect();
    const x = e.clientX - rect.left - 80 + scrollLeft;
    cursorMs = Math.max(0, x / pixelsPerMs);
  }

  function handleMouseLeave() {
    cursorMs = null;
  }

  // --- Show CRUD ---
  function addShow() {
    const name = prompt("Show name:");
    if (!name) return;
    onchange({ ...lightFile, shows: [...lightFile.shows, { name, cues: [] }] });
  }

  function deleteShow(index: number) {
    if (!confirm(`Delete show "${lightFile.shows[index].name}"?`)) return;
    clearSelection();
    onchange({
      ...lightFile,
      shows: lightFile.shows.filter((_, i) => i !== index),
    });
  }

  // --- Sequence CRUD ---
  function addSequence() {
    const name = prompt("Sequence name:");
    if (!name) return;
    const newSeqs = [...lightFile.sequences, { name, cues: [] }];
    onchange({ ...lightFile, sequences: newSeqs });
    editingSequenceIndex = newSeqs.length - 1;
  }

  function deleteSequence(index: number) {
    if (!confirm(`Delete sequence "${lightFile.sequences[index].name}"?`))
      return;
    editingSequenceIndex = null;
    onchange({
      ...lightFile,
      sequences: lightFile.sequences.filter((_, i) => i !== index),
    });
  }

  function handleSequenceChange(seq: Sequence) {
    if (editingSequenceIndex === null) return;
    const sequences = [...lightFile.sequences];
    sequences[editingSequenceIndex] = seq;
    onchange({ ...lightFile, sequences });
  }

  // --- Show cue CRUD ---
  function handleShowCueChange(showIndex: number, cueIndex: number, cue: Cue) {
    const shows = [...lightFile.shows];
    const cues = [...shows[showIndex].cues];
    cues[cueIndex] = cue;
    shows[showIndex] = { ...shows[showIndex], cues };
    onchange({ ...lightFile, shows });
  }

  function handleShowCueDelete(showIndex: number, cueIndex: number) {
    clearSelection();
    const shows = [...lightFile.shows];
    shows[showIndex] = {
      ...shows[showIndex],
      cues: shows[showIndex].cues.filter((_, i) => i !== cueIndex),
    };
    onchange({ ...lightFile, shows });
  }

  function handleShowCueAdd(showIndex: number, cue: Cue) {
    const shows = [...lightFile.shows];
    const cues = [...shows[showIndex].cues, cue];
    cues.sort(
      (a, b) =>
        timestampToMs(a.timestamp, lightFile.tempo) -
        timestampToMs(b.timestamp, lightFile.tempo),
    );
    shows[showIndex] = { ...shows[showIndex], cues };
    onchange({ ...lightFile, shows });
    selectedShowIndex = showIndex;
    selectedCueIndex = cues.indexOf(cue);
  }

  // --- Selection ---
  function selectShowCue(showIndex: number, cueIndex: number) {
    selectedShowIndex = showIndex;
    selectedCueIndex = cueIndex;
  }

  function clearSelection() {
    selectedShowIndex = null;
    selectedCueIndex = null;
  }

  function handleSelectedCueChange(cue: Cue) {
    if (selectedShowIndex === null || selectedCueIndex === null) return;
    handleShowCueChange(selectedShowIndex, selectedCueIndex, cue);
  }

  function handleSelectedCueDelete() {
    if (selectedShowIndex === null || selectedCueIndex === null) return;
    handleShowCueDelete(selectedShowIndex, selectedCueIndex);
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

<div class="timeline-editor">
  <TimelineToolbar
    {pixelsPerMs}
    {cursorMs}
    {snapEnabled}
    {snapResolution}
    hasTempo={!!lightFile.tempo}
    onzoom={handleZoom}
    onfitview={fitView}
    onsnapchange={(enabled, res) => {
      snapEnabled = enabled;
      snapResolution = res;
    }}
    onaddshow={addShow}
    onaddsequence={addSequence}
  />

  <div class="timeline-body">
    <div
      class="timeline-scroll"
      bind:this={scrollContainer}
      onscroll={handleScroll}
      onmousemove={handleMouseMove}
      onmouseleave={handleMouseLeave}
      role="region"
      aria-label="Lighting timeline"
      tabindex="-1"
    >
      <div class="sticky-row ruler-row">
        <div class="lane-label-spacer"></div>
        <div class="lane-canvas-area">
          <TimeRuler
            {pixelsPerMs}
            {totalDurationMs}
            tempo={lightFile.tempo}
            {scrollLeft}
            {viewportWidth}
          />
        </div>
      </div>

      {#if getMergedPeaks().length > 0}
        <div class="sticky-row">
          <WaveformLane
            peaks={getMergedPeaks()}
            songDurationMs={songDurationMs || totalDurationMs}
            {pixelsPerMs}
            {scrollLeft}
            {viewportWidth}
          />
        </div>
      {/if}

      {#each lightFile.shows as show, si (show.name)}
        <div class="sticky-row">
          <ShowLane
            name={show.name}
            cues={show.cues}
            laneType="show"
            {pixelsPerMs}
            {scrollLeft}
            {viewportWidth}
            tempo={lightFile.tempo}
            selectedCueIndex={selectedShowIndex === si
              ? selectedCueIndex
              : null}
            {snapEnabled}
            {snapResolution}
            onselect={(ci) => selectShowCue(si, ci)}
            oncuechange={(ci, cue) => handleShowCueChange(si, ci, cue)}
            oncuedelete={(ci) => handleShowCueDelete(si, ci)}
            oncueadd={(cue) => handleShowCueAdd(si, cue)}
            ondelete={() => deleteShow(si)}
          />
        </div>
      {/each}

      {#if lightFile.shows.length === 0}
        <div class="sticky-row empty-lanes">
          <p>No shows. Click "+ Show" to get started.</p>
        </div>
      {/if}

      <div class="scroll-spacer" style:width="{getTotalWidthPx()}px"></div>
    </div>
  </div>

  <!-- Detail area -->
  <div class="detail-area">
    {#if getSelectedCue()}
      {@const selCue = getSelectedCue()}
      {#if selCue}
        <CuePropertiesPanel
          cue={selCue.cue}
          laneName={selCue.laneName}
          {groups}
          {sequenceNames}
          tempo={lightFile.tempo}
          onchange={handleSelectedCueChange}
          ondelete={handleSelectedCueDelete}
          onclose={clearSelection}
        />
      {/if}
    {:else if lightFile.sequences.length > 0}
      <div class="detail-sequences">
        <span class="detail-sequences-label">Sequences</span>
        <div class="seq-list">
          {#each lightFile.sequences as seq, i (seq.name)}
            <button class="seq-chip" onclick={() => (editingSequenceIndex = i)}>
              <span class="seq-chip-name">{seq.name}</span>
              <span class="seq-chip-count">{seq.cues.length} cues</span>
            </button>
          {/each}
        </div>
      </div>
    {:else}
      <div class="detail-empty">
        Select a cue to edit, or double-click a lane to create one.
      </div>
    {/if}
  </div>
</div>

<!-- Sequence editor modal -->
{#if editingSequenceIndex !== null && lightFile.sequences[editingSequenceIndex]}
  <SequenceEditorModal
    sequence={lightFile.sequences[editingSequenceIndex]}
    {groups}
    {sequenceNames}
    tempo={lightFile.tempo}
    {snapEnabled}
    {snapResolution}
    onchange={handleSequenceChange}
    ondelete={() => deleteSequence(editingSequenceIndex!)}
    onclose={() => (editingSequenceIndex = null)}
  />
{/if}

<style>
  .timeline-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1;
    min-height: 0;
  }
  .timeline-body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
    background: #111;
  }
  .timeline-scroll {
    flex: 1;
    overflow-x: auto;
    overflow-y: auto;
    min-width: 0;
    position: relative;
  }
  .sticky-row {
    position: sticky;
    left: 0;
    width: 100%;
  }
  .ruler-row {
    display: flex;
    z-index: 10;
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
    position: sticky;
    left: 0;
    top: 0;
  }
  .lane-label-spacer {
    width: 80px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
  }
  .lane-canvas-area {
    flex: 1;
    min-width: 0;
  }
  .scroll-spacer {
    height: 1px;
    pointer-events: none;
  }
  .empty-lanes {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 40px 20px;
    color: var(--text-dim);
    font-size: 13px;
  }

  .detail-area {
    flex-shrink: 0;
    height: 200px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
    background: var(--bg-card);
  }
  .detail-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-dim);
    font-size: 13px;
  }

  .detail-sequences {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 16px;
    height: 100%;
  }
  .detail-sequences-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
    padding-top: 4px;
    flex-shrink: 0;
  }
  .seq-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .seq-chip {
    background: rgba(139, 92, 246, 0.1);
    border: 1px solid rgba(139, 92, 246, 0.3);
    border-radius: var(--radius);
    padding: 4px 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    font-size: 12px;
  }
  .seq-chip:hover {
    background: rgba(139, 92, 246, 0.2);
    border-color: rgba(139, 92, 246, 0.5);
  }
  .seq-chip-name {
    font-weight: 500;
  }
  .seq-chip-count {
    font-size: 10px;
    color: var(--text-muted);
  }
</style>
