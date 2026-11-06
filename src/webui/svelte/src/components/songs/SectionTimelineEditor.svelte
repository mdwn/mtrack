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
    PilotConfig,
    PilotHint,
    Section,
    SongInfo,
    TrackPeaks,
  } from "../../gen/rules/mtrack_rules";
  import { t } from "svelte-i18n";
  import type {
    TempoConfig,
    MetronomeConfig,
    SongFile,
  } from "../../lib/api/songs";
  import { rules } from "../../lib/rules";
  import type { TempoSection } from "../../lib/lighting/types";
  import { accentsGlyph, subdivisionChip } from "../../lib/meter";
  import SectionBar from "./SectionBar.svelte";
  import SectionRuler from "./SectionRuler.svelte";
  import SectionWaveformLane from "./SectionWaveformLane.svelte";
  import { playbackStore } from "../../lib/ws/stores";
  import { playerClient } from "../../lib/grpc/client";
  import { sectionColor } from "../../lib/sectionColors";
  import TimelineMarkerLane from "./TimelineMarkerLane.svelte";
  import TempoMarkerDialog, {
    type MarkerPosition,
  } from "./TempoMarkerDialog.svelte";
  import PilotHintDialog from "./PilotHintDialog.svelte";
  import SectionEditDialog from "./SectionEditDialog.svelte";

  interface Props {
    song: SongInfo;
    waveformTracks: TrackPeaks[];
    sections: Section[];
    dirty?: boolean;
    /** The song's `tempo:` block; edited via the tempo layer. */
    tempo?: TempoConfig | null;
    /** The song's `pilot:` block; edited via the pilot layer. */
    pilot?: PilotConfig | null;
    /** The song's `metronome:` block; feel changes ride on tempo markers. */
    metronome?: MetronomeConfig | null;
    songName?: string;
    hasMidi?: boolean;
    /** The song directory listing, for the pilot clip picker. */
    songFiles?: SongFile[];
    /** Light shows with their own tempo maps (import sources). */
    lightShowTempos?: { file: string; tempo: TempoSection }[];
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
    songFiles = [],
    lightShowTempos = [],
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

  function handleSectionsChange(updated: Section[]) {
    sections = updated;
    dirty = true;
  }

  // --- Preview playback / playhead ---

  let isCurrentSong = $derived($playbackStore.song_name === song.name);

  // Smoothly-extrapolated elapsed, resynced on every 5Hz frame (matches
  // the BeatIndicator's approach). Never extrapolates past 600ms so a
  // dropped connection freezes the playhead instead of running away.
  const MAX_EXTRAPOLATION_MS = 600;
  let smoothElapsedMs = $state(0);
  $effect(() => {
    if (!isCurrentSong || !$playbackStore.is_playing) {
      smoothElapsedMs = $playbackStore.elapsed_ms;
      return;
    }
    let raf = 0;
    const tick = () => {
      const state = $playbackStore;
      const since = Math.min(
        performance.now() - state.received_at,
        MAX_EXTRAPOLATION_MS,
      );
      smoothElapsedMs = Math.min(
        state.elapsed_ms + since,
        state.song_duration_ms || Infinity,
      );
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  // Dragging the playhead scrubs: the line follows the pointer locally and
  // a single seek fires on release (seeks restart sources, so continuous
  // seeking while dragging would thrash).
  let playheadDragMs = $state<number | null>(null);
  let timelineWrapEl: HTMLDivElement | undefined = $state();

  let playheadMs = $derived(
    playheadDragMs ??
      (!$playbackStore.is_playing && isCurrentSong
        ? ($playbackStore.pending_start_ms ?? smoothElapsedMs)
        : smoothElapsedMs),
  );
  let playheadX = $derived(LABEL_WIDTH + playheadMs * pixelsPerMs - scrollLeft);

  function playheadMsFromPointer(clientX: number): number {
    if (!timelineWrapEl) return playheadMs;
    const rect = timelineWrapEl.getBoundingClientRect();
    const ms = (clientX - rect.left - LABEL_WIDTH + scrollLeft) / pixelsPerMs;
    return Math.max(0, Math.min(ms, songDurationMs));
  }

  function handlePlayheadDown(e: PointerEvent) {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    playheadDragMs = playheadMsFromPointer(e.clientX);
    e.preventDefault();
  }

  function handlePlayheadMove(e: PointerEvent) {
    if (playheadDragMs === null) return;
    playheadDragMs = playheadMsFromPointer(e.clientX);
  }

  function handlePlayheadUp() {
    if (playheadDragMs === null) return;
    const target = playheadDragMs;
    // Hold the line at the drop position until the seek's state comes back.
    previewSeek(target).finally(() => {
      if (playheadDragMs === target) playheadDragMs = null;
    });
  }

  function handlePlayheadKeydown(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    previewSeek(playheadMs + (e.key === "ArrowLeft" ? -5000 : 5000));
  }

  function formatPlayheadMs(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    return `${Math.floor(totalSec / 60)}:${(totalSec % 60).toString().padStart(2, "0")}`;
  }

  /** Musical position + tempo/meter at the playhead, from the player's grid
   * and tempo map (the `tempo:` block's, else the one the grid itself
   * makes) — live while playing and while dragging. The readout rounds the
   * tempo to a tenth: inside a ramp it is a running value. */
  let playheadInfo = $derived.by(() => {
    const grid = song.beat_grid;
    if (!grid) return null;
    const at = rules.grid_query(grid, { secs: playheadMs / 1000 });
    if (!at) return null;
    const point = rules.tempo_query(tempo ? { config: tempo } : { grid }, {
      ms: playheadMs,
    });
    return {
      measure: at.measure,
      beat: Math.floor(at.beat),
      bpm: point ? Math.round(point.bpm * 10) / 10 : null,
      sig: point ? point.sig.join("/") : null,
    };
  });

  function msToProtoDuration(ms: number) {
    return {
      seconds: BigInt(Math.floor(ms / 1000)),
      nanos: Math.round((ms % 1000) * 1e6),
    };
  }

  /** Play/pause. The player has no native pause, so pausing stops playback
   * and stores the position as the pending seek; play resumes from it. */
  async function previewPlayPause() {
    try {
      if (isCurrentSong && $playbackStore.is_playing) {
        const at = smoothElapsedMs;
        await playerClient.stop({});
        await playerClient.seek({ position: msToProtoDuration(at) });
      } else if (isCurrentSong) {
        await playerClient.play({});
      } else {
        await playerClient.playSongFrom({
          songName: song.name,
          startTime: msToProtoDuration(0),
        });
      }
    } catch (e) {
      console.error("preview play/pause failed:", e);
    }
  }

  /** Full stop: also rewinds the resume point to the top. */
  async function previewStop() {
    try {
      if (isCurrentSong && $playbackStore.is_playing) {
        await playerClient.stop({});
      }
      if (isCurrentSong) {
        await playerClient.seek({ position: msToProtoDuration(0) });
      }
    } catch (e) {
      console.error("preview stop failed:", e);
    }
  }

  /** Ruler click: seek the preview (starting the song here if needed). */
  async function previewSeek(ms: number) {
    // Both ends: ArrowLeft near the top would otherwise send a negative
    // duration, which the backend rejects — a silent no-op instead of a
    // seek to the start.
    const clamped = Math.max(0, Math.min(ms, songDurationMs));
    try {
      if (isCurrentSong) {
        await playerClient.seek({ position: msToProtoDuration(clamped) });
      } else {
        await playerClient.playSongFrom({
          songName: song.name,
          startTime: msToProtoDuration(clamped),
        });
      }
    } catch (e) {
      console.error("preview seek failed:", e);
    }
  }

  // --- Section edit dialog ---

  let sectionDialogIndex = $state<number | null>(null);

  /** The nearest (measure, beat) at a timeline position, snapped to `snap`
   * of a beat by the player's own rounding — which re-asks the grid, so a
   * position three quarters through the last beat of a measure becomes the
   * next measure's beat 1, never this measure's beat "one past the end".
   * Used to capture the preview playhead as a section boundary. */
  function msToMeasureBeat(
    ms: number,
    snap = 0.5,
  ): { measure: number; beat: number } | null {
    const grid = song.beat_grid;
    const at = grid
      ? rules.grid_query(grid, { secs: ms / 1000, snap: { step: snap } })
      : null;
    return at ? { measure: at.measure, beat: at.beat } : null;
  }

  /** Playhead position as a section boundary, when this song is loaded. */
  let playheadPos = $derived(
    isCurrentSong ? msToMeasureBeat(playheadMs) : null,
  );
  /** The same playhead in seconds: what the dialogs mark and capture. */
  let playheadTime = $derived(isCurrentSong ? playheadMs / 1000 : null);

  /** Beat-snapping for edge drags: only when zoomed in far enough that
   * individual beats are visually distinct (the grid's average beat
   * spacing, in pixels — a zoom threshold, not a position rule); zoomed
   * out, edges keep snapping to measure lines (SectionBar falls back on
   * null). Edge drags snap to whole beats; fractions stay a dialog affair. */
  const BEAT_SNAP_MIN_PX = 14;
  function snapBoundary(ms: number): { measure: number; beat: number } | null {
    const grid = song.beat_grid;
    if (!grid || grid.beats.length < 2) return null;
    const avgBeatGapMs =
      ((grid.beats[grid.beats.length - 1] - grid.beats[0]) /
        (grid.beats.length - 1)) *
      1000;
    if (avgBeatGapMs * pixelsPerMs < BEAT_SNAP_MIN_PX) return null;
    return msToMeasureBeat(ms, 1);
  }

  function patchSection(index: number, patch: Partial<Section>) {
    const updated = [...sections];
    const merged = { ...updated[index], ...patch };
    // An empty name keeps the previous one; "auto" color drops the key.
    // Beat 1 is the measure boundary — drop the key to keep configs clean.
    if (!merged.name) merged.name = updated[index].name;
    if (!merged.color) delete merged.color;
    if (!merged.start_beat || merged.start_beat === 1) delete merged.start_beat;
    if (!merged.end_beat || merged.end_beat === 1) delete merged.end_beat;
    updated[index] = merged;
    handleSectionsChange(updated);
  }

  function deleteSection(index: number) {
    handleSectionsChange(sections.filter((_, i) => i !== index));
    sectionDialogIndex = null;
  }

  // --- Tempo / pilot marker layers ---

  let tempoDialogTarget = $state<"start" | MarkerPosition | null>(null);
  let pilotDialogIndex = $state<number | null>(null);

  /** Time (ms) of a measure/beat position on the beat grid: where the
   * player's `BeatGrid::beat_time` puts it, or the song's end for a position
   * the grid does not hold. */
  function measureBeatToMs(measure: number, beat: number): number {
    const grid = song.beat_grid;
    if (!grid) return 0;
    const at = rules.grid_query(grid, { measure, beat });
    return at ? at.secs * 1000 : songDurationMs;
  }

  /** Signature of the bar a position is in, as the player's tempo map has
   * it; the player's default metre while the tempo block makes no map. */
  function sigAt(measure: number, beat: number): [number, number] {
    const at = tempo
      ? rules.tempo_query({ config: tempo }, { measure, beat })
      : undefined;
    return at?.sig ?? rules.tempo_defaults().time_signature;
  }

  /** One marker per position, merging tempo changes with metronome feel
   * changes at the same measure. */
  let tempoMarkers = $derived.by(() => {
    if (!tempo) return [];
    const baseParts = [
      `${tempo.bpm}`,
      tempo.time_signature ?? rules.tempo_defaults().time_signature.join("/"),
    ];
    if (metronome?.accents?.length) {
      // The base pattern as the renderer sizes it to the metre the block
      // sets at bar 1 (`sig_set`: the one the pads are for).
      const opening =
        rules.tempo_query({ config: tempo }, { measure: 1, beat: 1 })
          ?.sig_set ?? sigAt(1, 1);
      baseParts.push(
        accentsGlyph(
          rules.accent_pattern({ ...metronome, changes: [] }, 1, opening[0]) ??
            [],
        ),
      );
    }
    if (metronome?.subdivision !== undefined && metronome.subdivision !== 1) {
      baseParts.push(subdivisionChip(metronome.subdivision, sigAt(1, 1)[1]));
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
      // The glyph is sized to the metre the marker sets (`sig_set`), as the
      // dialog's pads are; `sig` is the bar's, for the subdivision chip.
      const set =
        rules.tempo_query(
          { config: tempo },
          { measure: pos.measure, beat: pos.beat },
        )?.sig_set ?? sig;
      if (mc?.accents && metronome)
        parts.push(
          accentsGlyph(
            rules.accent_pattern(metronome, pos.measure, set[0]) ?? [],
          ),
        );
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

  /** The BPM in force at a measure's downbeat, as the player plays it
   * (inside a ramp, the ramp's value there), to the hundredth a `tempo:`
   * block is written to — a formatting choice for the new marker's value,
   * not a rule; the base tempo while the tempo block makes no map. */
  function effectiveBpmAt(measure: number): number {
    if (!tempo) return rules.tempo_defaults().bpm;
    const bpm = rules.tempo_query({ config: tempo }, { measure, beat: 1 })?.bpm;
    return bpm === undefined ? tempo.bpm : Math.round(bpm * 100) / 100;
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
      const { bpm, time_signature } = rules.tempo_defaults();
      ontempochange?.({ bpm, time_signature: time_signature.join("/") });
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
    // the backend rejects a non-ascending map, so the player's own ordering
    // puts the new change in its place.
    const entry = { measure, bpm: effectiveBpmAt(measure) };
    const appended = { ...tempo, changes: [...(tempo.changes ?? []), entry] };
    ontempochange?.(rules.tempo_normalize(appended)?.config ?? appended);
    tempoDialogTarget = { measure, beat: 1 };
  }

  /** Nearest beat as measure/beat for a position (ms), as the player snaps
   * a time to one; null off-grid. Beat 1 stays implicit. */
  function nearestBeatAt(
    ms: number,
  ): { measure: number; beat?: number } | null {
    const grid = song.beat_grid;
    const at = grid
      ? rules.grid_query(grid, { secs: ms / 1000, snap: { step: 1 } })
      : undefined;
    if (!at) return null;
    return at.beat > 1
      ? { measure: at.measure, beat: at.beat }
      : { measure: at.measure };
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

  function patchPilotHint(index: number, patch: Partial<PilotHint>) {
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
    <span class="toolbar-title">{$t("songs.detail.tabs.sections")}</span>
    {#if isCurrentSong && playheadInfo}
      <span class="playhead-info mono">
        <span class="playhead-info__pos"
          >m{playheadInfo.measure}.{playheadInfo.beat}</span
        >
        · {formatPlayheadMs(playheadMs)}
        {#if playheadInfo.bpm !== null}
          · {playheadInfo.bpm}
          {$t("tempo.bpm")}
        {/if}
        {#if playheadInfo.sig}
          · {playheadInfo.sig}
        {/if}
      </span>
    {/if}
    <div class="toolbar-controls">
      {#if !song.beat_grid}
        <span class="no-grid-warning"
          >No beat grid — add a tempo map or click track for measure-based
          sections</span
        >
      {/if}
      <button
        class="btn btn-sm preview-btn"
        class:preview-btn--live={isCurrentSong && $playbackStore.is_playing}
        onclick={previewPlayPause}
        title={isCurrentSong && $playbackStore.is_playing
          ? $t("sections.preview.pause")
          : $t("sections.preview.play")}
      >
        {isCurrentSong && $playbackStore.is_playing ? "⏸" : "▶"}
      </button>
      <button
        class="btn btn-sm preview-btn"
        onclick={previewStop}
        disabled={!isCurrentSong}
        title={$t("sections.preview.stop")}
      >
        ⏹
      </button>
      <button class="btn btn-sm" onclick={zoomOut} title="Zoom out">−</button>
      <button class="btn btn-sm" onclick={fitView} title="Fit to view"
        >Fit</button
      >
      <button class="btn btn-sm" onclick={zoomIn} title="Zoom in">+</button>
    </div>
  </div>

  <div class="timeline-wrap" bind:this={timelineWrapEl}>
    {#if isCurrentSong && playheadX >= LABEL_WIDTH}
      <div
        class="playhead"
        class:playhead--dragging={playheadDragMs !== null}
        style:left="{playheadX}px"
        role="slider"
        tabindex="0"
        aria-label={$t("sections.preview.playhead")}
        aria-valuemin={0}
        aria-valuemax={Math.floor(songDurationMs / 1000)}
        aria-valuenow={Math.floor(playheadMs / 1000)}
        aria-valuetext={formatPlayheadMs(playheadMs)}
        onpointerdown={handlePlayheadDown}
        onpointermove={handlePlayheadMove}
        onpointerup={handlePlayheadUp}
        onpointercancel={() => (playheadDragMs = null)}
        onkeydown={handlePlayheadKeydown}
      >
        {#if playheadDragMs !== null}
          <span class="playhead__time mono">{formatPlayheadMs(playheadMs)}</span
          >
        {/if}
      </div>
    {/if}
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
        beatToMs={measureBeatToMs}
        {snapBoundary}
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
        onseek={previewSeek}
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
    maxMeasure={measureTimesMs.length || 9999}
    {playheadTime}
    beatGrid={song.beat_grid}
    {lightShowTempos}
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
    maxMeasure={measureTimesMs.length || 9999}
    {tempo}
    beatGrid={song.beat_grid}
    posToMs={measureBeatToMs}
    {playheadPos}
    {playheadTime}
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
    {tempo}
    maxMeasure={measureTimesMs.length || 9999}
    {playheadTime}
    songName={songName ?? song.name}
    {songFiles}
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
  .playhead-info {
    font-size: 12px;
    color: var(--text-muted);
    margin-left: 12px;
    margin-right: auto;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .playhead-info__pos {
    color: var(--nc-amber-fg);
    font-weight: 600;
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
  .timeline-wrap {
    position: relative;
  }
  .timeline-scroll {
    overflow-x: auto;
    overflow-y: hidden;
    position: relative;
    max-height: 400px;
  }
  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    /* Wide invisible grab zone around the 2px visible line. */
    width: 14px;
    margin-left: -7px;
    background: transparent;
    z-index: 20;
    cursor: ew-resize;
    touch-action: none;
  }
  .playhead::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 6px;
    width: 2px;
    background: var(--nc-amber-fg);
    box-shadow: 0 0 6px color-mix(in srgb, var(--nc-amber-fg) 50%, transparent);
  }
  .playhead--dragging::before,
  .playhead:focus-visible::before {
    width: 3px;
    left: 5.5px;
    box-shadow: 0 0 10px color-mix(in srgb, var(--nc-amber-fg) 80%, transparent);
  }
  .playhead:focus-visible {
    outline: none;
  }
  .playhead__time {
    position: absolute;
    top: 2px;
    left: 12px;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 6px;
    background: var(--nc-amber-fg);
    color: var(--nc-bg-1);
    font-weight: 600;
    white-space: nowrap;
  }
  .preview-btn--live {
    color: var(--nc-amber-fg);
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
