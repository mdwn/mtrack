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
    Cue,
    TempoSection,
    Timestamp,
    SubLaneType,
  } from "../../../lib/lighting/types";
  import { emptyCue, absoluteTimestamp } from "../../../lib/lighting/types";
  import {
    msToPixel,
    timestampToMs,
    msToTimestamp,
    getGridLines,
    getAdjustedGridLines,
    snapToNearestGrid,
    computeAdjustedCuePositions,
    offsetMeasuresToMs,
    durationStringToMs,
    getSequenceIterationMs,
    type OffsetMarker,
  } from "../../../lib/lighting/timeline-state";
  import CueBlock from "./CueBlock.svelte";

  interface Props {
    name: string;
    cues: Cue[];
    laneType: "show" | "sequence";
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    tempo?: TempoSection;
    selectedCueIndex: number | null;
    snapEnabled: boolean;
    snapResolution: "beat" | "measure";
    onselect: (index: number) => void;
    oncuechange: (index: number, cue: Cue) => void;
    oncuedelete: (index: number) => void;
    oncueadd: (cue: Cue) => void;
    ondelete: () => void;
    oneffectresize?: (cueIndex: number, newDurationStr: string) => void;
    onloopchange?: (cueIndex: number, newLoopCount: number) => void;
    subLaneType?: SubLaneType;
    laneHeight?: number;
    hideLabel?: boolean;
    offsets?: OffsetMarker[];
    playheadMs?: number | null;
    sequenceDefs?: import("../../../lib/lighting/types").Sequence[];
  }

  let {
    name,
    cues,
    laneType,
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    tempo,
    selectedCueIndex,
    snapEnabled,
    snapResolution,
    onselect,
    oncuechange,
    oncuedelete,
    oncueadd,
    ondelete,
    subLaneType,
    laneHeight,
    hideLabel = false,
    offsets = [],
    playheadMs = null,
    oneffectresize,
    onloopchange,
    sequenceDefs = [],
  }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let laneEl: HTMLDivElement | undefined = $state();

  // Compute cue positions with cumulative offset adjustment
  function getCuePositions() {
    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + viewportWidth) / pixelsPerMs;
    const margin = 100 / pixelsPerMs;

    // Apply cumulative offsets for shows (sequences don't use offsets)
    const adjusted =
      laneType === "show"
        ? computeAdjustedCuePositions(cues, tempo)
        : cues.map((cue, index) => ({
            cue,
            index,
            adjustedMs: timestampToMs(cue.timestamp, tempo),
            cumulativeOffsetMs: 0,
          }));

    return adjusted
      .map((ap) => {
        const ms = ap.adjustedMs;
        const px = msToPixel(ms, pixelsPerMs) - scrollLeft;
        // Estimate block end from the longest matching effect's duration
        let blockEndMs = ms + 500;
        const layerFilter = subLaneType?.startsWith("effects:")
          ? subLaneType.split(":")[1]
          : undefined;
        let maxDurMs = 0;
        for (const eff of ap.cue.effects) {
          if (layerFilter && (eff.effect.layer ?? "background") !== layerFilter)
            continue;
          const durStr = eff.effect.duration ?? eff.effect.extra?.hold_time;
          const durMs = durationStringToMs(durStr, tempo, ms);
          if (durMs > maxDurMs) maxDurMs = durMs;
        }
        if (maxDurMs > 0) blockEndMs = ms + maxDurMs;
        // For sequence refs, compute total expanded duration
        if (subLaneType === "sequences") {
          for (const ref of ap.cue.sequences) {
            if (ref.stop) continue;
            const def = sequenceDefs.find((s) => s.name === ref.name);
            if (!def) continue;
            const iterMs = getSequenceIterationMs(def, ms, tempo);
            const loopCount = ref.loop ? parseInt(ref.loop, 10) || 1 : 1;
            const totalMs = iterMs * loopCount;
            if (ms + totalMs > blockEndMs) blockEndMs = ms + totalMs;
          }
        }
        // Visible if any part of the block overlaps the viewport
        const visible =
          blockEndMs >= viewStartMs - margin && ms <= viewEndMs + margin;
        return {
          cue: ap.cue,
          index: ap.index,
          ms,
          px,
          visible,
          cumulativeOffsetMs: ap.cumulativeOffsetMs,
        };
      })
      .filter(({ cue: c }) => {
        if (!subLaneType) return true;
        if (subLaneType === "effects") return c.effects.length > 0;
        if (subLaneType === "commands") return c.commands.length > 0;
        if (subLaneType === "sequences") return c.sequences.length > 0;
        if (subLaneType.startsWith("effects:")) {
          const layer = subLaneType.split(":")[1]; // "background", "midground", "foreground"
          return c.effects.some(
            (e) => (e.effect.layer ?? "background") === layer,
          );
        }
        return true;
      });
  }

  // Find the cumulative offset in ms at an adjusted-ms position
  function getCumulativeOffsetAtMs(adjustedMs: number): number {
    if (laneType !== "show") return 0;
    const adjusted = computeAdjustedCuePositions(cues, tempo);
    // Walk backwards to find the last cue at or before this position
    for (let i = adjusted.length - 1; i >= 0; i--) {
      if (adjusted[i].adjustedMs <= adjustedMs) {
        // Use the next cue's cumulativeOffsetMs if available (it includes
        // any offset from cue[i]). Otherwise compute it manually.
        if (i + 1 < adjusted.length) {
          return adjusted[i + 1].cumulativeOffsetMs;
        }
        let total = adjusted[i].cumulativeOffsetMs;
        if (
          adjusted[i].cue.offset_measures &&
          adjusted[i].cue.offset_measures! > 0 &&
          tempo
        ) {
          total += offsetMeasuresToMs(
            adjusted[i].cue.offset_measures!,
            adjusted[i].adjustedMs,
            tempo,
          );
        }
        return total;
      }
    }
    return 0;
  }

  function handleDblClick(e: MouseEvent) {
    if (!laneEl) return;
    const rect = laneEl.getBoundingClientRect();
    const x = e.clientX - rect.left + scrollLeft;
    let adjustedMs = x / pixelsPerMs;
    adjustedMs = Math.max(0, adjustedMs);

    // Subtract cumulative offset to get the raw timestamp ms
    const offsetMs = getCumulativeOffsetAtMs(adjustedMs);
    let rawMs = adjustedMs - offsetMs;
    rawMs = Math.max(0, rawMs);

    if (snapEnabled && tempo) {
      rawMs = snapToNearestGrid(rawMs, tempo, snapResolution);
    }

    const ts: Timestamp = tempo
      ? msToTimestamp(rawMs, "measure_beat", tempo)
      : absoluteTimestamp(rawMs);

    const newCue = emptyCue(ts);
    if (subLaneType === "effects") {
      newCue.effects = [
        {
          groups: ["all"],
          effect: { type: "static", colors: [], duration: "5s", extra: {} },
        },
      ];
    } else if (subLaneType === "commands") {
      newCue.commands = [{ command: "clear" }];
    } else if (subLaneType === "sequences") {
      newCue.sequences = [{ name: "" }];
    }
    oncueadd(newCue);
  }

  function handleCueMove(index: number, deltaMs: number) {
    const cue = cues[index];
    // Move operates on raw timestamps (offset is structural, not per-cue position)
    const currentMs = timestampToMs(cue.timestamp, tempo);
    let newMs = Math.max(0, currentMs + deltaMs);

    if (snapEnabled && tempo) {
      newMs = snapToNearestGrid(newMs, tempo, snapResolution);
    }

    const newTs: Timestamp =
      cue.timestamp.type === "measure_beat" && tempo
        ? msToTimestamp(newMs, "measure_beat", tempo)
        : absoluteTimestamp(newMs);

    oncuechange(index, { ...cue, timestamp: newTs });
  }

  // Draw grid lines on the canvas background
  function drawGrid() {
    if (!canvasEl) return;
    const dpr = window.devicePixelRatio || 1;
    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    if (w <= 0 || h <= 0) return;
    canvasEl.width = w * dpr;
    canvasEl.height = h * dpr;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    if (!tempo) return;

    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + viewportWidth) / pixelsPerMs;
    const gridLines =
      offsets.length > 0
        ? getAdjustedGridLines(tempo, viewStartMs, viewEndMs, offsets)
        : getGridLines(tempo, viewStartMs, viewEndMs);

    for (const line of gridLines) {
      const x = msToPixel(line.ms, pixelsPerMs) - scrollLeft;
      ctx.strokeStyle =
        line.type === "measure"
          ? "rgba(94, 202, 234, 0.15)"
          : "rgba(94, 202, 234, 0.06)";
      ctx.lineWidth = line.type === "measure" ? 1 : 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    }

    // Playhead line
    if (playheadMs !== null && playheadMs !== undefined) {
      const px = msToPixel(playheadMs, pixelsPerMs) - scrollLeft;
      if (px >= 0 && px <= w) {
        ctx.strokeStyle = "#22c55e";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(px, 0);
        ctx.lineTo(px, h);
        ctx.stroke();
      }
    }
  }

  $effect(() => {
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void tempo;
    void offsets;
    void playheadMs;
    drawGrid();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="show-lane"
  class:sequence={laneType === "sequence"}
  style:height={laneHeight ? `${laneHeight}px` : undefined}
>
  {#if !hideLabel}
    <div class="lane-label">
      <span class="lane-name" title={name}>{name}</span>
      <button
        class="btn-icon lane-delete"
        title={$t("timeline.showLane.deleteLane", {
          values: { type: laneType },
        })}
        onclick={ondelete}
      >
        &#10005;
      </button>
    </div>
  {/if}
  <div class="lane-content" bind:this={laneEl} ondblclick={handleDblClick}>
    <canvas bind:this={canvasEl} class="lane-grid-canvas"></canvas>
    {#each getCuePositions() as cp (cp.index)}
      {#if cp.visible}
        <CueBlock
          cue={cp.cue}
          positionPx={cp.px}
          selected={selectedCueIndex === cp.index}
          {pixelsPerMs}
          {subLaneType}
          {tempo}
          cueMs={cp.ms}
          onselect={() => onselect(cp.index)}
          onmove={(deltaMs) => handleCueMove(cp.index, deltaMs)}
          ondelete={() => oncuedelete(cp.index)}
          onresize={oneffectresize
            ? (dur) => oneffectresize(cp.index, dur)
            : undefined}
          onloopchange={onloopchange
            ? (count) => onloopchange(cp.index, count)
            : undefined}
          {sequenceDefs}
        />
      {/if}
    {/each}
  </div>
</div>

<style>
  .show-lane {
    display: flex;
    height: 52px;
    border-bottom: 1px solid var(--border);
  }
  .show-lane.sequence {
    background: rgba(239, 96, 163, 0.03);
  }
  .lane-label {
    width: 80px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    padding: 0 8px;
    gap: 2px;
    border-right: 1px solid var(--border);
    background: var(--bg-card);
  }
  .lane-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .lane-delete {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 13px;
    padding: 1px 3px;
    border-radius: 3px;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .show-lane:hover .lane-delete {
    opacity: 1;
  }
  .lane-delete:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .lane-content {
    flex: 1;
    position: relative;
    min-width: 0;
    overflow: hidden;
  }
  .lane-grid-canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
</style>
