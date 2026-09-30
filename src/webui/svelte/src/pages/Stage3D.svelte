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
   *
   * Two sources feed the one scene (design section 12.1): Live, the
   * stores the engine's state message fills, and Preview, a show
   * evaluated offline at a scrubbed moment and shaped the same way.
   */
  import { onMount } from "svelte";
  import { t } from "svelte-i18n";
  import {
    cellStore,
    fixtureStore,
    metadataStore,
    poseStore,
    venueStore,
  } from "../lib/ws/stores";
  import PreviewPanel from "../components/lighting/PreviewPanel.svelte";
  import {
    missingBeamCount,
    previewParams,
    wheelFixtureCount,
    type PreviewFrame,
  } from "../lib/lighting/preview";
  import { SKY_BEAM_LENGTH } from "../lib/stage/rig";
  import type {
    CameraPreset,
    SceneStats,
    SceneryStats,
    StageScene,
  } from "../lib/stage/scene3d";

  interface Props {
    /** Heading level of the page title; Lighting mounts this under its own h1. */
    heading?: "h1" | "h2";
  }

  let { heading = "h1" }: Props = $props();

  const start = previewParams(window.location.hash);
  /** Which source feeds the scene. */
  let mode: "live" | "preview" = $state(start.preview ? "preview" : "live");
  /** What Preview last evaluated; null before the first answer. */
  let feed: PreviewFrame | null = $state(null);

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

  /** What the scene draws: the engine's state, or the previewed moment. */
  const shown = $derived(
    mode === "preview"
      ? (feed ?? { fixtures: {}, poses: {}, cells: {} })
      : { fixtures: $fixtureStore, poses: $poseStore, cells: $cellStore },
  );
  // Caveats: shown only when they apply.
  const wheels = $derived(wheelFixtureCount($metadataStore));
  const misses = $derived(missingBeamCount(shown.poses));
  const noState = $derived(
    mode === "live" &&
      Object.keys($fixtureStore).length === 0 &&
      Object.keys($poseStore).length === 0,
  );

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
    scene?.setChannels(shown.fixtures);
  });
  $effect(() => {
    scene?.setPoses(shown.poses);
  });
  $effect(() => {
    scene?.setCells(shown.cells);
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
      <div class="stage3d__titleline">
        <svelte:element this={heading} class="page__title"
          >{$t("stage3d.title")}</svelte:element
        >
        <span
          class="badge stage3d__mode"
          class:stage3d__mode--preview={mode === "preview"}
          data-testid="stage3d-mode"
          data-mode={mode}
          >{mode === "preview"
            ? $t("stage3d.modePreview")
            : $t("stage3d.modeLive")}</span
        >
      </div>
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
      <div
        class="stage3d__presets"
        role="group"
        aria-label={$t("stage3d.source")}
      >
        {#each [["live", "stage3d.modeLive"], ["preview", "stage3d.modePreview"]] as [key, label] (key)}
          <button
            class="btn btn-sm"
            class:stage3d__preset--active={mode === key}
            type="button"
            aria-pressed={mode === key}
            data-testid="stage3d-mode-{key}"
            onclick={() => (mode = key as "live" | "preview")}
          >
            {$t(label)}
          </button>
        {/each}
      </div>
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

  {#if mode === "preview"}
    <PreviewPanel
      initialSong={start.song}
      initialTime={start.time}
      onfeed={(frame) => (feed = frame)}
    />
  {/if}

  <div
    class="stage3d__viewport"
    bind:this={hostEl}
    data-renderer={renderer}
    data-frames={frames}
    data-source={mode}
    data-fed={mode === "preview" ? JSON.stringify(shown) : undefined}
  >
    <canvas class="stage3d__canvas" bind:this={canvasEl}></canvas>
    {#if wheels > 0 || misses > 0 || noState}
      <ul class="stage3d__caveats" data-testid="stage3d-caveats">
        {#if wheels > 0}
          <li class="stage3d__pill" data-testid="caveat-wheel">
            {$t("stage3d.caveatWheel", { values: { count: wheels } })}
          </li>
        {/if}
        {#if misses > 0}
          <li class="stage3d__pill" data-testid="caveat-beam">
            {$t("stage3d.caveatBeam", {
              values: { count: misses, length: SKY_BEAM_LENGTH },
            })}
          </li>
        {/if}
        {#if noState}
          <li class="stage3d__pill" data-testid="caveat-nostate">
            {$t("stage3d.caveatNoState")}
          </li>
        {/if}
      </ul>
    {/if}
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
  .stage3d__titleline {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .stage3d__mode--preview {
    background: var(--accent-subtle);
    border-color: var(--accent);
  }
  .stage3d__caveats {
    position: absolute;
    left: 10px;
    top: 10px;
    z-index: 1;
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    pointer-events: none;
    max-width: calc(100% - 20px);
  }
  .stage3d__pill {
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    background: var(--bg-card);
    color: var(--text);
    border: 1px solid var(--border);
    opacity: 0.92;
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
