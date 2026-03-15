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
  import type { TempoSection } from "../../../lib/lighting/types";
  import {
    msToPixel,
    getGridLines,
    formatMs,
  } from "../../../lib/lighting/timeline-state";

  interface Props {
    pixelsPerMs: number;
    totalDurationMs: number;
    tempo?: TempoSection;
    scrollLeft: number;
    viewportWidth: number;
    onclick?: (ms: number) => void;
  }

  let {
    pixelsPerMs,
    totalDurationMs,
    tempo,
    scrollLeft,
    viewportWidth,
    onclick,
  }: Props = $props();

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

    // Visible time range
    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + viewportWidth) / pixelsPerMs;

    // Draw absolute time ticks
    const tickIntervalMs = chooseTickInterval(pixelsPerMs);
    const firstTick = Math.floor(viewStartMs / tickIntervalMs) * tickIntervalMs;

    ctx.fillStyle = "#555";
    ctx.font = "10px monospace";
    ctx.textAlign = "center";

    for (let t = firstTick; t <= viewEndMs; t += tickIntervalMs) {
      const x = msToPixel(t, pixelsPerMs) - scrollLeft;
      // Major tick
      ctx.strokeStyle = "#444";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, h - 12);
      ctx.lineTo(x, h);
      ctx.stroke();

      // Sub-ticks
      const subCount = 4;
      const subInterval = tickIntervalMs / subCount;
      for (let s = 1; s < subCount; s++) {
        const sx = msToPixel(t + s * subInterval, pixelsPerMs) - scrollLeft;
        ctx.strokeStyle = "#333";
        ctx.beginPath();
        ctx.moveTo(sx, h - 6);
        ctx.lineTo(sx, h);
        ctx.stroke();
      }

      // Label
      ctx.fillStyle = "#888";
      ctx.fillText(formatMs(t), x, h - 16);
    }

    // Draw measure/beat grid if tempo exists
    if (tempo) {
      const gridLines = getGridLines(tempo, viewStartMs, viewEndMs);
      for (const line of gridLines) {
        const x = msToPixel(line.ms, pixelsPerMs) - scrollLeft;
        if (line.type === "measure") {
          ctx.strokeStyle = "rgba(91, 91, 214, 0.5)";
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x, 14);
          ctx.stroke();

          if (line.label) {
            ctx.fillStyle = "rgba(91, 91, 214, 0.9)";
            ctx.font = "9px monospace";
            ctx.textAlign = "center";
            ctx.fillText(line.label, x, 10);
          }
        } else {
          ctx.strokeStyle = "rgba(91, 91, 214, 0.2)";
          ctx.lineWidth = 0.5;
          ctx.beginPath();
          ctx.moveTo(x, 4);
          ctx.lineTo(x, 14);
          ctx.stroke();
        }
      }
    }

    // Bottom border
    ctx.strokeStyle = "#333";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, h - 0.5);
    ctx.lineTo(w, h - 0.5);
    ctx.stroke();
  }

  /** Choose a human-friendly tick interval based on zoom. */
  function chooseTickInterval(pxPerMs: number): number {
    const candidates = [
      100, 200, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000,
    ];
    const minPixelsBetweenTicks = 80;
    for (const c of candidates) {
      if (c * pxPerMs >= minPixelsBetweenTicks) return c;
    }
    return 60000;
  }

  function handleClick(e: MouseEvent) {
    if (!onclick || !canvasEl) return;
    const rect = canvasEl.getBoundingClientRect();
    const x = e.clientX - rect.left + scrollLeft;
    const ms = x / pixelsPerMs;
    onclick(Math.max(0, ms));
  }

  $effect(() => {
    // Re-draw when any dependency changes
    void pixelsPerMs;
    void totalDurationMs;
    void tempo;
    void scrollLeft;
    void viewportWidth;
    draw();
  });
</script>

<canvas bind:this={canvasEl} class="ruler-canvas" onclick={handleClick}
></canvas>

<style>
  .ruler-canvas {
    display: block;
    width: 100%;
    height: 36px;
    cursor: pointer;
  }
</style>
