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
    peaks: number[];
    songDurationMs: number;
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
  }

  let { peaks, songDurationMs, pixelsPerMs, scrollLeft, viewportWidth }: Props =
    $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();

  function draw() {
    if (!canvasEl || peaks.length === 0 || songDurationMs <= 0) return;
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

    const msPerPeak = songDurationMs / peaks.length;
    const viewStartMs = scrollLeft / pixelsPerMs;
    const mid = h / 2;

    ctx.fillStyle = "rgba(91, 91, 214, 0.4)";
    ctx.beginPath();
    ctx.moveTo(0, mid);

    // Draw top half
    for (let px = 0; px < w; px++) {
      const ms = viewStartMs + px / pixelsPerMs;
      if (ms > songDurationMs) break;
      const peakIdx = Math.floor(ms / msPerPeak);
      // Average nearby peaks for smooth rendering
      const startIdx = Math.max(0, peakIdx - 1);
      const endIdx = Math.min(peaks.length - 1, peakIdx + 1);
      let maxPeak = 0;
      for (let i = startIdx; i <= endIdx; i++) {
        maxPeak = Math.max(maxPeak, peaks[i]);
      }
      const amp = maxPeak * (mid - 2);
      ctx.lineTo(px, mid - amp);
    }

    // Draw bottom half (mirror)
    for (let px = w - 1; px >= 0; px--) {
      const ms = viewStartMs + px / pixelsPerMs;
      if (ms > songDurationMs) {
        ctx.lineTo(px, mid);
        continue;
      }
      const peakIdx = Math.floor(ms / msPerPeak);
      const startIdx = Math.max(0, peakIdx - 1);
      const endIdx = Math.min(peaks.length - 1, peakIdx + 1);
      let maxPeak = 0;
      for (let i = startIdx; i <= endIdx; i++) {
        maxPeak = Math.max(maxPeak, peaks[i]);
      }
      const amp = maxPeak * (mid - 2);
      ctx.lineTo(px, mid + amp);
    }

    ctx.closePath();
    ctx.fill();

    // Center line
    ctx.strokeStyle = "rgba(91, 91, 214, 0.2)";
    ctx.lineWidth = 0.5;
    ctx.beginPath();
    ctx.moveTo(0, mid);
    ctx.lineTo(w, mid);
    ctx.stroke();
  }

  $effect(() => {
    void peaks;
    void songDurationMs;
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    draw();
  });
</script>

<div class="waveform-lane">
  <div class="lane-label">Waveform</div>
  <canvas bind:this={canvasEl} class="waveform-canvas"></canvas>
</div>

<style>
  .waveform-lane {
    display: flex;
    height: 48px;
    border-bottom: 1px solid var(--border);
  }
  .lane-label {
    width: 80px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px;
    font-size: 10px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    border-right: 1px solid var(--border);
    background: var(--bg-card);
  }
  .waveform-canvas {
    display: block;
    flex: 1;
    height: 100%;
    min-width: 0;
  }
</style>
