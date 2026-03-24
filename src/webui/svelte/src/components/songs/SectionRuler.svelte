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
  interface Props {
    songDurationMs: number;
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    /** Beat grid measure start times in ms */
    measureTimesMs?: number[];
  }

  let {
    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- kept for API consistency
    songDurationMs,
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    measureTimesMs = [],
  }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();

  const RULER_HEIGHT = 28;
  const LABEL_WIDTH = 80;

  function formatTime(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    const min = Math.floor(totalSec / 60);
    const sec = totalSec % 60;
    return `${min}:${sec.toString().padStart(2, "0")}`;
  }

  function chooseTickInterval(pxPerMs: number): number {
    const candidates = [
      100, 200, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000,
    ];
    for (const c of candidates) {
      if (c * pxPerMs >= 40) return c;
    }
    return 60000;
  }

  function draw() {
    if (!canvasEl) return;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    canvasEl.width = w * dpr;
    canvasEl.height = h * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + w) / pixelsPerMs;

    // Time ticks.
    const tickInterval = chooseTickInterval(pixelsPerMs);
    const firstTick = Math.floor(viewStartMs / tickInterval) * tickInterval;

    ctx.strokeStyle = "rgba(255,255,255,0.3)";
    ctx.fillStyle = "rgba(255,255,255,0.5)";
    ctx.font = "10px var(--mono, monospace)";
    ctx.textAlign = "center";
    ctx.lineWidth = 1;

    for (let t = firstTick; t <= viewEndMs; t += tickInterval) {
      if (t < 0) continue;
      const x = t * pixelsPerMs - scrollLeft;

      // Major tick.
      ctx.beginPath();
      ctx.moveTo(x, h - 10);
      ctx.lineTo(x, h);
      ctx.stroke();

      // Label.
      ctx.fillText(formatTime(t), x, h - 13);
    }

    // Measure markers from beat grid with density-based thinning.
    if (measureTimesMs.length > 1) {
      // Compute the median pixel gap between adjacent measures.
      const avgGapMs =
        (measureTimesMs[measureTimesMs.length - 1] - measureTimesMs[0]) /
        (measureTimesMs.length - 1);
      const avgGapPx = avgGapMs * pixelsPerMs;

      // Choose stride: power of 2 so labeled measures stay musically meaningful.
      // Target: at least MIN_LABEL_GAP pixels between labeled measures.
      const MIN_LABEL_GAP = 32;
      let stride = 1;
      while (avgGapPx * stride < MIN_LABEL_GAP) {
        stride *= 2;
      }

      ctx.font = "9px var(--mono, monospace)";
      ctx.textAlign = "center";
      ctx.lineWidth = 1;

      for (let i = 0; i < measureTimesMs.length; i++) {
        const ms = measureTimesMs[i];
        if (ms < viewStartMs || ms > viewEndMs) continue;
        const x = ms * pixelsPerMs - scrollLeft;
        const isLabeled = i % stride === 0;

        // Tick line — labeled measures get full height, others get a shorter tick.
        ctx.strokeStyle = isLabeled
          ? "rgba(94, 202, 234, 0.5)"
          : "rgba(94, 202, 234, 0.2)";
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, isLabeled ? 12 : 6);
        ctx.stroke();

        // Label only at stride intervals.
        if (isLabeled) {
          ctx.fillStyle = "rgba(94, 202, 234, 0.9)";
          ctx.fillText(`${i + 1}`, x, 10);
        }
      }
    }

    // Bottom border.
    ctx.strokeStyle = "var(--border)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, h - 0.5);
    ctx.lineTo(w, h - 0.5);
    ctx.stroke();
  }

  $effect(() => {
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void measureTimesMs;
    draw();
  });
</script>

<div class="ruler" style:height="{RULER_HEIGHT}px">
  <div class="label-spacer" style:width="{LABEL_WIDTH}px"></div>
  <div class="ruler-content">
    <canvas bind:this={canvasEl} class="ruler-canvas"></canvas>
  </div>
</div>

<style>
  .ruler {
    display: flex;
    position: sticky;
    top: 0;
    left: 0;
    z-index: 10;
    background: var(--bg);
  }
  .label-spacer {
    flex-shrink: 0;
    border-right: 1px solid var(--border);
  }
  .ruler-content {
    flex: 1;
    position: relative;
    overflow: hidden;
  }
  .ruler-canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
