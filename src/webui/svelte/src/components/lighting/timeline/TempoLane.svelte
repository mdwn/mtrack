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
  import type { TempoSection } from "../../../lib/lighting/types";
  import {
    buildTempoSegments,
    msToPixel,
    type OffsetMarker,
  } from "../../../lib/lighting/timeline-state";

  interface Props {
    tempo: TempoSection;
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    totalDurationMs: number;
    offsets?: OffsetMarker[];
  }

  let {
    tempo,
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    totalDurationMs,
    offsets = [],
  }: Props = $props();

  // Compute cumulative offset at a raw ms position
  function cumulativeOffsetAt(rawMs: number): number {
    let cumulative = 0;
    for (const off of offsets) {
      if (off.type !== "offset" || off.durationMs <= 0) continue;
      if (off.rawMs > rawMs) break;
      cumulative += off.durationMs;
    }
    return cumulative;
  }

  let canvasEl: HTMLCanvasElement | undefined = $state();

  function draw() {
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

    const segments = buildTempoSegments(tempo);
    const colors = ["rgba(94, 202, 234, 0.12)", "rgba(139, 92, 246, 0.12)"];

    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      const rawNextMs =
        i < segments.length - 1 ? segments[i + 1].startMs : totalDurationMs;
      const adjustedStart = seg.startMs + cumulativeOffsetAt(seg.startMs);
      const adjustedEnd = rawNextMs + cumulativeOffsetAt(rawNextMs);
      const x1 = msToPixel(adjustedStart, pixelsPerMs) - scrollLeft;
      const x2 = msToPixel(adjustedEnd, pixelsPerMs) - scrollLeft;

      // Skip segments entirely outside viewport
      if (x2 < 0 || x1 > w) continue;

      // Draw segment block
      ctx.fillStyle = colors[i % 2];
      ctx.fillRect(Math.max(0, x1), 0, Math.min(x2, w) - Math.max(0, x1), h);

      // Segment border
      if (x1 >= 0 && x1 <= w) {
        ctx.strokeStyle = "rgba(94, 202, 234, 0.3)";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x1, 0);
        ctx.lineTo(x1, h);
        ctx.stroke();
      }

      // Label
      const labelX = Math.max(x1 + 6, 4);
      if (labelX < x2 - 20 && labelX < w) {
        const ts = seg.beatsPerMeasure + "/" + seg.beatValue;
        const label = `${seg.bpm} BPM, ${ts}`;
        ctx.fillStyle = "rgba(94, 202, 234, 0.8)";
        ctx.font = "12px monospace";
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        ctx.fillText(label, labelX, h / 2);
      }
    }
  }

  $effect(() => {
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void totalDurationMs;
    void tempo;
    void offsets;
    draw();
  });
</script>

<div class="tempo-lane">
  <div class="tempo-label">{$t("timeline.tempo")}</div>
  <div class="tempo-content">
    <canvas bind:this={canvasEl} class="tempo-canvas"></canvas>
  </div>
</div>

<style>
  .tempo-lane {
    display: flex;
    height: 24px;
    border-bottom: 1px solid var(--border);
  }
  .tempo-label {
    width: 80px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px;
    font-size: 12px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    border-right: 1px solid var(--border);
    background: var(--bg-card);
  }
  .tempo-content {
    flex: 1;
    min-width: 0;
    position: relative;
  }
  .tempo-canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
