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
  import { effectiveTheme } from "../../lib/theme";
  interface Props {
    songDurationMs: number;
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    /** Beat grid measure start times in ms */
    measureTimesMs?: number[];
    /** Click on the ruler, in song ms — the preview scrub target. */
    onseek?: (ms: number) => void;
  }

  let {
    // eslint-disable-next-line @typescript-eslint/no-unused-vars -- kept for API consistency
    songDurationMs,
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    measureTimesMs = [],
    onseek,
  }: Props = $props();

  function handleClick(e: MouseEvent) {
    if (!onseek) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const ms = (scrollLeft + e.clientX - rect.left) / pixelsPerMs;
    onseek(Math.max(0, ms));
  }

  let canvasEl: HTMLCanvasElement | undefined = $state();

  const RULER_HEIGHT = 34;
  // Two bands: measures on top, clock underneath. Sharing one row put a
  // measure number and a timestamp within 5px of each other, which read as
  // one jumbled line at any useful zoom.
  const MEASURE_BASELINE = 10;
  const MEASURE_TICK_TOP = 12;
  const TIME_BASELINE = 26;
  const TIME_TICK_TOP = 28;
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

  /** A CSS custom property as a concrete colour the canvas can paint with. */
  function token(name: string, fallback: string): string {
    if (!canvasEl) return fallback;
    const value = getComputedStyle(canvasEl).getPropertyValue(name).trim();
    return value || fallback;
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

    // Themed, not hardcoded white: the clock band was invisible in light mode.
    const dim = token("--text-dim", "rgba(255,255,255,0.5)");
    const accent = token("--accent", "rgb(94, 202, 234)");
    ctx.strokeStyle = dim;
    ctx.fillStyle = dim;
    ctx.font = "10px var(--mono, monospace)";
    ctx.textAlign = "center";
    ctx.lineWidth = 1;

    for (let t = firstTick; t <= viewEndMs; t += tickInterval) {
      if (t < 0) continue;
      const x = t * pixelsPerMs - scrollLeft;

      ctx.beginPath();
      ctx.moveTo(x, TIME_TICK_TOP);
      ctx.lineTo(x, h);
      ctx.stroke();

      ctx.fillText(formatTime(t), x, TIME_BASELINE);
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
        ctx.strokeStyle = accent;
        ctx.globalAlpha = isLabeled ? 0.55 : 0.25;
        ctx.beginPath();
        ctx.moveTo(x, MEASURE_TICK_TOP);
        ctx.lineTo(x, isLabeled ? MEASURE_TICK_TOP + 9 : MEASURE_TICK_TOP + 4);
        ctx.stroke();

        // Label only at stride intervals.
        if (isLabeled) {
          ctx.globalAlpha = 1;
          ctx.fillStyle = accent;
          ctx.fillText(`${i + 1}`, x, MEASURE_BASELINE);
        }
        ctx.globalAlpha = 1;
      }
    }

    // Bottom border.
    ctx.strokeStyle = token("--border", "rgba(255,255,255,0.15)");
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(0, h - 0.5);
    ctx.lineTo(w, h - 0.5);
    ctx.stroke();
  }

  $effect(() => {
    void $effectiveTheme;
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void measureTimesMs;
    draw();
  });
</script>

<div class="ruler" style:height="{RULER_HEIGHT}px">
  <div class="label-spacer" style:width="{LABEL_WIDTH}px"></div>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ruler-content"
    class:ruler-content--seekable={!!onseek}
    onclick={handleClick}
  >
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
  .ruler-content--seekable {
    cursor: pointer;
  }
  .ruler-canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
