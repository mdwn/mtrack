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
  import {
    metadataStore,
    fixtureStore,
    effectsStore,
    venueStore,
    poseStore,
  } from "../../../lib/ws/stores";
  import type {
    FixtureChannels,
    FixtureMetadata,
    VenueMetadata,
  } from "../../../lib/ws/stores";
  import {
    beamEnd,
    drawBeam,
    fitFrame,
    hasGeometry,
    positionalLayout,
    tagLayout,
    toPx,
    trayLayout,
    type Pt,
    type Rect,
    type StageFrame,
  } from "../../../lib/stage/layout";

  const FIXTURE_RADIUS = 14;
  const GLOW_RADIUS = 32;
  const PADDING = 30;

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let ctx: CanvasRenderingContext2D | null = $state(null);

  let layoutPositions: Record<string, { x: number; y: number }> = {};
  let manualPositions: Record<string, { x: number; y: number }> = {};
  let prevW = 0;
  let prevH = 0;
  let animFrame: number | null = null;

  // Drag state
  let dragFixture: string | null = null;
  let dragOffsetX = 0;
  let dragOffsetY = 0;

  let frame: StageFrame | null = null;
  let focusPositions: Record<string, Pt> = {};

  /** The same picture as the dashboard's stage view, read-only: a stage
   *  plot when the venue has geometry, the tag heuristic otherwise. */
  function computeLayout(
    fixtures: Record<string, FixtureMetadata>,
    venue: VenueMetadata | null,
  ) {
    const names = Object.keys(fixtures);
    if (!canvasEl || names.length === 0) return;

    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    const points = venue?.focus_points ?? {};

    if (!hasGeometry(fixtures, points)) {
      frame = null;
      focusPositions = {};
      const auto = tagLayout(fixtures, w, h, PADDING + 20);
      layoutPositions = {};
      for (const name of names) {
        layoutPositions[name] = manualPositions[name] ?? auto[name];
      }
      return;
    }

    const stage: Rect = {
      x: PADDING - 10,
      y: PADDING - 10,
      w: w - 2 * PADDING + 20,
      h: h - 2 * PADDING + 20,
    };
    const anyUnplaced = Object.values(fixtures).some((f) => f.position == null);
    const trayHeight = anyUnplaced ? 2 * FIXTURE_RADIUS + 16 : 0;
    const plot: Rect = {
      x: stage.x + FIXTURE_RADIUS + 4,
      y: stage.y + 6,
      w: stage.w - 2 * (FIXTURE_RADIUS + 4),
      h: stage.h - 6 - 12 - trayHeight,
    };
    frame = fitFrame(fixtures, points, plot);
    const laid = positionalLayout(fixtures, frame);
    layoutPositions = {
      ...laid.placed,
      ...trayLayout(
        laid.unplaced,
        {
          x: stage.x,
          y: stage.y + stage.h - trayHeight,
          w: stage.w,
          h: trayHeight,
        },
        FIXTURE_RADIUS,
      ),
    };
    focusPositions = {};
    for (const [name, point] of Object.entries(points)) {
      focusPositions[name] = toPx(frame, point);
    }
  }

  function resizeCanvas() {
    if (!canvasEl) return;
    const dpr = window.devicePixelRatio || 1;
    const newW = canvasEl.clientWidth;
    const newH = canvasEl.clientHeight;
    canvasEl.width = newW * dpr;
    canvasEl.height = newH * dpr;
    const c = canvasEl.getContext("2d");
    if (c) {
      c.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx = c;
    }

    if (prevW > 0 && prevH > 0) {
      const sx = newW / prevW;
      const sy = newH / prevH;
      for (const name in manualPositions) {
        manualPositions[name].x *= sx;
        manualPositions[name].y *= sy;
      }
    }
    prevW = newW;
    prevH = newH;

    computeLayout($metadataStore, $venueStore);
  }

  function draw(fixtureStates: Record<string, FixtureChannels>) {
    if (!canvasEl || !ctx) return;

    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    ctx.clearRect(0, 0, w, h);

    // Theme-aware stage chrome. The stage stays darker than the page
    // even in light mode so colored fixture glows still pop, but it's
    // no longer a hardcoded near-black slab on a paper background.
    const isDark = document.documentElement.classList.contains("nc--dark");
    const stageFill = isDark ? "#1a1a1a" : "#efeeee"; // gray-100
    const stageStroke = isDark ? "#3a3a3e" : "#c9c7c8"; // gray-300
    const fixtureStroke = isDark ? "#555" : "#9a9a9a"; // gray-400
    const labelFill = isDark ? "#888" : "#4a4849"; // gray-600

    // Stage outline
    ctx.fillStyle = stageFill;
    ctx.fillRect(
      PADDING - 10,
      PADDING - 10,
      w - 2 * PADDING + 20,
      h - 2 * PADDING + 20,
    );
    ctx.strokeStyle = stageStroke;
    ctx.lineWidth = 1;
    ctx.strokeRect(
      PADDING - 10,
      PADDING - 10,
      w - 2 * PADDING + 20,
      h - 2 * PADDING + 20,
    );

    for (const name of Object.keys($metadataStore)) {
      const pos = layoutPositions[name];
      if (!pos) continue;

      const state = fixtureStates[name] || {};
      const r = state.red || 0;
      const g = state.green || 0;
      const b = state.blue || 0;
      const dimmer = state.dimmer !== undefined ? state.dimmer : 255;
      const strobe = state.strobe || 0;

      const brightness = dimmer / 255;
      const fr = Math.round(r * brightness);
      const fg = Math.round(g * brightness);
      const fb = Math.round(b * brightness);

      let strobeVisible = true;
      if (strobe > 10) {
        const freq = 2 + (strobe / 255) * 18;
        const phase = (Date.now() / 1000) * freq;
        strobeVisible = Math.sin(phase * Math.PI * 2) > 0;
      }

      const intensity = (fr + fg + fb) / (3 * 255);
      const finalR = strobeVisible ? fr : 0;
      const finalG = strobeVisible ? fg : 0;
      const finalB = strobeVisible ? fb : 0;

      // Glow
      if (intensity > 0.02 && strobeVisible) {
        const gradient = ctx.createRadialGradient(
          pos.x,
          pos.y,
          FIXTURE_RADIUS,
          pos.x,
          pos.y,
          GLOW_RADIUS,
        );
        gradient.addColorStop(
          0,
          `rgba(${finalR},${finalG},${finalB},${intensity * 0.5})`,
        );
        gradient.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = gradient;
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, GLOW_RADIUS, 0, Math.PI * 2);
        ctx.fill();
      }

      // Fixture body — dashed when the venue has not placed it yet.
      ctx.fillStyle = `rgb(${finalR},${finalG},${finalB})`;
      ctx.strokeStyle = fixtureStroke;
      ctx.lineWidth = 1;
      if (frame && $metadataStore[name]?.position == null) {
        ctx.setLineDash([3, 2]);
      }
      ctx.beginPath();
      ctx.arc(pos.x, pos.y, FIXTURE_RADIUS, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      ctx.setLineDash([]);

      // Label
      ctx.fillStyle = labelFill;
      ctx.font = "9px monospace";
      ctx.textAlign = "center";
      ctx.fillText(name, pos.x, pos.y + FIXTURE_RADIUS + 10);
    }

    // Beams of placed movers, ending at their footprint on the deck.
    if (frame) {
      for (const [name, pose] of Object.entries($poseStore)) {
        const meta = $metadataStore[name];
        const from = layoutPositions[name];
        if (!meta?.position || !from) continue;
        const end = toPx(frame, beamEnd(meta.position, pose.aim, pose.floor));
        drawBeam(
          ctx,
          from,
          end,
          fixtureStates[name] || {},
          pose.floor !== null,
          {
            dark: isDark,
            width: 2,
            dot: 5,
          },
        );
      }
    }

    // Focus points: the pins the show can aim at.
    const focusFill = isDark ? "#d9a441" : "#b8801f";
    for (const [name, pos] of Object.entries(focusPositions)) {
      ctx.fillStyle = focusFill;
      ctx.beginPath();
      ctx.moveTo(pos.x, pos.y - 6);
      ctx.lineTo(pos.x + 6, pos.y);
      ctx.lineTo(pos.x, pos.y + 6);
      ctx.lineTo(pos.x - 6, pos.y);
      ctx.closePath();
      ctx.fill();
      ctx.font = "bold 8px monospace";
      ctx.textAlign = "center";
      ctx.fillText(name, pos.x, pos.y - 9);
    }
  }

  function animLoop() {
    draw($fixtureStore);
    animFrame = requestAnimationFrame(animLoop);
  }

  // Hit-test
  function fixtureAt(cx: number, cy: number): string | null {
    for (const name of Object.keys(layoutPositions)) {
      const pos = layoutPositions[name];
      const dx = cx - pos.x;
      const dy = cy - pos.y;
      if (dx * dx + dy * dy <= FIXTURE_RADIUS * FIXTURE_RADIUS) return name;
    }
    return null;
  }

  function canvasCoords(e: MouseEvent): { x: number; y: number } {
    const rect = canvasEl!.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  function onMouseDown(e: MouseEvent) {
    if (frame) return; // read-only plot; edit on the dashboard
    const pt = canvasCoords(e);
    const name = fixtureAt(pt.x, pt.y);
    if (name) {
      dragFixture = name;
      dragOffsetX = pt.x - layoutPositions[name].x;
      dragOffsetY = pt.y - layoutPositions[name].y;
      canvasEl!.style.cursor = "grabbing";
      e.preventDefault();
    }
  }

  function onMouseMove(e: MouseEvent) {
    const pt = canvasCoords(e);
    if (dragFixture) {
      const newX = pt.x - dragOffsetX;
      const newY = pt.y - dragOffsetY;
      layoutPositions[dragFixture] = { x: newX, y: newY };
      manualPositions[dragFixture] = { x: newX, y: newY };
    } else {
      canvasEl!.style.cursor = fixtureAt(pt.x, pt.y) ? "grab" : "default";
    }
  }

  function onMouseUp() {
    if (dragFixture) {
      dragFixture = null;
      canvasEl!.style.cursor = "grab";
    }
  }

  function onMouseLeave() {
    if (dragFixture) {
      dragFixture = null;
      canvasEl!.style.cursor = "default";
    }
  }

  function touchCoords(e: TouchEvent): { x: number; y: number } {
    const rect = canvasEl!.getBoundingClientRect();
    const touch = e.touches[0] || e.changedTouches[0];
    return { x: touch.clientX - rect.left, y: touch.clientY - rect.top };
  }

  function onTouchStart(e: TouchEvent) {
    if (e.touches.length !== 1 || frame) return;
    const pt = touchCoords(e);
    const name = fixtureAt(pt.x, pt.y);
    if (name) {
      dragFixture = name;
      dragOffsetX = pt.x - layoutPositions[name].x;
      dragOffsetY = pt.y - layoutPositions[name].y;
      e.preventDefault();
    }
  }

  function onTouchMove(e: TouchEvent) {
    if (!dragFixture || e.touches.length !== 1) return;
    const pt = touchCoords(e);
    const newX = pt.x - dragOffsetX;
    const newY = pt.y - dragOffsetY;
    layoutPositions[dragFixture] = { x: newX, y: newY };
    manualPositions[dragFixture] = { x: newX, y: newY };
    e.preventDefault();
  }

  function onTouchEnd() {
    dragFixture = null;
  }

  $effect(() => {
    if (canvasEl) {
      resizeCanvas();
      animLoop();
      window.addEventListener("resize", resizeCanvas);
    }
    return () => {
      window.removeEventListener("resize", resizeCanvas);
      if (animFrame !== null) cancelAnimationFrame(animFrame);
    };
  });

  $effect(() => {
    computeLayout($metadataStore, $venueStore);
  });

  let hasFixtures = $derived(Object.keys($metadataStore).length > 0);
  let activeEffects = $derived($effectsStore);
</script>

<div class="stage-preview" class:empty={!hasFixtures}>
  {#if hasFixtures}
    <canvas
      bind:this={canvasEl}
      onmousedown={onMouseDown}
      onmousemove={onMouseMove}
      onmouseup={onMouseUp}
      onmouseleave={onMouseLeave}
      ontouchstart={onTouchStart}
      ontouchmove={onTouchMove}
      ontouchend={onTouchEnd}
    ></canvas>
    {#if activeEffects.length > 0}
      <div class="effects-list">
        {#each activeEffects as fx (fx)}
          <span class="effect-tag">{fx}</span>
        {/each}
      </div>
    {/if}
  {:else}
    <div class="no-fixtures">{$t("timeline.noFixtures")}</div>
  {/if}
</div>

<style>
  .stage-preview {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--inset-bg);
    border-radius: var(--nc-radius-sm);
    overflow: hidden;
    position: relative;
  }
  .stage-preview.empty {
    align-items: center;
    justify-content: center;
  }
  canvas {
    display: block;
    width: 100%;
    flex: 1;
    min-height: 0;
  }
  .no-fixtures {
    color: var(--text-dim);
    font-size: 13px;
  }
  .effects-list {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    padding: 4px 6px;
    background: rgba(0, 0, 0, 0.3);
    max-height: 72px;
    overflow-y: auto;
  }
  .effect-tag {
    font-size: 10px;
    font-family: var(--mono);
    color: var(--text-muted);
    background: var(--bg-hover);
    padding: 1px 5px;
    border-radius: 3px;
    white-space: nowrap;
  }
</style>
