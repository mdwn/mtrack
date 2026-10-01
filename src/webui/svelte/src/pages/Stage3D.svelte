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
   *
   * The venue is part of the address (`#/lighting/stage/<venue>`). The
   * current venue (or none named) is drawn live; any other venue is drawn
   * from its file — fixtures at rest in a neutral white, no live colour,
   * and no Preview, which evaluates against the engine's venue.
   */
  import { onMount, untrack } from "svelte";
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
    fetchVenueScene,
    VenueNotFoundError,
    type VenueScene,
  } from "../lib/api/config";
  import {
    concerns,
    fixtureTypeChanges,
    venueChanges,
  } from "../lib/lighting/changes";
  import { lightingHref } from "../lib/lightingRoute";
  import type {
    FixtureChannels,
    FixtureMetadata,
    VenueMetadata,
  } from "../lib/ws/stores";
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
    /** The venue the address names; null for the current venue. */
    venue?: string | null;
    /** Directory overrides, for reading a venue that is not the current one. */
    fixtureTypesDir?: string;
    venuesDir?: string;
  }

  let {
    heading = "h1",
    venue = null,
    fixtureTypesDir = "",
    venuesDir = "",
  }: Props = $props();

  /** The address names a venue that is not the engine's: draw its file. */
  const fromFile = $derived(!!venue && venue !== $venueStore?.name);
  /** The file view's answer, its failure, or a venue the server lacks. */
  let fileScene = $state<VenueScene | null>(null);
  let fileError = $state("");
  let fileMissing = $state(false);

  async function loadFile(name: string) {
    try {
      const got = await fetchVenueScene(
        name,
        venuesDir || undefined,
        fixtureTypesDir || undefined,
      );
      if (venue !== name) return;
      fileScene = got;
      fileError = "";
      fileMissing = false;
    } catch (e: unknown) {
      if (venue !== name) return;
      fileScene = null;
      fileMissing = e instanceof VenueNotFoundError;
      fileError = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    const name = fromFile ? venue : null;
    void venuesDir;
    void fixtureTypesDir;
    untrack(() => {
      fileScene = null;
      fileError = "";
      fileMissing = false;
      if (name) void loadFile(name);
    });
  });
  // A file view follows its files: a save of the venue or of any fixture
  // type is re-read at once.
  $effect(() => {
    const change = $venueChanges;
    untrack(() => {
      if (fromFile && venue && concerns(change, venue)) void loadFile(venue);
    });
  });
  let typesSeen = untrack(() => $fixtureTypeChanges);
  $effect(() => {
    const seq = $fixtureTypeChanges;
    untrack(() => {
      if (seq === typesSeen) return;
      typesSeen = seq;
      if (fromFile && venue) void loadFile(venue);
    });
  });

  /** The fixtures and venue the scene is built from. */
  const shownMeta = $derived<Record<string, FixtureMetadata>>(
    fromFile ? (fileScene?.fixtures ?? {}) : $metadataStore,
  );
  const shownVenue = $derived<VenueMetadata | null>(
    fromFile
      ? fileScene
        ? { ...fileScene.venue, dir: null }
        : null
      : $venueStore,
  );
  /** A file view's fixtures are lit a neutral white, so lenses and beam
   *  directions read without pretending to be a show. */
  const fileLight = $derived<Record<string, FixtureChannels>>(
    Object.fromEntries(
      Object.keys(fromFile ? shownMeta : {}).map((name) => [
        name,
        { red: 255, green: 255, blue: 255, dimmer: 150 },
      ]),
    ),
  );
  /** Fixtures the venue does not place: the scene rows them on a tray. */
  const unplaced = $derived(
    Object.values(shownMeta).filter((m) => !m.position).length,
  );
  /** The venue the page names (and whose plot "place them" links to). */
  const shownName = $derived(fromFile ? venue : ($venueStore?.name ?? null));

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

  const fixtureCount = $derived(Object.keys(shownMeta).length);

  /** What the scene draws: the engine's state, or the previewed moment. */
  const shown = $derived(
    fromFile
      ? { fixtures: fileLight, poses: {}, cells: {} }
      : mode === "preview"
        ? (feed ?? { fixtures: {}, poses: {}, cells: {} })
        : { fixtures: $fixtureStore, poses: $poseStore, cells: $cellStore },
  );
  // Caveats: shown only when they apply.
  const wheels = $derived(wheelFixtureCount(shownMeta));
  const misses = $derived(missingBeamCount(shown.poses));
  const noState = $derived(
    !fromFile &&
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
    const fixtures = shownMeta;
    const shownV = shownVenue;
    if (scene) void scene.setVenue(fixtures, shownV);
  });
  $effect(() => {
    const path = shownVenue?.scenery ?? null;
    if (scene) void scene.setScenery(path);
  });
  // Preview evaluates against the engine's venue: a file view has none.
  $effect(() => {
    if (fromFile && mode === "preview") mode = "live";
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
          data-mode={fromFile ? "file" : mode}
          >{fromFile
            ? $t("stage3d.modeFile")
            : mode === "preview"
              ? $t("stage3d.modePreview")
              : $t("stage3d.modeLive")}</span
        >
      </div>
      <p class="page__subtitle stage3d__subtitle">
        {#if shownName}
          <span data-testid="stage3d-venue"
            >{fromFile
              ? $t("stage.venueFileNotLive", { values: { name: shownName } })
              : $t("stage.currentVenueLive", {
                  values: { name: shownName },
                })}</span
          > ·
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
        {#if shownVenue?.scenery_error}
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
            disabled={fromFile && key === "preview"}
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

  {#if fromFile && !fileMissing}
    <p class="stage3d__note" data-testid="stage3d-no-preview">
      {$t("stage3d.noPreviewFile")}
      <a href="#/lighting/groups">{$t("stage3d.makeCurrent")}</a>
    </p>
  {/if}
  {#if unplaced > 0 && shownName}
    <p class="stage3d__note" data-testid="stage3d-unplaced">
      {$t("stage3d.unplaced", { values: { count: unplaced } })}
      <a href={lightingHref("venues", shownName)}
        >{$t("stage3d.placeThem", { values: { count: unplaced } })}</a
      >
    </p>
  {/if}

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
    data-source={fromFile ? "file" : mode}
    data-venue={shownName ?? ""}
    data-fixtures={fixtureCount}
    data-placed={fixtureCount - unplaced}
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
    {#if fromFile && fileMissing}
      <div class="stage3d__fallback" data-testid="stage3d-missing">
        <p>
          {$t("stage3d.noSuchVenue", { values: { name: venue } })}
          <a href="#/lighting/venues">{$t("stage3d.toVenues")}</a>
        </p>
      </div>
    {:else if fromFile && fileError}
      <div class="stage3d__fallback" data-testid="stage3d-file-error">
        {fileError}
      </div>
    {:else if fromFile && !fileScene}
      <div class="stage3d__fallback">{$t("common.loading")}</div>
    {:else if renderer === "none"}
      <div class="stage3d__fallback">{$t("stage3d.noWebgl")}</div>
    {:else if fixtureCount === 0}
      <div class="stage3d__fallback">
        {shownName
          ? $t("stage3d.noFixturesIn", { values: { name: shownName } })
          : $t("stage3d.noFixtures")}
      </div>
    {/if}
    <span class="stage3d__grid" data-testid="stage3d-grid"
      >{$t("stage3d.grid")}</span
    >
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
  .stage3d__note {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted, var(--text));
  }
  .stage3d__note a,
  .stage3d__fallback a {
    font-weight: 600;
    color: var(--accent);
  }
  .stage3d__grid {
    position: absolute;
    left: 10px;
    bottom: 10px;
    z-index: 1;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    background: var(--bg-card);
    color: var(--text-muted, var(--text));
    border: 1px solid var(--border);
    opacity: 0.85;
    pointer-events: none;
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
  .stage3d__fallback a {
    pointer-events: auto;
  }
  .stage3d__hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted, var(--text));
    opacity: 0.8;
  }
</style>
