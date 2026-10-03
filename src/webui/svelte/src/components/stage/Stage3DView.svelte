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
<script module lang="ts">
  import type { SceneStats, SceneryStats } from "../../lib/stage/scene3d";

  /** What the view is showing, for its host's header. */
  export interface Stage3DInfo {
    /** Drawn from the venue's file (not the current venue). */
    fromFile: boolean;
    /** The named venue does not exist. */
    missing: boolean;
    /** The venue shown, when known. */
    name: string | null;
    fixtures: number;
    unplaced: number;
    stats: SceneStats | null;
    scenery: SceneryStats | null;
    sceneryError: string | null;
    renderer: "loading" | "webgl" | "none";
  }
</script>

<script lang="ts">
  /**
   * The Stage 3D scene in a box (design §16.3), as the venue card's 3D
   * view draws it: the scene set up, fed and freed in one place, filling
   * whatever box its host gives it. The card loads this component (and
   * three.js with the scene module) the first time 3D is pressed.
   *
   * The source: `venue` null (or the engine's current venue) is live — the
   * stores the engine fills, or `previewFrame` when the host previews a
   * show. Any other venue is read from its file through `/scene` and drawn
   * at rest in a neutral white; it is re-read whenever that venue or any
   * fixture type is saved. The camera is framed once per venue and then
   * left where the user puts it.
   */
  import { onMount, untrack } from "svelte";
  import { t } from "svelte-i18n";
  import {
    cellStore,
    fixtureStore,
    metadataStore,
    poseStore,
    underTestStore,
    venueStore,
  } from "../../lib/ws/stores";
  import type {
    FixtureChannels,
    FixtureMetadata,
    VenueMetadata,
  } from "../../lib/ws/stores";
  import {
    fetchVenueScene,
    VenueNotFoundError,
    type VenueScene,
  } from "../../lib/api/config";
  import {
    concerns,
    fixtureTypeChanges,
    venueChanges,
  } from "../../lib/lighting/changes";
  import { lightingHref } from "../../lib/lightingRoute";
  import {
    wheelFixtureCount,
    type PreviewFrame,
  } from "../../lib/lighting/preview";
  import type { StageScene } from "../../lib/stage/scene3d";

  interface Props {
    /** The venue to show; null for the current venue. */
    venue?: string | null;
    /** Directory overrides, for reading a venue that is not the current one. */
    fixtureTypesDir?: string;
    venuesDir?: string;
    /** A previewed moment to draw instead of the live state (live venue
     *  only); null draws the engine's state. */
    previewFrame?: PreviewFrame | null;
    /** Fixture labels; left unset, on for a small rig and off for a big one. */
    labels?: boolean;
    /** Selected fixtures, marked in the scene. */
    selection?: string[];
    /** Makes fixtures clickable: a click (not a drag, which orbits) on a
     *  fixture selects it, shift-click toggles it, empty space clears. */
    onSelect?: (names: string[]) => void;
    /** A line in the view's corner saying what the mouse does. */
    hint?: string;
    /** Told what the view is showing, for the host's header. */
    oninfo?: (info: Stage3DInfo) => void;
  }

  let {
    venue = null,
    fixtureTypesDir = "",
    venuesDir = "",
    previewFrame = null,
    labels = undefined,
    selection = [],
    onSelect,
    hint = "",
    oninfo,
  }: Props = $props();

  /** The address names a venue that is not the engine's: draw its file. */
  const fromFile = $derived(!!venue && venue !== $venueStore?.name);
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
  // type is re-read at once (the current venue follows the engine).
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
  const unplaced = $derived(
    Object.values(shownMeta).filter((m) => !m.position).length,
  );
  const shownName = $derived(fromFile ? venue : ($venueStore?.name ?? null));
  const fixtureCount = $derived(Object.keys(shownMeta).length);
  const previewing = $derived(!fromFile && previewFrame !== null);

  /** What the scene draws: the engine's state, or the previewed moment. */
  const shown = $derived(
    fromFile
      ? { fixtures: fileLight, poses: {}, cells: {} }
      : previewFrame
        ? previewFrame
        : { fixtures: $fixtureStore, poses: $poseStore, cells: $cellStore },
  );
  const wheels = $derived(wheelFixtureCount(shownMeta));
  const noState = $derived(
    !fromFile &&
      !previewing &&
      Object.keys($fixtureStore).length === 0 &&
      Object.keys($poseStore).length === 0,
  );

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let hostEl: HTMLDivElement | undefined = $state();
  let scene: StageScene | null = $state(null);
  let renderer: "loading" | "webgl" | "none" = $state("loading");
  let stats = $state<Stage3DInfo["stats"]>(null);
  let scenery = $state<Stage3DInfo["scenery"]>(null);
  /** Frames drawn, sampled every few frames — proof the scene is live. */
  let frames = $state(0);
  /** Beams lighting the deck in the last sampled frame (`data-deck-lights`). */
  let deckLights = $state(0);
  /** Each fixture's placement as drawn, for tests (`data-transforms`). */
  let transforms = $state("{}");
  /** The camera and each fixture's place on the canvas (`data-view`). */
  let probe = $state("{}");

  $effect(() => {
    oninfo?.({
      fromFile,
      missing: fromFile && fileMissing,
      name: shownName,
      fixtures: fixtureCount,
      unplaced,
      stats,
      scenery,
      sceneryError: shownVenue?.scenery_error ?? null,
      renderer,
    });
  });

  onMount(() => {
    let raf = 0;
    let disposed = false;
    let observer: ResizeObserver | null = null;
    let live: StageScene | null = null;

    (async () => {
      try {
        // three.js arrives with this chunk only; nothing else in the UI
        // loads it.
        const { StageScene } = await import("../../lib/stage/scene3d");
        if (disposed || !canvasEl || !hostEl) return;
        live = new StageScene(canvasEl);
        live.onStats = (s) => {
          stats = s;
          transforms = JSON.stringify(live?.fixtureTransforms() ?? {});
        };
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
          if (live.framesRendered % 15 === 0) {
            frames = live.framesRendered;
            deckLights = live.deckLightCount;
            probe = JSON.stringify(live.viewProbe());
          }
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
  // Only a different venue reframes the camera.
  let framedFor: string | null | undefined = undefined;
  $effect(() => {
    const fixtures = shownMeta;
    const shownV = shownVenue;
    if (!scene) return;
    const key = fromFile ? `file:${venue}` : `live:${shownV?.name ?? ""}`;
    const frame = untrack(() => framedFor !== key);
    framedFor = key;
    void scene.setVenue(fixtures, shownV, frame);
  });
  let sceneryPath: string | null | undefined = undefined;
  $effect(() => {
    const path = shownVenue?.scenery ?? null;
    if (!scene || path === untrack(() => sceneryPath)) return;
    sceneryPath = path;
    void scene.setScenery(path);
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
    scene?.setLabels(labels ?? fixtureCount <= 40);
  });
  $effect(() => {
    scene?.setSelection(selection);
  });
  /** Live only: a file or a previewed moment is not what is on the wire. */
  const underTest = $derived(fromFile || previewing ? [] : $underTestStore);
  $effect(() => {
    scene?.setUnderTest(underTest);
  });

  // A click selects; a drag orbits. Told apart by how far the pointer went.
  let down: { x: number; y: number } | null = null;
  function onPointerDown(e: PointerEvent) {
    down = { x: e.clientX, y: e.clientY };
  }
  function onPointerUp(e: PointerEvent) {
    const start = down;
    down = null;
    if (!onSelect || !scene || !canvasEl || !start) return;
    if (Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) return;
    const rect = canvasEl.getBoundingClientRect();
    const x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
    const y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
    const name = scene.pick(x, y);
    if (!name) onSelect([]);
    else if (e.shiftKey)
      onSelect(
        selection.includes(name)
          ? selection.filter((n) => n !== name)
          : [...selection, name],
      );
    else onSelect([name]);
  }
</script>

<div
  class="stage3d__viewport"
  bind:this={hostEl}
  data-renderer={renderer}
  data-frames={frames}
  data-deck-lights={deckLights}
  data-source={fromFile ? "file" : previewing ? "preview" : "live"}
  data-venue={shownName ?? ""}
  data-fixtures={fixtureCount}
  data-placed={fixtureCount - unplaced}
  data-selected={selection.join(",")}
  data-under-test={underTest.join(",")}
  data-transforms={transforms}
  data-view={probe}
  data-fed={previewing ? JSON.stringify(shown) : undefined}
>
  <canvas
    class="stage3d__canvas"
    bind:this={canvasEl}
    onpointerdown={onPointerDown}
    onpointerup={onPointerUp}
  ></canvas>
  {#if wheels > 0 || noState || (unplaced > 0 && shownName)}
    <ul class="stage3d__caveats" data-testid="stage3d-caveats">
      {#if unplaced > 0 && shownName}
        <li
          class="stage3d__pill stage3d__pill--note"
          data-testid="stage3d-unplaced"
        >
          {$t("stage3d.unplaced", { values: { count: unplaced } })}
          <a href={lightingHref("venues", shownName)}
            >{$t("stage3d.placeThem", { values: { count: unplaced } })}</a
          >
        </li>
      {/if}
      {#if wheels > 0}
        <li class="stage3d__pill" data-testid="caveat-wheel">
          {$t("stage3d.caveatWheel", { values: { count: wheels } })}
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
    <div class="stage3d__fallback" data-testid="stage3d-nowebgl">
      {$t("stage3d.noWebgl")}
    </div>
  {:else if fixtureCount === 0}
    <div class="stage3d__fallback">
      {shownName
        ? $t("stage3d.noFixturesIn", { values: { name: shownName } })
        : $t("stage3d.noFixtures")}
    </div>
  {/if}
  <div class="stage3d__corner">
    <span class="stage3d__tag" data-testid="stage3d-grid"
      >{$t("stage3d.grid")}</span
    >
    {#if hint}
      <span class="stage3d__tag" data-testid="stage3d-hint">{hint}</span>
    {/if}
  </div>
</div>

<style>
  .stage3d__viewport {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: inherit;
    overflow: hidden;
    /* The room is dark whatever the UI theme. */
    background: #0b0e13;
  }
  .stage3d__canvas {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
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
  .stage3d__pill--note {
    border-radius: var(--radius, 8px);
    pointer-events: auto;
    line-height: 1.4;
  }
  .stage3d__pill a,
  .stage3d__fallback a {
    font-weight: 600;
    color: var(--accent);
  }
  .stage3d__fallback {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: #c9d1db;
    pointer-events: none;
    padding: 24px;
    text-align: center;
  }
  .stage3d__fallback a {
    pointer-events: auto;
  }
  .stage3d__corner {
    position: absolute;
    left: 10px;
    right: 10px;
    bottom: 10px;
    z-index: 1;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    pointer-events: none;
  }
  .stage3d__tag {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    background: rgba(11, 14, 19, 0.7);
    color: #c9d1db;
    border: 1px solid rgba(201, 209, 219, 0.25);
  }
</style>
