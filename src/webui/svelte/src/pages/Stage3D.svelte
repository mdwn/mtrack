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
   * Stage 3D (design §16.3): the venue as a room — deck, fixtures from
   * their rig models, live pan and tilt, beams from live colour and level,
   * focus markers, an orbit camera. A pre-viz sketch a band can read, fed
   * by the same stores as the stage plot.
   */
  import { onMount } from "svelte";
  import { t } from "svelte-i18n";
  import {
    fixtureStore,
    metadataStore,
    poseStore,
    venueStore,
  } from "../lib/ws/stores";
  import type {
    CameraPreset,
    SceneStats,
    SceneryStats,
    StageScene,
  } from "../lib/stage/scene3d";

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let hostEl: HTMLDivElement | undefined = $state();
  let scene: StageScene | null = $state(null);
  let renderer: "loading" | "webgl" | "none" = $state("loading");
  let stats: SceneStats | null = $state(null);
  let scenery: SceneryStats | null = $state(null);
  let preset: CameraPreset = $state("foh");
  /** Fixture labels: on for a small rig, off when they would carpet it. */
  let labels = $state(true);
  let labelsChosen = $state(false);
  /** Frames drawn, sampled every few frames — proof the scene is live. */
  let frames = $state(0);

  const fixtureCount = $derived(Object.keys($metadataStore).length);

  function choosePreset(next: CameraPreset) {
    preset = next;
    scene?.setCamera(next);
  }

  onMount(() => {
    let raf = 0;
    let disposed = false;
    let observer: ResizeObserver | null = null;
    let live: StageScene | null = null;

    (async () => {
      try {
        // three.js arrives with this chunk only; nothing else in the UI
        // loads it.
        const { StageScene } = await import("../lib/stage/scene3d");
        if (disposed || !canvasEl || !hostEl) return;
        live = new StageScene(canvasEl);
        live.onStats = (s) => (stats = s);
        live.onSceneryStats = (s) => (scenery = s);
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
        console.warn("Stage 3D: no WebGL renderer", e);
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

  // The venue and its fixtures rebuild the scene; live state just feeds it.
  $effect(() => {
    const fixtures = $metadataStore;
    const venue = $venueStore;
    if (scene) void scene.setVenue(fixtures, venue);
  });
  $effect(() => {
    const path = $venueStore?.scenery ?? null;
    if (scene) void scene.setScenery(path);
  });
  $effect(() => {
    scene?.setChannels($fixtureStore);
  });
  $effect(() => {
    scene?.setPoses($poseStore);
  });
  $effect(() => {
    if (!labelsChosen) labels = fixtureCount <= 40;
  });
  $effect(() => {
    scene?.setLabels(labels);
  });
</script>

<div class="stage3d page">
  <div class="page__head stage3d__head">
    <div>
      <h1 class="page__title">{$t("stage3d.title")}</h1>
      <p class="page__subtitle stage3d__subtitle">
        {#if $venueStore}
          {$venueStore.name} ·
        {/if}
        {$t("stage3d.fixtures", { values: { count: fixtureCount } })}
        {#if stats}
          · {$t("stage3d.placed", {
            values: { placed: stats.placed, total: stats.fixtures },
          })}
          {#if stats.generic > 0}
            · {$t("stage3d.generic", { values: { count: stats.generic } })}
          {/if}
        {/if}
        {#if $venueStore?.scenery_error}
          · {$t("stage3d.sceneryError")}
        {/if}
        {#if scenery}
          · {$t("stage3d.scenery", { values: { count: scenery.drawn } })}
          {#if scenery.skipped > 0}
            {$t("stage3d.sceneryUndrawn", {
              values: {
                count: scenery.skipped,
                formats: scenery.formats.map((f) => "." + f).join(", "),
              },
            })}
          {/if}
        {/if}
      </p>
    </div>
    <div class="stage3d__actions">
      <div class="stage3d__presets" role="group" aria-label="Camera">
        {#each [["foh", "stage3d.foh"], ["side", "stage3d.side"], ["top", "stage3d.top"]] as [key, label] (key)}
          <button
            class="btn btn-sm"
            class:stage3d__preset--active={preset === key}
            type="button"
            onclick={() => choosePreset(key as CameraPreset)}
          >
            {$t(label)}
          </button>
        {/each}
      </div>
      <button
        class="btn btn-sm"
        class:stage3d__preset--active={labels}
        type="button"
        aria-pressed={labels}
        onclick={() => {
          labelsChosen = true;
          labels = !labels;
        }}
      >
        {$t("stage3d.labels")}
      </button>
      <a href="#/" class="btn btn-sm">{$t("stage3d.back")}</a>
    </div>
  </div>

  <div
    class="stage3d__viewport"
    bind:this={hostEl}
    data-renderer={renderer}
    data-frames={frames}
  >
    <canvas class="stage3d__canvas" bind:this={canvasEl}></canvas>
    {#if renderer === "none"}
      <div class="stage3d__fallback">{$t("stage3d.noWebgl")}</div>
    {:else if fixtureCount === 0}
      <div class="stage3d__fallback">{$t("stage3d.noFixtures")}</div>
    {/if}
  </div>
  <p class="stage3d__hint">{$t("stage3d.hint")}</p>
</div>

<style>
  .stage3d {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: calc(100vh - 120px);
  }
  .stage3d__subtitle {
    margin: 4px 0 0;
  }
  .stage3d__actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .stage3d__presets {
    display: inline-flex;
    gap: 4px;
  }
  .stage3d__preset--active {
    background: var(--accent-subtle);
    border-color: var(--accent);
  }
  .stage3d__viewport {
    position: relative;
    flex: 1;
    min-height: 420px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    background: var(--bg-card);
  }
  .stage3d__canvas {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
  }
  .stage3d__fallback {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted, var(--text));
    pointer-events: none;
    padding: 24px;
    text-align: center;
  }
  .stage3d__hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted, var(--text));
    opacity: 0.8;
  }
</style>
