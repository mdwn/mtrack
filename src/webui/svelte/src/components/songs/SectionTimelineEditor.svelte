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
  import type {
    SongSummary,
    WaveformTrack,
    TempoConfig,
    PilotConfig,
    PilotHintConfig,
    MetronomeConfig,
  } from "../../lib/api/songs";
  import { sortTempoChanges } from "../../lib/util/tempo";
  import { accentsGlyph, subdivisionChip } from "../../lib/meter";
  import SectionBar from "./SectionBar.svelte";
  import SectionRuler from "./SectionRuler.svelte";
  import SectionWaveformLane from "./SectionWaveformLane.svelte";
  import { sectionColor } from "../../lib/sectionColors";
  import TimelineMarkerLane from "./TimelineMarkerLane.svelte";
  import TempoMarkerDialog from "./TempoMarkerDialog.svelte";
  import PilotHintDialog from "./PilotHintDialog.svelte";
  import SectionEditDialog from "./SectionEditDialog.svelte";

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
    color?: string;
  }

  interface Props {
    song: SongSummary;
    waveformTracks: WaveformTrack[];
    sections: SectionEntry[];
    dirty?: boolean;
    /** The song's `tempo:` block; edited via the tempo layer. */
    tempo?: TempoConfig | null;
    /** The song's `pilot:` block; edited via the pilot layer. */
    pilot?: PilotConfig | null;
    /** The song's `metronome:` block; feel changes ride on tempo markers. */
    metronome?: MetronomeConfig | null;
    songName?: string;
    hasMidi?: boolean;
    ontempochange?: (tempo: TempoConfig | null) => void;
    onpilotchange?: (pilot: PilotConfig | null) => void;
    onmetronomechange?: (metronome: MetronomeConfig | null) => void;
  }

  let {
    song,
    waveformTracks,
    sections = $bindable([]),
    dirty = $bindable(false), // eslint-disable-line no-useless-assignment -- consumed by parent via bind:dirty
    tempo = null,
    pilot = null,
    metronome = null,
    songName,
    hasMidi = false,
    ontempochange,
    onpilotchange,
    onmetronomechange,
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

  // --- Section edit dialog ---

  let sectionDialogIndex = $state<number | null>(null);

  function patchSection(index: number, patch: Partial<SectionEntry>) {
    const updated = [...sections];
    const merged = { ...updated[index], ...patch };
    // An empty name keeps the previous one; "auto" color drops the key.
    if (!merged.name) merged.name = updated[index].name;
    if (!merged.color) delete merged.color;
    updated[index] = merged;
    handleSectionsChange(updated);
  }

  function deleteSection(index: number) {
    handleSectionsChange(sections.filter((_, i) => i !== index));
    sectionDialogIndex = null;
  }

  // --- Tempo / pilot marker layers ---

  interface MarkerPosition {
    measure: number;
    beat: number;
  }

  let tempoDialogTarget = $state<"start" | MarkerPosition | null>(null);
  let pilotDialogIndex = $state<number | null>(null);

  /** Time (ms) of a measure/beat position on the beat grid. */
  function measureBeatToMs(measure: number, beat: number): number {
    const grid = song.beat_grid;
    if (!grid) return 0;
    const startIdx = grid.measure_starts[measure - 1];
    if (startIdx === undefined) return songDurationMs;
    const time = grid.beats[startIdx + (beat - 1)];
    return time === undefined ? songDurationMs : time * 1000;
  }

  /** Signature in effect at a position (base + prior changes). */
  function sigAt(measure: number, beat: number): [number, number] {
    const parse = (raw: string | undefined): [number, number] | null => {
      const match = /^\s*(\d+)\s*\/\s*(\d+)\s*$/.exec(raw ?? "");
      return match ? [parseInt(match[1]), parseInt(match[2])] : null;
    };
    let sig = parse(tempo?.time_signature) ?? [4, 4];
    const sorted = [...(tempo?.changes ?? [])].sort(
      (a, b) => a.measure - b.measure || (a.beat ?? 1) - (b.beat ?? 1),
    );
    for (const c of sorted) {
      const atOrBefore =
        c.measure < measure || (c.measure === measure && (c.beat ?? 1) <= beat);
      if (!atOrBefore) break;
      sig = parse(c.time_signature) ?? sig;
    }
    return sig;
  }

  /** Pattern resized to the meter at its position, like the renderer. */
  function chipPattern(levels: number[], numerator: number): number[] {
    return Array.from({ length: numerator }, (_, i) => levels[i] ?? 1);
  }

  /** One marker per position, merging tempo changes with metronome feel
   * changes at the same measure. */
  let tempoMarkers = $derived.by(() => {
    if (!tempo) return [];
    const baseParts = [`${tempo.bpm}`, tempo.time_signature ?? "4/4"];
    if (metronome?.accents?.length) {
      baseParts.push(
        accentsGlyph(chipPattern(metronome.accents, sigAt(1, 0)[0])),
      );
    }
    if (metronome?.subdivision !== undefined && metronome.subdivision !== 1) {
      baseParts.push(subdivisionChip(metronome.subdivision, sigAt(1, 0)[1]));
    }
    const markers = [
      {
        id: "start",
        ms: (tempo.start ?? 0) * 1000,
        label: baseParts.join(" · "),
      },
    ];
    // Collect marker positions: every tempo change plus every metronome
    // change measure (feel changes attach to the lowest-beat marker of
    // their measure, or stand alone).
    const positions: Record<string, MarkerPosition> = {};
    for (const c of tempo.changes ?? []) {
      const beat = c.beat ?? 1;
      positions[`${c.measure}:${beat}`] = { measure: c.measure, beat };
    }
    for (const c of metronome?.changes ?? []) {
      if (!feelHostBeat(c.measure)) {
        positions[`${c.measure}:1`] = { measure: c.measure, beat: 1 };
      }
    }
    const sorted = Object.values(positions).sort(
      (a, b) => a.measure - b.measure || a.beat - b.beat,
    );
    for (const pos of sorted) {
      const tc = (tempo.changes ?? []).find(
        (c) => c.measure === pos.measure && (c.beat ?? 1) === pos.beat,
      );
      const mc =
        pos.beat === (feelHostBeat(pos.measure) ?? 1)
          ? (metronome?.changes ?? []).find((c) => c.measure === pos.measure)
          : undefined;
      const parts: string[] = [];
      if (tc?.bpm !== undefined) parts.push(`${tc.bpm}`);
      if (tc?.time_signature) parts.push(tc.time_signature);
      const sig = sigAt(pos.measure, pos.beat);
      if (mc?.accents)
        parts.push(accentsGlyph(chipPattern(mc.accents, sig[0])));
      if (mc?.subdivision !== undefined) {
        parts.push(subdivisionChip(mc.subdivision, sig[1]));
      }
      markers.push({
        id: `c${pos.measure}:${pos.beat}`,
        ms: measureBeatToMs(pos.measure, pos.beat),
        label: (tc?.transition ? "↗ " : "") + (parts.join(" · ") || "—"),
      });
    }
    return markers;
  });

  /** The beat of the marker hosting feel changes for a measure: the lowest
   * tempo-change beat there, or 1 (undefined when no tempo change exists,
   * which also maps to a beat-1 marker). */
  function feelHostBeat(measure: number): number | undefined {
    const beats = (tempo?.changes ?? [])
      .filter((c) => c.measure === measure)
      .map((c) => c.beat ?? 1);
    return beats.length > 0 ? Math.min(...beats) : undefined;
  }

  let pilotMarkers = $derived(
    (pilot?.hints ?? []).map((h, i) => ({
      id: `h${i}`,
      ms:
        "measure" in h.at
          ? measureBeatToMs(h.at.measure, h.at.beat ?? 1)
          : h.at.time * 1000,
      label: h.label || "…",
      icon: h.file ? "🔊" : undefined,
    })),
  );

  function nearestMeasureAt(ms: number): number {
    let best = 1;
    let bestDist = Infinity;
    for (let i = 0; i < measureTimesMs.length; i++) {
      const dist = Math.abs(measureTimesMs[i] - ms);
      if (dist < bestDist) {
        bestDist = dist;
        best = i + 1;
      }
    }
    return best;
  }

  /** The BPM in effect at a measure (base tempo + preceding changes). */
  function effectiveBpmAt(measure: number): number {
    if (!tempo) return 120;
    let bpm = tempo.bpm;
    for (const c of sortTempoChanges(tempo.changes ?? [])) {
      if (c.measure <= measure && c.bpm !== undefined) bpm = c.bpm;
    }
    return bpm;
  }

  function handleTempoMarkerClick(id: string) {
    if (id === "start") {
      tempoDialogTarget = "start";
      return;
    }
    const match = /^c(\d+):(\d+)$/.exec(id);
    if (match) {
      tempoDialogTarget = {
        measure: parseInt(match[1]),
        beat: parseInt(match[2]),
      };
    }
  }

  function handleTempoEmptyClick(ms: number) {
    if (!tempo) {
      // No tempo map yet — create one and open its editor right away.
      ontempochange?.({ bpm: 120, time_signature: "4/4" });
      tempoDialogTarget = "start";
      return;
    }
    if (measureTimesMs.length === 0) {
      tempoDialogTarget = "start";
      return;
    }
    const measure = nearestMeasureAt(ms);
    if (measure <= 1) {
      tempoDialogTarget = "start";
      return;
    }
    // A marker already at this measure? Open it instead of stacking.
    const hostBeat = feelHostBeat(measure);
    if (hostBeat !== undefined) {
      tempoDialogTarget = { measure, beat: hostBeat };
      return;
    }
    if ((metronome?.changes ?? []).some((c) => c.measure === measure)) {
      tempoDialogTarget = { measure, beat: 1 };
      return;
    }
    // Tapping a spot before an existing change must not append out of order —
    // the backend rejects a non-ascending map.
    const entry = { measure, bpm: effectiveBpmAt(measure) };
    const changes = sortTempoChanges([...(tempo.changes ?? []), entry]);
    ontempochange?.({ ...tempo, changes });
    tempoDialogTarget = { measure, beat: 1 };
  }

  /** Nearest beat as measure/beat for a position (ms), or null off-grid. */
  function nearestBeatAt(
    ms: number,
  ): { measure: number; beat?: number } | null {
    const grid = song.beat_grid;
    if (!grid || grid.beats.length === 0) return null;
    const secs = ms / 1000;
    let nearest = 0;
    for (let i = 1; i < grid.beats.length; i++) {
      if (
        Math.abs(grid.beats[i] - secs) < Math.abs(grid.beats[nearest] - secs)
      ) {
        nearest = i;
      }
    }
    let measure = 0;
    while (
      measure + 1 < grid.measure_starts.length &&
      grid.measure_starts[measure + 1] <= nearest
    ) {
      measure++;
    }
    const beat = nearest - grid.measure_starts[measure] + 1;
    const at: { measure: number; beat?: number } = { measure: measure + 1 };
    if (beat > 1) at.beat = beat;
    return at;
  }

  function handlePilotMarkerClick(id: string) {
    pilotDialogIndex = parseInt(id.slice(1));
  }

  function handlePilotEmptyClick(ms: number) {
    const at = nearestBeatAt(ms) ?? { time: Math.round(ms / 100) / 10 };
    const hints = [...(pilot?.hints ?? []), { at, label: "" }];
    onpilotchange?.({ ...(pilot ?? {}), hints });
    pilotDialogIndex = hints.length - 1;
  }

  function patchPilotHint(index: number, patch: Partial<PilotHintConfig>) {
    const hints = [...(pilot?.hints ?? [])];
    const merged = { ...hints[index], ...patch };
    if (merged.align === "end") delete merged.align;
    if (merged.offset === 0 || merged.offset === undefined)
      delete merged.offset;
    if (!merged.file) delete merged.file;
    hints[index] = merged;
    onpilotchange?.({ ...(pilot ?? {}), hints });
  }

  function deletePilotHint(index: number) {
    const hints = (pilot?.hints ?? []).filter((_, i) => i !== index);
    onpilotchange?.({ ...(pilot ?? {}), hints });
    pilotDialogIndex = null;
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
          >No beat grid — add a tempo map or click track for measure-based
          sections</span
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
    <TimelineMarkerLane
      laneLabel={$t("timelineLayers.tempo")}
      markers={tempoMarkers}
      {pixelsPerMs}
      {scrollLeft}
      accent="#f97316"
      emptyHint={$t("timelineLayers.tempoEmptyHint")}
      onmarkerclick={handleTempoMarkerClick}
      onemptyclick={handleTempoEmptyClick}
    />

    <SectionBar
      {sections}
      {pixelsPerMs}
      {scrollLeft}
      {viewportWidth}
      {measureTimesMs}
      {songDurationMs}
      emptyHint={song.beat_grid ? $t("sections.emptyHint") : ""}
      onsectionschange={handleSectionsChange}
      onsectionedit={(index) => (sectionDialogIndex = index)}
    />

    <TimelineMarkerLane
      laneLabel={$t("timelineLayers.pilot")}
      markers={pilotMarkers}
      {pixelsPerMs}
      {scrollLeft}
      accent="#8b5cf6"
      emptyHint={$t("timelineLayers.pilotEmptyHint")}
      onmarkerclick={handlePilotMarkerClick}
      onemptyclick={handlePilotEmptyClick}
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
      {#each sections as section, i (section.name)}
        <span
          class="section-chip"
          style:border-color="color-mix(in srgb, {sectionColor(
            section.color,
            i,
          )} 60%, transparent)"
        >
          {section.name}
          <span class="chip-range"
            >m{section.start_measure}–{section.end_measure}</span
          >
        </span>
      {/each}
    </div>
  {/if}
