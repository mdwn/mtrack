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
  } from "../../../lib/ws/stores";
  import type {
    FixtureChannels,
    FixtureMetadata,
  } from "../../../lib/ws/stores";

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

  function computeLayout(fixtures: Record<string, FixtureMetadata>) {
    const names = Object.keys(fixtures);
    if (!canvasEl || names.length === 0) return;

    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;

    const groups: Record<string, string[]> = {
      left: [],
      right: [],
      front: [],
      back: [],
      mid: [],
      other: [],
    };

    for (const name of names) {
      const tags = fixtures[name].tags || [];
      let placed = false;
      for (const tag of tags) {
        const key = tag.toLowerCase();
        if (key in groups && key !== "other") {
          groups[key].push(name);
          placed = true;
          break;
        }
      }
      if (!placed) groups.other.push(name);
    }

    groups.front = groups.front.concat(groups.other);

    const inset = PADDING + 20;
    const regions: Record<
      string,
      { x: number; y: number; dx: number; dy: number }
    > = {
      left: { x: inset, y: h * 0.25, dx: 0, dy: h * 0.5 },
      right: { x: w - inset, y: h * 0.25, dx: 0, dy: h * 0.5 },
      back: { x: w * 0.25, y: inset, dx: w * 0.5, dy: 0 },
      front: { x: w * 0.25, y: h - inset, dx: w * 0.5, dy: 0 },
      mid: { x: w * 0.35, y: h * 0.4, dx: w * 0.3, dy: h * 0.2 },
    };

    for (const [groupName, region] of Object.entries(regions)) {
      const group = groups[groupName];
      if (!group || group.length === 0) continue;
      const count = group.length;
      for (let i = 0; i < count; i++) {
        const name = group[i];
        if (manualPositions[name]) {
          layoutPositions[name] = manualPositions[name];
        } else {
          const t = count === 1 ? 0.5 : i / (count - 1);
          layoutPositions[name] = {
            x: region.x + region.dx * t,
            y: region.y + region.dy * t,
          };
        }
      }
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

    computeLayout($metadataStore);
  }

  function draw(fixtureStates: Record<string, FixtureChannels>) {
    if (!canvasEl || !ctx) return;

    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    ctx.clearRect(0, 0, w, h);

    // Stage outline
    ctx.fillStyle = "#1a1a1a";
    ctx.fillRect(
      PADDING - 10,
      PADDING - 10,
      w - 2 * PADDING + 20,
      h - 2 * PADDING + 20,
    );
    ctx.strokeStyle = "#3a3a3e";
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

      // Fixture body
      ctx.fillStyle = `rgb(${finalR},${finalG},${finalB})`;
      ctx.strokeStyle = "#555";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(pos.x, pos.y, FIXTURE_RADIUS, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();

      // Label
      ctx.fillStyle = "#666";
      ctx.font = "9px monospace";
      ctx.textAlign = "center";
      ctx.fillText(name, pos.x, pos.y + FIXTURE_RADIUS + 10);
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
    if (e.touches.length !== 1) return;
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
    computeLayout($metadataStore);
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
    background: #111;
    border-radius: var(--radius);
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
    background: rgba(255, 255, 255, 0.06);
    padding: 1px 5px;
    border-radius: 3px;
    white-space: nowrap;
  }
</style>
