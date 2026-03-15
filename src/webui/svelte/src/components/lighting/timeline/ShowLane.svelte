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
    TempoSection,
    Timestamp,
  } from "../../../lib/lighting/types";
  import { emptyCue, absoluteTimestamp } from "../../../lib/lighting/types";
  import {
    msToPixel,
    timestampToMs,
    msToTimestamp,
    getGridLines,
    snapToNearestGrid,
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
  }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let laneEl: HTMLDivElement | undefined = $state();

  // Compute cue positions — plain function to avoid deep reactive proxy traversal
  function getCuePositions() {
    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + viewportWidth) / pixelsPerMs;
    const margin = 100 / pixelsPerMs;

    return cues.map((cue, index) => {
      const ms = timestampToMs(cue.timestamp, tempo);
      const px = msToPixel(ms, pixelsPerMs) - scrollLeft;
      const visible = ms >= viewStartMs - margin && ms <= viewEndMs + margin;
      return { cue, index, ms, px, visible };
    });
  }

  function handleDblClick(e: MouseEvent) {
    if (!laneEl) return;
    const rect = laneEl.getBoundingClientRect();
    const x = e.clientX - rect.left + scrollLeft;
    let ms = x / pixelsPerMs;
    ms = Math.max(0, ms);

    if (snapEnabled && tempo) {
      ms = snapToNearestGrid(ms, tempo, snapResolution);
    }

    // Use measure_beat if the file has tempo, otherwise absolute
    const ts: Timestamp = tempo
      ? msToTimestamp(ms, "measure_beat", tempo)
      : absoluteTimestamp(ms);

    oncueadd(emptyCue(ts));
  }

  function handleCueMove(index: number, deltaMs: number) {
    const cue = cues[index];
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
    const gridLines = getGridLines(tempo, viewStartMs, viewEndMs);

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
  }

  $effect(() => {
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void tempo;
    drawGrid();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="show-lane" class:sequence={laneType === "sequence"}>
  <div class="lane-label">
    <span class="lane-name" title={name}>{name}</span>
    <button
      class="btn-icon lane-delete"
      title="Delete {laneType}"
      onclick={ondelete}
    >
      &#10005;
    </button>
  </div>
  <div class="lane-content" bind:this={laneEl} ondblclick={handleDblClick}>
    <canvas bind:this={canvasEl} class="lane-grid-canvas"></canvas>
    {#each getCuePositions() as cp (cp.index)}
      {#if cp.visible}
        <CueBlock
          cue={cp.cue}
          positionPx={cp.px}
          selected={selectedCueIndex === cp.index}
          {pixelsPerMs}
          onselect={() => onselect(cp.index)}
          onmove={(deltaMs) => handleCueMove(cp.index, deltaMs)}
          ondelete={() => oncuedelete(cp.index)}
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
    font-size: 11px;
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
    font-size: 10px;
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
