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
    LightFile,
    Cue,
    Sequence,
    SubLaneType,
  } from "../../../lib/lighting/types";
  import type { WaveformTrack } from "../../../lib/api/songs";
  import {
    timestampToMs,
    msToPixel,
    computeAdjustedCuePositions,
    offsetMeasuresToMs,
    type OffsetMarker,
  } from "../../../lib/lighting/timeline-state";
  import TimelineToolbar from "./TimelineToolbar.svelte";
  import TimeRuler from "./TimeRuler.svelte";
  import WaveformLane from "./WaveformLane.svelte";
  import ShowGroup from "./ShowGroup.svelte";
  import TempoLane from "./TempoLane.svelte";
  import CuePropertiesPanel from "./CuePropertiesPanel.svelte";
  import SequenceEditorModal from "./SequenceEditorModal.svelte";
  import StagePreview from "./StagePreview.svelte";
  import { t } from "svelte-i18n";
  import { tick } from "svelte";

  interface Props {
    lightFile: LightFile;
    groups: string[];
    sequenceNames: string[];
    songDurationMs: number;
    waveformTracks: WaveformTrack[];
    isPlaying?: boolean;
    playheadMs?: number | null;
    onchange: (lightFile: LightFile) => void;
    onplay?: (ms: number) => void;
    onstop?: () => void;
  }

  let {
    lightFile,
    groups,
    sequenceNames,
    songDurationMs,
    waveformTracks,
    isPlaying = false,
    playheadMs = null,
    onchange,
    onplay,
    onstop,
  }: Props = $props();

  // Timeline state
  let pixelsPerMs = $state(0.15);
  let scrollLeft = $state(0);
  let viewportWidth = $state(800);
  let cursorMs = $state<number | null>(null);
  let snapEnabled = $state(true);
  let snapResolution = $state<"beat" | "measure">("beat");

  // Playback cursor (the ruler position to play from)
  let playCursorMs = $state<number>(0);

  function handlePlay() {
    onplay?.(playCursorMs);
  }

  function handlePause() {
    // Remember where the playhead was so we can resume from there
    if (playheadMs !== null && playheadMs !== undefined) {
      playCursorMs = playheadMs;
    }
    onstop?.();
  }

  function handleStop() {
    playCursorMs = 0;
    onstop?.();
    scrollToCursorMs(0);
  }

  function scrollToCursorMs(ms: number) {
    if (!scrollContainer) return;
    const contentWidth = viewportWidth - 80;
    scrollContainer.scrollLeft = ms * pixelsPerMs - contentWidth / 2;
  }

  function handleSkipStart() {
    playCursorMs = 0;
    scrollToCursorMs(0);
  }

  function handleSkipEnd() {
    playCursorMs = totalDurationMs;
    scrollToCursorMs(totalDurationMs);
  }

  function handleKeydown(e: KeyboardEvent) {
    // Ignore if focus is in an input/textarea/select
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

    if (e.code === "Space") {
      e.preventDefault();
      if (isPlaying) {
        handlePause();
      } else if (onplay) {
        handlePlay();
      }
    } else if (e.code === "Home") {
      e.preventDefault();
      if (!isPlaying) handleSkipStart();
    } else if (e.code === "End") {
      e.preventDefault();
      if (!isPlaying) handleSkipEnd();
    }
  }

  // Selection
  let selectedShowIndex = $state<number | null>(null);
  let selectedCueIndex = $state<number | null>(null);
  let selectedSubLane = $state<SubLaneType | null>(null);

  // Sequence editing modal
  let editingSequenceIndex = $state<number | null>(null);

  // Offset context menu
  let offsetMenu = $state<{
    x: number;
    y: number;
    ms: number;
    /** If editing an existing offset, the marker info */
    editing?: OffsetMarker;
  } | null>(null);
  let offsetInputValue = $state("");

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

  // Compute offset markers from all show cues using adjusted positions
  function getOffsetMarkers(): OffsetMarker[] {
    const markers: OffsetMarker[] = [];
    for (let si = 0; si < lightFile.shows.length; si++) {
      const show = lightFile.shows[si];
      const adjusted = computeAdjustedCuePositions(show.cues, lightFile.tempo);
      for (const ap of adjusted) {
        const rawMs = timestampToMs(ap.cue.timestamp, lightFile.tempo);
        if (ap.cue.offset_measures !== undefined) {
          const durationMs = lightFile.tempo
            ? offsetMeasuresToMs(
                ap.cue.offset_measures,
                ap.adjustedMs,
                lightFile.tempo,
              )
            : 0;
          markers.push({
            ms: ap.adjustedMs,
            rawMs,
            durationMs,
            type: "offset",
            measures: ap.cue.offset_measures,
            showIndex: si,
            cueIndex: ap.index,
          });
        }
        if (ap.cue.reset_measures) {
          markers.push({
            ms: ap.adjustedMs,
            rawMs,
            durationMs: 0,
            type: "reset",
            measures: 0,
            showIndex: si,
            cueIndex: ap.index,
          });
        }
      }
    }
    return markers.sort((a, b) => a.ms - b.ms);
  }

  function getMaxShowCueTime(lf: LightFile): number {
    let max = 0;
    for (const show of lf.shows) {
      const adjusted = computeAdjustedCuePositions(show.cues, lf.tempo);
      for (const ap of adjusted) {
        let end = ap.adjustedMs;
        // Include the offset duration after the last cue
        if (ap.cue.offset_measures && ap.cue.offset_measures > 0 && lf.tempo) {
          end += offsetMeasuresToMs(
            ap.cue.offset_measures,
            ap.adjustedMs,
            lf.tempo,
          );
        }
        max = Math.max(max, end);
      }
    }
    return max;
  }

  // --- Scroll ---
  let scrollRaf = 0;
  let zooming = false;
  function handleScroll() {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      // Don't overwrite scrollLeft during a zoom — the zoom handler owns it.
      if (!zooming && scrollContainer) scrollLeft = scrollContainer.scrollLeft;
    });
  }

  const MIN_ZOOM = 0.01;
  const MAX_ZOOM = 2;

  async function applyZoom(
    newPxPerMs: number,
    anchorMs: number,
    anchorPx: number,
  ) {
    zooming = true;
    pixelsPerMs = newPxPerMs;
    // Flush DOM so the scroll-spacer width reflects the new pixelsPerMs
    // before we set scrollLeft (otherwise the browser clamps to the old range).
    await tick();
    if (scrollContainer) {
      const newScroll = anchorMs * newPxPerMs - anchorPx;
      scrollContainer.scrollLeft = newScroll;
      scrollLeft = scrollContainer.scrollLeft;
    }
    zooming = false;
  }

  function handleZoom(newPixelsPerMs: number) {
    if (!scrollContainer) return;
    // Anchor on the center of the content area (exclude 80px label column)
    const contentWidth = viewportWidth - 80;
    // Read directly from DOM — the state variable may be stale during rapid zoom.
    const currentScroll = scrollContainer.scrollLeft;
    const centerMs = (currentScroll + contentWidth / 2) / pixelsPerMs;
    applyZoom(newPixelsPerMs, centerMs, contentWidth / 2);
  }

  function handleRulerPan(deltaPx: number) {
    if (!scrollContainer) return;
    scrollContainer.scrollLeft = Math.max(
      0,
      scrollContainer.scrollLeft + deltaPx,
    );
  }

  function handleWheel(e: WheelEvent) {
    if (!scrollContainer) return;
    // Only zoom when Ctrl/Cmd is held, otherwise let normal scroll happen
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();

    const rect = scrollContainer.getBoundingClientRect();
    // Mouse position relative to the content area (account for 80px label column)
    const mouseXInViewport = e.clientX - rect.left - 80;
    // Read directly from DOM — the state variable may be stale during rapid zoom.
    const currentScroll = scrollContainer.scrollLeft;
    const mouseMs = (currentScroll + mouseXInViewport) / pixelsPerMs;

    const zoomFactor = e.deltaY > 0 ? 1 / 1.15 : 1.15;
    const newPxPerMs = Math.min(
      MAX_ZOOM,
      Math.max(MIN_ZOOM, pixelsPerMs * zoomFactor),
    );

    applyZoom(newPxPerMs, mouseMs, mouseXInViewport);
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

  // --- Offset context menu ---
  function handleRulerContextMenu(
    ms: number,
    clientX: number,
    clientY: number,
  ) {
    if (lightFile.shows.length === 0) return;
    offsetMenu = { x: clientX, y: clientY, ms };
    offsetInputValue = "";
  }

  function handleOffsetClick(off: OffsetMarker) {
    // Open the popover for editing this existing offset
    // Position near the offset marker on the ruler
    if (!scrollContainer) return;
    const rect = scrollContainer.getBoundingClientRect();
    const x = msToPixel(off.ms, pixelsPerMs) - scrollLeft + rect.left + 80;
    const y = rect.top;
    offsetMenu = { x, y, ms: off.ms, editing: off };
    offsetInputValue = String(off.measures);
  }

  function closeOffsetMenu() {
    offsetMenu = null;
  }

  function addOffsetToShow(showIndex: number) {
    const measures = parseInt(offsetInputValue);
    if (isNaN(measures) || measures <= 0) return;

    const show = lightFile.shows[showIndex];
    const adjusted = computeAdjustedCuePositions(show.cues, lightFile.tempo);
    let targetIndex = -1;
    for (let i = adjusted.length - 1; i >= 0; i--) {
      if (adjusted[i].adjustedMs <= offsetMenu!.ms) {
        targetIndex = i;
        break;
      }
    }

    if (targetIndex < 0) {
      if (show.cues.length > 0) targetIndex = 0;
      else return;
    }

    const shows = [...lightFile.shows];
    const cues = [...shows[showIndex].cues];
    cues[targetIndex] = { ...cues[targetIndex], offset_measures: measures };
    shows[showIndex] = { ...shows[showIndex], cues };
    onchange({ ...lightFile, shows });
    closeOffsetMenu();
  }

  function updateEditingOffset() {
    if (!offsetMenu?.editing) return;
    const measures = parseInt(offsetInputValue);
    const { showIndex, cueIndex } = offsetMenu.editing;
    const shows = [...lightFile.shows];
    const cues = [...shows[showIndex].cues];
    if (isNaN(measures) || measures <= 0) {
      // Remove offset
      cues[cueIndex] = { ...cues[cueIndex], offset_measures: undefined };
    } else {
      cues[cueIndex] = { ...cues[cueIndex], offset_measures: measures };
    }
    shows[showIndex] = { ...shows[showIndex], cues };
    onchange({ ...lightFile, shows });
    closeOffsetMenu();
  }

  function deleteEditingOffset() {
    if (!offsetMenu?.editing) return;
    const { showIndex, cueIndex } = offsetMenu.editing;
    const shows = [...lightFile.shows];
    const cues = [...shows[showIndex].cues];
    cues[cueIndex] = { ...cues[cueIndex], offset_measures: undefined };
    shows[showIndex] = { ...shows[showIndex], cues };
    onchange({ ...lightFile, shows });
    closeOffsetMenu();
  }

  // --- Selection ---
  function selectShowCue(
    showIndex: number,
    cueIndex: number,
    subLane?: SubLaneType,
  ) {
    selectedShowIndex = showIndex;
    selectedCueIndex = cueIndex;
    selectedSubLane = subLane ?? null;
  }

  function clearSelection() {
    selectedShowIndex = null;
    selectedCueIndex = null;
    selectedSubLane = null;
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
    {isPlaying}
    canPlay={lightFile.shows.length > 0 && !!onplay}
    {playheadMs}
    {playCursorMs}
    onzoom={handleZoom}
    onfitview={fitView}
    onsnapchange={(enabled, res) => {
      snapEnabled = enabled;
      snapResolution = res;
    }}
    onaddshow={addShow}
    onaddsequence={addSequence}
    onplay={handlePlay}
    onpause={handlePause}
    onstop={handleStop}
    onskipstart={handleSkipStart}
    onskipend={handleSkipEnd}
  />

  <div class="timeline-body">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="timeline-scroll"
      bind:this={scrollContainer}
      onscroll={handleScroll}
      onwheel={handleWheel}
      onmousemove={handleMouseMove}
      onmouseleave={handleMouseLeave}
      onkeydown={handleKeydown}
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
            offsets={getOffsetMarkers()}
            {playheadMs}
            {playCursorMs}
            onclick={(ms) => {
              playCursorMs = ms;
            }}
            onpan={handleRulerPan}
            oncontextmenu={handleRulerContextMenu}
            onoffsetclick={handleOffsetClick}
          />
        </div>
      </div>

      {#if lightFile.tempo}
        <div class="sticky-row">
          <TempoLane
            tempo={lightFile.tempo}
            {pixelsPerMs}
            {scrollLeft}
            {viewportWidth}
            {totalDurationMs}
            offsets={getOffsetMarkers()}
          />
        </div>
      {/if}

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
          <ShowGroup
            name={show.name}
            cues={show.cues}
            {pixelsPerMs}
            {scrollLeft}
            {viewportWidth}
            tempo={lightFile.tempo}
            selectedCueIndex={selectedShowIndex === si
              ? selectedCueIndex
              : null}
            selectedSubLane={selectedShowIndex === si ? selectedSubLane : null}
            {snapEnabled}
            {snapResolution}
            offsets={getOffsetMarkers()}
            {playheadMs}
            onselect={(ci, subLane) => selectShowCue(si, ci, subLane)}
            oncuechange={(ci, cue) => handleShowCueChange(si, ci, cue)}
            oncuedelete={(ci) => handleShowCueDelete(si, ci)}
            oncueadd={(cue) => handleShowCueAdd(si, cue)}
            ondelete={() => deleteShow(si)}
          />
        </div>
      {/each}

      {#if lightFile.shows.length === 0}
        <div class="sticky-row empty-lanes">
          <p>{$t("timeline.noShows")}</p>
        </div>
      {/if}

      <div class="scroll-spacer" style:width="{getTotalWidthPx()}px"></div>
    </div>
  </div>

  <!-- Bottom panel: stage preview + detail area -->
  <div class="bottom-panel">
    <div class="stage-area">
      <StagePreview />
    </div>
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
            focusTab={selectedSubLane}
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
              <button
                class="seq-chip"
                onclick={() => (editingSequenceIndex = i)}
              >
                <span class="seq-chip-name">{seq.name}</span>
                <span class="seq-chip-count">{seq.cues.length} cues</span>
              </button>
            {/each}
          </div>
        </div>
      {:else}
        <div class="detail-empty">
          {$t("timeline.selectCue")}
        </div>
      {/if}
    </div>
  </div>
</div>

<!-- Offset context menu -->
{#if offsetMenu}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="offset-menu-backdrop"
    onclick={closeOffsetMenu}
    onkeydown={(e) => e.key === "Escape" && closeOffsetMenu()}
  ></div>
  <div
    class="offset-menu"
    style:left="{offsetMenu.x}px"
    style:top="{offsetMenu.y}px"
  >
    {#if offsetMenu.editing}
      <div class="offset-menu-title">{$t("timeline.editOffset")}</div>
      <input
        type="number"
        class="offset-menu-input"
        min="1"
        placeholder={$t("timeline.measures")}
        bind:value={offsetInputValue}
        onkeydown={(e) => {
          if (e.key === "Escape") closeOffsetMenu();
          if (e.key === "Enter") updateEditingOffset();
        }}
      />
      <div class="offset-menu-actions">
        <button
          class="btn btn-sm offset-menu-btn"
          onclick={updateEditingOffset}
        >
          {$t("common.save")}
        </button>
        <button
          class="btn btn-sm btn-danger offset-menu-btn"
          onclick={deleteEditingOffset}
        >
          {$t("common.delete")}
        </button>
      </div>
    {:else}
      <div class="offset-menu-title">{$t("timeline.addOffsetMeasures")}</div>
      <input
        type="number"
        class="offset-menu-input"
        min="1"
        placeholder="e.g. 8"
        bind:value={offsetInputValue}
        onkeydown={(e) => {
          if (e.key === "Escape") closeOffsetMenu();
          if (e.key === "Enter" && lightFile.shows.length === 1)
            addOffsetToShow(0);
        }}
      />
      {#if lightFile.shows.length === 1}
        <button
          class="btn btn-sm offset-menu-btn"
          onclick={() => addOffsetToShow(0)}
        >
          {$t("common.add")}
        </button>
      {:else}
        {#each lightFile.shows as show, si (show.name)}
          <button
            class="btn btn-sm offset-menu-btn"
            onclick={() => addOffsetToShow(si)}
          >
            {show.name}
          </button>
        {/each}
      {/if}
    {/if}
  </div>
{/if}

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
    font-size: 14px;
  }

  .bottom-panel {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
    height: 280px;
  }
  .stage-area {
    width: 320px;
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .detail-area {
    flex: 1;
    min-width: 0;
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
    font-size: 14px;
  }

  .detail-sequences {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 16px;
    height: 100%;
  }
  .detail-sequences-label {
    font-size: 12px;
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
    background: var(--pink-dim);
    border: 1px solid rgba(239, 96, 163, 0.3);
    border-radius: var(--radius);
    padding: 4px 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    font-size: 13px;
  }
  .seq-chip:hover {
    background: rgba(239, 96, 163, 0.2);
    border-color: rgba(239, 96, 163, 0.5);
  }
  .seq-chip-name {
    font-weight: 500;
  }
  .seq-chip-count {
    font-size: 13px;
    color: var(--text-muted);
  }

  .offset-menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 99;
  }
  .offset-menu {
    position: fixed;
    z-index: 100;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    min-width: 160px;
  }
  .offset-menu-title {
    font-size: 13px;
    color: var(--text-muted);
    font-weight: 600;
  }
  .offset-menu-input {
    width: 100%;
    padding: 4px 6px;
    font-size: 13px;
    font-family: var(--mono);
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--bg-input);
    color: var(--text);
    box-sizing: border-box;
  }
  .offset-menu-input:focus {
    border-color: var(--accent);
    outline: none;
  }
  .offset-menu-actions {
    display: flex;
    gap: 4px;
  }
  .offset-menu-btn {
    text-align: left;
    justify-content: flex-start;
  }
</style>
