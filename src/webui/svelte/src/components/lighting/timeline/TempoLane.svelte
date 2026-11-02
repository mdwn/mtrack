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
    msToPixel,
    offsetTimeline,
    tempoLaneSpans,
    type OffsetMarker,
  } from "../../../lib/lighting/timeline-state";
  import { effectiveTheme } from "../../../lib/theme";

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

  // The cumulative offset at a raw ms position, as the cues have it
  let cumulativeOffsetAt = $derived(offsetTimeline(offsets));

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

    // Light-mode text on a light-cyan tint needs a darker hue to keep
    // contrast above WCAG-AA. Dark mode keeps the brighter cyan.
    const isDark = document.documentElement.classList.contains("nc--dark");
    const labelColor = isDark
      ? "rgba(94, 202, 234, 0.95)"
      : "rgba(22, 106, 130, 0.95)"; // cyan-700
    const borderColor = isDark
      ? "rgba(94, 202, 234, 0.3)"
      : "rgba(31, 143, 175, 0.4)"; // cyan-600

    const spans = tempoLaneSpans(tempo);
    const colors = ["rgba(94, 202, 234, 0.12)", "rgba(139, 92, 246, 0.12)"];

    for (let i = 0; i < spans.length; i++) {
      const span = spans[i];
      const rawNextMs =
        i < spans.length - 1 ? spans[i + 1].startMs : totalDurationMs;
      const adjustedStart = span.startMs + cumulativeOffsetAt(span.startMs);
      const adjustedEnd = rawNextMs + cumulativeOffsetAt(rawNextMs);
      const x1 = msToPixel(adjustedStart, pixelsPerMs) - scrollLeft;
      const x2 = msToPixel(adjustedEnd, pixelsPerMs) - scrollLeft;

      // Skip spans entirely outside viewport
      if (x2 < 0 || x1 > w) continue;

      // Draw span block
      ctx.fillStyle = colors[i % 2];
      ctx.fillRect(Math.max(0, x1), 0, Math.min(x2, w) - Math.max(0, x1), h);

      // Span border
      if (x1 >= 0 && x1 <= w) {
        ctx.strokeStyle = borderColor;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x1, 0);
        ctx.lineTo(x1, h);
        ctx.stroke();
      }

      // Label
      const labelX = Math.max(x1 + 6, 4);
      if (labelX < x2 - 20 && labelX < w) {
        ctx.fillStyle = labelColor;
        ctx.font = "12px monospace";
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        ctx.fillText(span.label, labelX, h / 2);
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
    void $effectiveTheme;
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
    border-bottom: 1px solid var(--card-border);
  }
  .tempo-label {
    width: 100px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 10px;
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 11px;
    line-height: 1.2;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--nc-fg-3);
    border-right: 1px solid var(--card-border);
    background: var(--card-bg);
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
