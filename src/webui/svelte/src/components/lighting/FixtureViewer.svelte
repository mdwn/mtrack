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
  /**
   * One fixture in 3D, from its type's rig model: the Stage 3D scene with a
   * one-fixture venue, framed on the body and lit full white so lenses and
   * cells read. Until it draws — or when it cannot (no WebGL, no rig) — the
   * archive's thumbnail stands in, or a note that there is no model.
   */
  import { onMount } from "svelte";
  import { t } from "svelte-i18n";
  import type { StageScene } from "../../lib/stage/scene3d";

  interface Props {
    typeName: string;
    /** The rig model, as a path in the asset store; null when there is none. */
    rig: string | null;
    thumbnail: string | null;
    /** The mode being looked at, named in the corner. The model drawn is
     *  the type's own (its pinned mode's rig) whichever it is. */
    modeLabel?: string;
  }

  let { typeName, rig, thumbnail, modeLabel = "" }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let hostEl: HTMLDivElement | undefined = $state();
  let scene: StageScene | null = $state(null);
  let renderer: "loading" | "webgl" | "none" = $state("loading");
  /** The fixture is built and the camera is on it. */
  let framed = $state(false);
  let frames = $state(0);

  onMount(() => {
    let raf = 0;
    let disposed = false;
    let observer: ResizeObserver | null = null;
    let live: StageScene | null = null;
    // Nothing to draw: the fallback is the answer, and three.js stays unloaded.
    if (!rig) {
      renderer = "none";
      return;
    }

    (async () => {
      try {
        // three.js arrives with this chunk only, as on the venue card's 3D view.
        const { StageScene } = await import("../../lib/stage/scene3d");
        if (disposed || !canvasEl || !hostEl) return;
        live = new StageScene(canvasEl);
        live.setLabels(false);
        scene = live;
        renderer = "webgl";
        const fit = () => {
          if (!hostEl || !live) return;
          const rect = hostEl.getBoundingClientRect();
          live.resize(Math.max(1, rect.width), Math.max(1, rect.height));
        };
        observer = new ResizeObserver(fit);
        observer.observe(hostEl);
        fit();
        const loop = () => {
          if (disposed || !live) return;
          live.render();
          if (live.framesRendered % 15 === 0) frames = live.framesRendered;
          raf = requestAnimationFrame(loop);
        };
        loop();
      } catch (e) {
        console.warn("Fixture viewer: no WebGL renderer", e);
        renderer = "none";
      }
    })();

    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
      observer?.disconnect();
      live?.dispose();
      scene = null;
    };
  });

  // The type is a one-fixture venue: hung well clear of the (hidden) deck,
  // full white so every lens and cell is lit.
  $effect(() => {
    const live = scene;
    const name = typeName;
    const path = rig;
    if (!live || !path) return;
    framed = false;
    void live
      .setVenue(
        { [name]: { tags: [], type: name, position: [0, 0, 2], rig: path } },
        null,
      )
      .then(() => {
        if (scene !== live) return;
        live.setChannels({
          [name]: { red: 255, green: 255, blue: 255, dimmer: 255 },
        });
        framed = live.frameFixtures();
      });
  });
</script>

<div
  class="viewer"
  bind:this={hostEl}
  data-testid="ft-viewer"
  data-renderer={renderer}
  data-framed={framed}
  data-frames={frames}
>
  <canvas
    class="viewer__canvas"
    class:viewer__canvas--hidden={!framed}
    bind:this={canvasEl}
    aria-label={$t("lighting.gdtfDetails.viewer", {
      values: { name: typeName },
    })}
  ></canvas>
  {#if modeLabel}
    <span class="viewer__mode" data-testid="ft-viewer-mode">{modeLabel}</span>
  {/if}
  {#if framed}
    <span class="viewer__hint">{$t("lighting.gdtfDetails.hint")}</span>
  {:else}
    <div class="viewer__fallback">
      {#if thumbnail}
        <img
          class="viewer__thumb"
          src={`/api/lighting/assets/${thumbnail}`}
          alt={$t("lighting.gdtfDetails.thumbnail", {
            values: { name: typeName },
          })}
          data-testid="ft-viewer-thumbnail"
        />
      {:else if renderer === "loading"}
        <span>{$t("common.loading")}</span>
      {:else}
        <span data-testid="ft-viewer-none"
          >{$t("lighting.gdtfDetails.noModel")}</span
        >
      {/if}
    </div>
  {/if}
</div>

<style>
  .viewer {
    position: relative;
    width: 100%;
    aspect-ratio: 16 / 10;
    max-height: 320px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    /* The scene's own background: a pre-viz room is dark in either theme. */
    background: #0b0e13;
  }
  .viewer__canvas {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
    cursor: grab;
  }
  .viewer__canvas--hidden {
    visibility: hidden;
  }
  .viewer__fallback {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 16px;
    text-align: center;
    font-size: 13px;
    color: var(--text-dim);
    background: var(--bg);
  }
  .viewer__thumb {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  /* Words on the dark room: the scene's own light grey, whatever the theme. */
  .viewer__hint,
  .viewer__mode {
    position: absolute;
    z-index: 1;
    font-size: 11px;
    color: #c9d2de;
    pointer-events: none;
  }
  .viewer__hint {
    left: 10px;
    bottom: 8px;
    opacity: 0.75;
  }
  .viewer__mode {
    right: 10px;
    top: 8px;
    font-family: var(--mono);
    max-width: calc(100% - 20px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