</div>

{#if tempoDialogTarget !== null && tempo}
  <TempoMarkerDialog
    {tempo}
    target={tempoDialogTarget}
    {metronome}
    feelHost={tempoDialogTarget === "start" ||
      tempoDialogTarget.beat === (feelHostBeat(tempoDialogTarget.measure) ?? 1)}
    {songName}
    {hasMidi}
    canGuess={!!song.beat_grid}
    ontempochange={(updated) => ontempochange?.(updated)}
    onmetronomechange={(updated) => onmetronomechange?.(updated)}
    onmove={(position) => (tempoDialogTarget = position)}
    onclose={() => (tempoDialogTarget = null)}
  />
{/if}

{#if sectionDialogIndex !== null && sections[sectionDialogIndex]}
  <SectionEditDialog
    section={sections[sectionDialogIndex]}
    index={sectionDialogIndex}
    onchange={(patch) => patchSection(sectionDialogIndex!, patch)}
    ondelete={() => deleteSection(sectionDialogIndex!)}
    onclose={() => (sectionDialogIndex = null)}
  />
{/if}

{#if pilotDialogIndex !== null && pilot?.hints?.[pilotDialogIndex]}
  <PilotHintDialog
    hint={pilot.hints[pilotDialogIndex]}
    hasBeatGrid={!!song.beat_grid}
    beatGrid={song.beat_grid}
    onchange={(patch) => patchPilotHint(pilotDialogIndex!, patch)}
    ondelete={() => deletePilotHint(pilotDialogIndex!)}
    onclose={() => (pilotDialogIndex = null)}
  />
{/if}

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
</style>
