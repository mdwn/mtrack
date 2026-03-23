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
    name: string;
    peaks: number[];
    songDurationMs: number;
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    /** Beat grid measure start times in ms for vertical grid lines */
    measureTimesMs?: number[];
  }

  let {
    name,
    peaks,
    songDurationMs,
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    measureTimesMs = [],
  }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();

  const LANE_HEIGHT = 40;
  const LABEL_WIDTH = 80;
  const WAVEFORM_COLOR = "rgba(94, 202, 234, 0.35)";
  const GRID_COLOR = "rgba(94, 202, 234, 0.12)";

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

    if (peaks.length === 0 || songDurationMs <= 0) return;

    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + w) / pixelsPerMs;
    const msPerPeak = songDurationMs / peaks.length;
    const mid = h / 2;

    // Draw measure grid lines.
    ctx.strokeStyle = GRID_COLOR;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (const ms of measureTimesMs) {
      if (ms < viewStartMs || ms > viewEndMs) continue;
      const x = ms * pixelsPerMs - scrollLeft;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
    }
    ctx.stroke();

    // Draw waveform.
    ctx.fillStyle = WAVEFORM_COLOR;
    ctx.beginPath();
    ctx.moveTo(0, mid);

    // Top half.
    for (let px = 0; px < w; px++) {
      const ms = viewStartMs + px / pixelsPerMs;
      const idx = Math.floor(ms / msPerPeak);
      if (idx < 0 || idx >= peaks.length) continue;
      // Average with neighbors for smoothing.
      let sum = peaks[idx];
      let count = 1;
      if (idx > 0) {
        sum += peaks[idx - 1];
        count++;
      }
      if (idx < peaks.length - 1) {
        sum += peaks[idx + 1];
        count++;
      }
      const amp = (sum / count) * (mid - 2);
      ctx.lineTo(px, mid - amp);
    }

    // Bottom half (mirror).
    for (let px = w - 1; px >= 0; px--) {
      const ms = viewStartMs + px / pixelsPerMs;
      const idx = Math.floor(ms / msPerPeak);
      if (idx < 0 || idx >= peaks.length) continue;
      let sum = peaks[idx];
      let count = 1;
      if (idx > 0) {
        sum += peaks[idx - 1];
        count++;
      }
      if (idx < peaks.length - 1) {
        sum += peaks[idx + 1];
        count++;
      }
      const amp = (sum / count) * (mid - 2);
      ctx.lineTo(px, mid + amp);
    }

    ctx.closePath();
    ctx.fill();

    // Center line.
    ctx.strokeStyle = "rgba(255,255,255,0.08)";
    ctx.lineWidth = 0.5;
    ctx.beginPath();
    ctx.moveTo(0, mid);
    ctx.lineTo(w, mid);
    ctx.stroke();
  }

  $effect(() => {
    void peaks;
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void measureTimesMs;
    draw();
  });
</script>

<div class="waveform-lane" style:height="{LANE_HEIGHT}px">
  <div class="lane-label" style:width="{LABEL_WIDTH}px">{name}</div>
  <div class="lane-content">
    <canvas bind:this={canvasEl} class="waveform-canvas"></canvas>
  </div>
</div>

<style>
  .waveform-lane {
    display: flex;
    border-bottom: 1px solid var(--border);
    position: sticky;
    left: 0;
  }
  .lane-label {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-dim);
    border-right: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lane-content {
    flex: 1;
    position: relative;
    overflow: hidden;
  }
  .waveform-canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
