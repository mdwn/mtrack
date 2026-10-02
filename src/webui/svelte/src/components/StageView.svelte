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
  import {
    concerns,
    fixtureTypeChanges,
    venueChanges,
  } from "../lib/lighting/changes";
  import {
    metadataStore,
    fixtureStore,
    cellStore,
    reloadStore,
    venueStore,
    poseStore,
  } from "../lib/ws/stores";
  import type {
    FixtureChannels,
    FixtureMetadata,
    Vec3,
    VenueMetadata,
  } from "../lib/ws/stores";
  import {
    ConflictError,
    fetchFixtureTypes,
    fetchVenue,
    saveVenue,
    type FixtureTypeEntry,
    type VenueData,
    type VenueError,
  } from "../lib/api/config";
  import type { FixturePose } from "../lib/ws/stores";
  import { venueFailure } from "../lib/ws/status";
  import { deckFootprint, restAim } from "../lib/stage/aim";
  import VenueInspector from "./lighting/VenueInspector.svelte";
  import { nudge } from "../lib/stage/arrange";
  import {
    beamEnd,
    cellBar,
    drawBeam,
    fitFrame,
    hasGeometry,
    nextFocusName,
    positionalLayout,
    tagLayout,
    toPx,
    toStage,
    trayLayout,
    type Pt,
    type Rect,
    type StageFrame,
  } from "../lib/stage/layout";
  import { venue3dHref, withParams } from "../lib/lightingRoute";
  // Types only: the 3D view and the preview panel load the first time 3D
  // is pressed, and three.js with them.
  import type { Stage3DInfo } from "./stage/Stage3DView.svelte";
  import type { PreviewFrame } from "../lib/lighting/preview";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { untrack } from "svelte";

  interface Props {
    /** Whether positions and focus points can be edited here. The dashboard
     *  shows the stage as a live view; editing lives on the Venues page. */
    editable?: boolean;
    /** Fixtures to ring (a group's members, on the Fit shows page). */
    highlight?: string[];
    /** Fixtures in a pending selection, ringed apart from the highlight. */
    selected?: string[];
    /** Makes fixtures clickable: called with the clicked fixture's name.
     *  A plot with this set is for choosing, so fixtures do not drag. */
    onFixtureClick?: (name: string) => void;
    /** When set, the next click on the plot creates a focus point with this
     *  name there (Enter on the focused plot puts it at the center). */
    placeFocus?: string | null;
    /** Called once a placement settles, with whether the save succeeded. */
    onFocusPlaced?: (name: string, ok: boolean) => void;
    /** The fixture types directory override, which the Venues inspector
     *  reads to tell movers from fixed fixtures. */
    fixtureTypesDir?: string;
    /** A venue to show from its file instead of the live one: set to a venue
     *  that is not the current one, the plot is a plain view of that file,
     *  edited through the same save, without live fixture colour. The
     *  current venue (or nothing) keeps the live plot. */
    fileVenue?: string | null;
    /** The venues directory override, for reading and saving `fileVenue`. */
    venuesDir?: string;
    /** `?view=3d` (the Venues page only): the card shows the venue in 3D
     *  in place of the plot; the inspector and the header stay. */
    view?: "plot" | "3d";
    /** `?mode=preview&song=&t=`: the 3D view previews a song's show at a
     *  moment (the current venue only). */
    previewMode?: "live" | "preview";
    previewSong?: string | null;
    previewTime?: number | null;
  }

  let {
    editable = false,
    highlight = [],
    selected = [],
    onFixtureClick,
    placeFocus = null,
    onFocusPlaced,
    fixtureTypesDir = "",
    fileVenue = null,
    venuesDir = "",
    view = "plot",
    previewMode = "live",
    previewSong = null,
    previewTime = null,
  }: Props = $props();

  // --- Plot | 3D. The view is the address, so Back, a reload and a shared
  // link keep it; leaving 3D drops a preview's moment with it.
  function setView(next: "plot" | "3d") {
    window.location.hash = withParams(
      window.location.hash,
      next === "3d"
        ? { view: "3d" }
        : { view: null, mode: null, song: null, t: null },
    );
  }
  function setMode(next: "live" | "preview") {
    window.location.hash = withParams(
      window.location.hash,
      next === "preview"
        ? { mode: "preview" }
        : { mode: null, song: null, t: null },
    );
  }
  /** The previewed moment is kept in the address without a history entry
   *  per scrub, so a reload or a shared link opens at it. */
  function rememberMoment(song: string, time: number) {
    window.history.replaceState(
      null,
      "",
      withParams(window.location.hash, {
        song,
        t: String(Math.round(time * 10) / 10),
      }),
    );
  }
  /** What the 3D view is showing (its stats line reads it). */
  let info3d = $state<Stage3DInfo | undefined>();
  /** The previewed moment; null until the first evaluation. */
  let feed = $state<PreviewFrame | null>(null);
  const EMPTY_FRAME: PreviewFrame = { fixtures: {}, poses: {}, cells: {} };

  const FIXTURE_RADIUS = 22;
  const GLOW_RADIUS = 50;
  const PADDING = 60;
  // The plot wants the room the tag layout spends on margins: smaller
  // discs, a tighter inset, and label allowances inside the plot.
  const GEO_RADIUS = 15;
  const GEO_GLOW = 34;
  const GEO_INSET = 28;
  const FOCUS_RADIUS = 8;
  const TRAY_HEIGHT = 2 * GEO_RADIUS + 34;
  /** Trim height a fixture dragged onto the plot is hung at, meters. */
  const DEFAULT_TRIM_M = 3;

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let ctx: CanvasRenderingContext2D | null = $state(null);

  // --- Tag-layout mode keeps its per-browser nudges, as before.
  const STORAGE_KEY = "mtrack-stage-positions";

  function loadManualPositions(): Record<string, Pt> {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) return JSON.parse(stored);
    } catch {
      /* ignore */
    }
    return {};
  }

  function saveManualPositions() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(manualPositions));
    } catch {
      /* ignore */
    }
  }

  // --- Layout state
  let layoutPositions: Record<string, Pt> = {};
  let focusPositions: Record<string, Pt> = {};
  let manualPositions: Record<string, Pt> = loadManualPositions();
  let frame: StageFrame | null = null;
  let plotRect: Rect = { x: 0, y: 0, w: 0, h: 0 };
  let trayRect: Rect | null = null;
  let unplaced: string[] = [];
  let prevW = 0;
  let prevH = 0;

  // --- Drag state: a fixture or a focus point.
  type Drag =
    | { kind: "fixture"; name: string; fromTray: boolean }
    | { kind: "focus"; name: string };
  let drag: Drag | null = null;
  let dragOffsetX = 0;
  let dragOffsetY = 0;

  let animFrame: number | null = null;

  // --- Selection (the editing plot): click, shift-click, marquee.
  /** Pixels a press may wander and still be a click. */
  const CLICK_SLOP = 4;
  let selection = $state<string[]>([]);
  let marquee: { x0: number; y0: number; x1: number; y1: number } | null = null;
  let marqueeAdds = false;
  let pressPt: Pt | null = null;
  let dragMoved = false;
  let dragShift = false;

  // --- Editing feedback
  let saveMsg = $state<{ ok: boolean; text: string } | null>(null);
  /** The last save's report that the current venue no longer loads. */
  let venueError = $state<VenueError | null>(null);
  let saving = $state(false);
  let renaming = $state<Record<string, string>>({});

  // --- What the plot shows: the live venue from the websocket, or (when
  // the Venues page picked another one) that venue's file, read through the
  // venues API. A file view has no live colour and no engine poses.
  // A current venue that did not load has no live fixtures to draw, which
  // left nothing to select and fix. The editable plot shows its file then,
  // so the inspector can put it right.
  let failedLive = $derived(editable ? ($venueFailure?.name ?? null) : null);
  let shownFile = $derived(fileVenue ?? failedLive);
  let viewingFile = $derived(
    !!shownFile &&
      (shownFile !== $venueStore?.name || shownFile === failedLive),
  );
  let fileView = $state<{
    name: string;
    venue: VenueData;
    version: string | null;
  } | null>(null);
  let fileError = $state("");
  let fileTypes = $state<Record<string, FixtureTypeEntry>>({});
  /** The version of the file the plot was loaded from; a save made against
   *  another one is refused by the server. */
  let fileVersion: string | null = null;

  async function loadFileView(name: string) {
    fileError = "";
    try {
      const got = await fetchVenue(name, venuesDir || undefined);
      if (shownFile !== name) return;
      fileView = {
        name,
        venue: { ...got.venue, name },
        version: got.version ?? null,
      };
      fileVersion = got.version ?? null;
    } catch (e: unknown) {
      if (shownFile !== name) return;
      fileView = null;
      fileError = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    const name = shownFile;
    const dir = venuesDir;
    void dir;
    if (viewingFile && name) {
      untrack(() => {
        fileView = null;
        void loadFileView(name);
      });
    } else {
      untrack(() => {
        fileView = null;
        fileError = "";
      });
    }
  });

  // A file view follows its file: any save of it (the venue form, a rename
  // of its types, an import) is re-read at once, not when the name changes.
  $effect(() => {
    const change = $venueChanges;
    untrack(() => {
      if (viewingFile && shownFile && concerns(change, shownFile))
        void loadFileView(shownFile);
    });
  });

  $effect(() => {
    if (!viewingFile) return;
    void $fixtureTypeChanges;
    fetchFixtureTypes(fixtureTypesDir || undefined)
      .then((r) => (fileTypes = r.fixtureTypes))
      .catch(() => {
        // Without types every fixture counts as fixed.
      });
  });

  function isMoverType(type: string): boolean {
    const ch = fileTypes[type]?.fixture_type?.channels ?? {};
    return "pan" in ch && "tilt" in ch;
  }

  let fileFixtures = $derived.by<Record<string, FixtureMetadata>>(() => {
    if (!fileView) return {};
    return Object.fromEntries(
      Object.entries(fileView.venue.fixtures).map(([key, f]) => [
        f.name ?? key,
        {
          tags: f.tags ?? [],
          type: f.fixture_type,
          position: f.position ?? null,
          rotation: f.rotation ?? null,
          capabilities: isMoverType(f.fixture_type) ? ["pan_tilt"] : [],
        } satisfies FixtureMetadata,
      ]),
    );
  });
  /** A fixed fixture's beam, from its mounting alone. */
  let filePoses = $derived.by<Record<string, FixturePose>>(() => {
    const out: Record<string, FixturePose> = {};
    for (const [name, f] of Object.entries(fileFixtures)) {
      if (!f.position || !f.rotation) continue;
      if (f.capabilities?.includes("pan_tilt")) continue;
      const aim = restAim(f.rotation);
      out[name] = {
        pan: 0,
        tilt: 0,
        aim,
        floor: deckFootprint(f.position, aim),
      };
    }
    return out;
  });
  let shownFixtures = $derived<Record<string, FixtureMetadata>>(
    viewingFile ? fileFixtures : $metadataStore,
  );
  let shownVenue = $derived<VenueMetadata | null>(
    viewingFile
      ? fileView
        ? {
            name: fileView.name,
            dir: venuesDir || null,
            focus_points: fileView.venue.focus_points ?? {},
          }
        : null
      : $venueStore,
  );
  let is3d = $derived(editable && view === "3d" && !!shownVenue);
  let previewing = $derived(is3d && !viewingFile && previewMode === "preview");
  let shownPoses = $derived<Record<string, FixturePose>>(
    viewingFile ? filePoses : $poseStore,
  );

  let venue = $derived(shownVenue);
  let focusPoints = $derived(venue?.focus_points ?? {});
  let geometryMode = $derived(hasGeometry(shownFixtures, focusPoints));
  let placedCount = $derived(
    Object.values(shownFixtures).filter((f) => f.position != null).length,
  );
  let focusNames = $derived(Object.keys(focusPoints).sort());
  /** Selection, the inspector and nudging are for the editing plot of a
   *  venue with stage geometry. */
  let selectable = $derived(editable && !!venue && geometryMode);

  // A fixture the venue no longer has drops out of the selection.
  $effect(() => {
    const known = shownFixtures;
    const kept = untrack(() => selection).filter((n) => n in known);
    if (kept.length !== untrack(() => selection).length) selection = kept;
  });

  function computeLayout(
    fixtures: Record<string, FixtureMetadata>,
    venueMeta: VenueMetadata | null,
  ) {
    if (!canvasEl) return;
    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    const points = venueMeta?.focus_points ?? {};

    if (!hasGeometry(fixtures, points)) {
      frame = null;
      trayRect = null;
      unplaced = [];
      focusPositions = {};
      const auto = tagLayout(fixtures, w, h, PADDING + 40);
      layoutPositions = {};
      for (const name of Object.keys(auto)) {
        layoutPositions[name] = manualPositions[name] ?? auto[name];
      }
      return;
    }

    // The plot fills the stage rectangle, less a tray along the bottom for
    // fixtures the venue has not placed yet. The fit rectangle is inset by
    // a disc plus a label so nothing is drawn against the edge.
    const stage: Rect = {
      x: GEO_INSET,
      y: GEO_INSET,
      w: w - 2 * GEO_INSET,
      h: h - 2 * GEO_INSET,
    };
    const anyUnplaced = Object.values(fixtures).some((f) => f.position == null);
    trayRect = anyUnplaced
      ? {
          x: stage.x,
          y: stage.y + stage.h - TRAY_HEIGHT,
          w: stage.w,
          h: TRAY_HEIGHT,
        }
      : null;
    const label = GEO_RADIUS + 18;
    plotRect = {
      x: stage.x + label,
      y: stage.y + label,
      w: stage.w - 2 * label,
      h: stage.h - label - (label + 14) - (trayRect ? trayRect.h : 0),
    };
    frame = fitFrame(fixtures, points, plotRect);
    const laid = positionalLayout(fixtures, frame);
    unplaced = laid.unplaced;
    layoutPositions = {
      ...laid.placed,
      ...(trayRect ? trayLayout(unplaced, trayRect, GEO_RADIUS) : {}),
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
    // Setting a canvas's size blanks it, even to the size it already has,
    // and the next animation frame is the first thing to paint on it
    // again. So the bitmap is only resized when it has to be, and then
    // drawn on at once, so no frame shows an empty stage.
    const resized =
      canvasEl.width !== newW * dpr || canvasEl.height !== newH * dpr;
    if (resized) {
      canvasEl.width = newW * dpr;
      canvasEl.height = newH * dpr;
    }
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

    computeLayout(shownFixtures, shownVenue);
    publishLayout();
    if (resized) redraw();
  }

  function drawPlotChrome(
    w: number,
    h: number,
    colors: { grid: string; axis: string; caption: string },
  ) {
    if (!ctx || !frame) return;
    // Meter grid, the centerline a touch stronger, the audience edge
    // labeled so the picture reads the right way up.
    // Grid lines run the width of the stage rectangle, one per meter, so
    // the deck reads as a floor and not a chart.
    const gridTop = GEO_INSET;
    const gridBottom = (trayRect ? trayRect.y : h - GEO_INSET) - 16;
    ctx.save();
    ctx.beginPath();
    ctx.rect(GEO_INSET, gridTop, w - 2 * GEO_INSET, gridBottom - gridTop);
    ctx.clip();
    ctx.lineWidth = 1;
    for (
      let x = Math.ceil(frame.minX) - 2;
      x <= Math.floor(frame.maxX) + 2;
      x++
    ) {
      const px = toPx(frame, [x, 0]).x;
      ctx.strokeStyle = x === 0 ? colors.axis : colors.grid;
      ctx.beginPath();
      ctx.moveTo(px, gridTop);
      ctx.lineTo(px, gridBottom);
      ctx.stroke();
    }
    for (
      let y = Math.ceil(frame.minY) - 2;
      y <= Math.floor(frame.maxY) + 2;
      y++
    ) {
      const py = toPx(frame, [0, y]).y;
      ctx.strokeStyle = y === 0 ? colors.axis : colors.grid;
      ctx.beginPath();
      ctx.moveTo(GEO_INSET, py);
      ctx.lineTo(w - GEO_INSET, py);
      ctx.stroke();
    }
    ctx.restore();

    ctx.fillStyle = colors.caption;
    ctx.font = "bold 10px monospace";
    ctx.textAlign = "center";
    ctx.fillText(get(t)("stage.audience"), w / 2, gridBottom + 11);
    if (trayRect) {
      ctx.strokeStyle = colors.grid;
      ctx.setLineDash([4, 4]);
      ctx.beginPath();
      ctx.moveTo(trayRect.x, trayRect.y);
      ctx.lineTo(trayRect.x + trayRect.w, trayRect.y);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillText(get(t)("stage.unplaced"), w / 2, trayRect.y + 12);
    }
  }

  function draw(fixtureStates: Record<string, FixtureChannels>) {
    // Per-cell values ride the same state message as the channels; read
    // here so a redraw for either shows both.
    const cellStates = get(cellStore);
    if (!canvasEl || !ctx) return;

    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    ctx.clearRect(0, 0, w, h);

    // Theme-aware stage chrome.
    const isDark = document.documentElement.classList.contains("nc--dark");
    const stageFill = isDark ? "#1a1a1a" : "#efeeee"; // gray-100
    const stageStroke = isDark ? "#3a3a3e" : "#c9c7c8"; // gray-300
    const stageCaption = isDark ? "#333" : "#9a9a9a"; // gray-400
    const gridLine = isDark ? "#242426" : "#e2e0e1";
    const axisLine = isDark ? "#3a3a3e" : "#cfcdce";
    const fixtureStroke = isDark ? "#555" : "#9a9a9a"; // gray-400
    const fixtureLabel = isDark ? "#888" : "#4a4849"; // gray-600
    const focusFill = isDark ? "#d9a441" : "#b8801f";
    const ringMember = isDark ? "#5aa9ff" : "#1c65c4";
    const ringSelected = isDark ? "#f0c040" : "#a86400";

    // Stage outline
    const inset = frame ? GEO_INSET : PADDING - 20;
    ctx.fillStyle = stageFill;
    ctx.fillRect(inset, inset, w - 2 * inset, h - 2 * inset);
    ctx.strokeStyle = stageStroke;
    ctx.lineWidth = 1;
    ctx.strokeRect(inset, inset, w - 2 * inset, h - 2 * inset);

    if (!frame) {
      ctx.fillStyle = stageCaption;
      ctx.font = "12px monospace";
      ctx.textAlign = "center";
      ctx.fillText(get(t)("stage.label"), w / 2, PADDING - 6);
    }

    const radius = frame ? GEO_RADIUS : FIXTURE_RADIUS;
    const glow = frame ? GEO_GLOW : GLOW_RADIUS;

    if (frame) {
      drawPlotChrome(w, h, {
        grid: gridLine,
        axis: axisLine,
        caption: stageCaption,
      });
    }

    for (const name of Object.keys(shownFixtures)) {
      const pos = layoutPositions[name];
      if (!pos) continue;
      const meta = shownFixtures[name];
      const isUnplaced = frame !== null && meta.position == null;

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
          radius,
          pos.x,
          pos.y,
          glow,
        );
        gradient.addColorStop(
          0,
          `rgba(${finalR},${finalG},${finalB},${intensity * 0.5})`,
        );
        gradient.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = gradient;
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, glow, 0, Math.PI * 2);
        ctx.fill();
      }

      // Fixture body — dashed when the venue has not placed it yet. A
      // pixel fixture is a disc of wedges, one per cell in the
      // manufacturer's order, each in its own colour when a per-cell
      // effect drives it (design §17.4), else the fixture's.
      ctx.strokeStyle = fixtureStroke;
      ctx.lineWidth = 1.5;
      if (isUnplaced) ctx.setLineDash([4, 3]);
      const cells = meta.cells ?? [];
      const cellState = cellStates[name];
      const cellColor = (cell: { name: string }) => {
        const own = cellState?.[cell.name];
        if (!own) return `rgb(${finalR},${finalG},${finalB})`;
        const k = (own.dimmer ?? dimmer) / 255;
        const cr = strobeVisible ? Math.round((own.red ?? 0) * k) : 0;
        const cg = strobeVisible ? Math.round((own.green ?? 0) * k) : 0;
        const cb = strobeVisible ? Math.round((own.blue ?? 0) * k) : 0;
        return `rgb(${cr},${cg},${cb})`;
      };
      const bar = cells.length > 1 ? cellBar(cells, meta.rotation) : null;
      if (bar) {
        // Cells along a line: a bar of segments in that direction on the
        // plot, seen from the audience.
        const len = radius * 2.6;
        const thick = radius * 0.9;
        const seg = len / cells.length;
        ctx.save();
        ctx.translate(pos.x, pos.y);
        ctx.rotate(bar.angle);
        for (const [i, cell] of bar.ordered.entries()) {
          ctx.fillStyle = cellColor(cell);
          ctx.fillRect(-len / 2 + i * seg, -thick / 2, seg, thick);
        }
        ctx.strokeRect(-len / 2, -thick / 2, len, thick);
        ctx.restore();
      } else if (cells.length > 1) {
        // Cells that do not line up: a disc of wedges, one per cell in
        // the manufacturer's order.
        const step = (Math.PI * 2) / cells.length;
        for (const [i, cell] of cells.entries()) {
          ctx.fillStyle = cellColor(cell);
          ctx.beginPath();
          ctx.moveTo(pos.x, pos.y);
          ctx.arc(
            pos.x,
            pos.y,
            radius,
            -Math.PI / 2 + i * step,
            -Math.PI / 2 + (i + 1) * step,
          );
          ctx.closePath();
          ctx.fill();
        }
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, radius, 0, Math.PI * 2);
        ctx.stroke();
      } else {
        // One cell (or none): the disc shows that cell's colour when a
        // per-cell effect drives it, else the fixture's.
        ctx.fillStyle =
          cells.length === 1
            ? cellColor(cells[0])
            : `rgb(${finalR},${finalG},${finalB})`;
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, radius, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
      }
      ctx.setLineDash([]);

      // Rings for the Fit shows page: solid for a group's members, dashed
      // and wider for the pending selection. (The page also lists both as
      // text, so colour is never the only signal.)
      if (highlight.includes(name)) {
        ctx.strokeStyle = ringMember;
        ctx.lineWidth = 3;
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, radius + 4, 0, Math.PI * 2);
        ctx.stroke();
      }
      if (selected.includes(name) || selection.includes(name)) {
        ctx.strokeStyle = ringSelected;
        ctx.lineWidth = 3;
        ctx.setLineDash([5, 3]);
        ctx.beginPath();
        ctx.arc(pos.x, pos.y, radius + 9, 0, Math.PI * 2);
        ctx.stroke();
        ctx.setLineDash([]);
      }

      // Label
      ctx.fillStyle = fixtureLabel;
      ctx.font = "11px monospace";
      ctx.textAlign = "center";
      ctx.fillText(name, pos.x, pos.y + radius + 14);
    }

    // Beams: where each placed fixture points, in the color it is
    // showing, ending at its footprint on the deck. Statics get a pose
    // too — pan 0, tilt 0 through their mounting — so the beam is also
    // what says which way a fixture is hung.
    if (frame) {
      for (const [name, pose] of Object.entries(shownPoses)) {
        const meta = shownFixtures[name];
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
            width: 3,
            dot: 9,
          },
        );
      }
    }

    if (marquee) {
      ctx.strokeStyle = ringSelected;
      ctx.fillStyle = "rgba(90, 169, 255, 0.12)";
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 3]);
      const x = Math.min(marquee.x0, marquee.x1);
      const y = Math.min(marquee.y0, marquee.y1);
      const w = Math.abs(marquee.x1 - marquee.x0);
      const h = Math.abs(marquee.y1 - marquee.y0);
      ctx.fillRect(x, y, w, h);
      ctx.strokeRect(x, y, w, h);
      ctx.setLineDash([]);
    }

    // Focus points: diamond pins the show can aim at.
    for (const [name, pos] of Object.entries(focusPositions)) {
      ctx.fillStyle = focusFill;
      ctx.strokeStyle = stageFill;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(pos.x, pos.y - FOCUS_RADIUS);
      ctx.lineTo(pos.x + FOCUS_RADIUS, pos.y);
      ctx.lineTo(pos.x, pos.y + FOCUS_RADIUS);
      ctx.lineTo(pos.x - FOCUS_RADIUS, pos.y);
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = focusFill;
      ctx.font = "bold 10px monospace";
      ctx.textAlign = "left";
      ctx.fillText(name, pos.x + FOCUS_RADIUS + 4, pos.y + 4);
    }
  }

  /** Draws the plot as it is now: live channel values, or none for a file. */
  function redraw() {
    draw(viewingFile ? {} : get(fixtureStore));
  }

  function animLoop() {
    redraw();
    animFrame = requestAnimationFrame(animLoop);
  }

  // --- Hit-testing: focus pins first, they sit on top.
  function hit(cx: number, cy: number): Drag | null {
    for (const name of Object.keys(focusPositions)) {
      const pos = focusPositions[name];
      const dx = cx - pos.x;
      const dy = cy - pos.y;
      if (dx * dx + dy * dy <= FOCUS_RADIUS * FOCUS_RADIUS * 1.5) {
        return { kind: "focus", name };
      }
    }
    const radius = frame ? GEO_RADIUS : FIXTURE_RADIUS;
    for (const name of Object.keys(layoutPositions)) {
      const pos = layoutPositions[name];
      const dx = cx - pos.x;
      const dy = cy - pos.y;
      if (dx * dx + dy * dy <= radius * radius) {
        return {
          kind: "fixture",
          name,
          fromTray: frame !== null && shownFixtures[name]?.position == null,
        };
      }
    }
    return null;
  }

  function canvasCoords(e: MouseEvent): Pt {
    const rect = canvasEl!.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  function beginDrag(pt: Pt, shift = false, marqueeOk = false): boolean {
    // A plot for choosing fixtures or placing a pin is not for dragging.
    if (onFixtureClick || placeFocus) return false;
    const picking = selectable && frame !== null;
    const target = hit(pt.x, pt.y);
    if (picking && !target && marqueeOk) {
      // Empty deck: a box that selects what it encloses.
      marquee = { x0: pt.x, y0: pt.y, x1: pt.x, y1: pt.y };
      marqueeAdds = shift;
      pressPt = pt;
      return true;
    }
    // In geometry mode a drag ends in a save; one at a time, so nothing
    // is silently lost while the previous save is in flight.
    if (frame && (saving || !editable)) return false;
    if (!target) return false;
    if (picking && target.kind === "fixture") {
      const name = target.name;
      if (shift) {
        selection = selection.includes(name)
          ? selection.filter((n) => n !== name)
          : [...selection, name];
      } else if (!selection.includes(name)) {
        selection = [name];
      }
    }
    pressPt = pt;
    dragMoved = false;
    dragShift = shift;
    drag = target;
    const pos =
      target.kind === "focus"
        ? focusPositions[target.name]
        : layoutPositions[target.name];
    dragOffsetX = pt.x - pos.x;
    dragOffsetY = pt.y - pos.y;
    return true;
  }

  function moveDrag(pt: Pt) {
    if (!drag) return;
    if (
      pressPt &&
      Math.hypot(pt.x - pressPt.x, pt.y - pressPt.y) > CLICK_SLOP
    ) {
      dragMoved = true;
    }
    const next = { x: pt.x - dragOffsetX, y: pt.y - dragOffsetY };
    if (drag.kind === "focus") {
      focusPositions[drag.name] = next;
    } else {
      layoutPositions[drag.name] = next;
      if (!frame) manualPositions[drag.name] = next;
    }
  }

  function inPlot(pt: Pt): boolean {
    return (
      pt.x >= plotRect.x &&
      pt.x <= plotRect.x + plotRect.w &&
      pt.y >= plotRect.y &&
      pt.y <= plotRect.y + plotRect.h
    );
  }

  async function endDrag() {
    const finished = drag;
    drag = null;
    if (!finished) return;
    if (!frame) {
      saveManualPositions();
      return;
    }
    if (!dragMoved && selectable) {
      // A click, not a drag: nothing moved, so nothing to save. A plain
      // click on a member of a larger selection narrows it to that one.
      if (
        finished.kind === "fixture" &&
        !dragShift &&
        selection.length > 1 &&
        selection.includes(finished.name)
      ) {
        selection = [finished.name];
      }
      computeLayout(shownFixtures, shownVenue);
      return;
    }
    const activeFrame = frame;
    if (finished.kind === "focus") {
      if (!inPlot(focusPositions[finished.name])) {
        // Dropped off the stage: put it back where the file says.
        computeLayout(shownFixtures, shownVenue);
        return;
      }
      const [x, y] = toStage(activeFrame, focusPositions[finished.name]);
      const z = focusPoints[finished.name]?.[2] ?? 0;
      await persist((v) => {
        v.focus_points = {
          ...(v.focus_points ?? {}),
          [finished.name]: [x, y, z],
        };
      });
      return;
    }
    const dropped = layoutPositions[finished.name];
    if (!inPlot(dropped)) {
      // Dropped back in the tray, or off the stage: nothing changed. A
      // placed fixture cannot be un-placed from here; edit the file.
      computeLayout(shownFixtures, shownVenue);
      return;
    }
    const [x, y] = toStage(activeFrame, dropped);
    const z = shownFixtures[finished.name]?.position?.[2] ?? DEFAULT_TRIM_M;
    await persist((v) => {
      const fixture = v.fixtures[finished.name];
      if (fixture) fixture.position = [x, y, z];
    });
  }

  // --- Persistence: the venue file is the truth. Read it, change the one
  // thing, write it back; the server reloads the running venue and pushes
  // fresh metadata, which redraws everything from the file's numbers.
  // The write carries the file's version. A file plot is a snapshot, so its
  // save must be against the version it was loaded at: if the file changed
  // since (another tab, an edit by hand), nothing is written and the plot
  // reloads for the user to reapply. The live plot re-reads just before it
  // writes, so it only guards the gap between that read and the write, and
  // retries once.
  async function persist(update: (venue: VenueData) => void): Promise<boolean> {
    const meta = shownVenue;
    if (!meta) return false;
    if (saving) {
      // Never silently: the caller's edit did not happen.
      saveMsg = {
        ok: false,
        text: get(t)("stage.saveFailed", {
          values: { error: get(t)("stage.busy") },
        }),
      };
      computeLayout(shownFixtures, shownVenue);
      return false;
    }
    saving = true;
    saveMsg = null;
    const file = viewingFile;
    try {
      for (let attempt = 0; ; attempt++) {
        const { venue: current, version: fresh } = await fetchVenue(
          meta.name,
          meta.dir ?? undefined,
        );
        if (file && fileVersion && fresh && fileVersion !== fresh) {
          throw new ConflictError("", fresh);
        }
        update(current);
        try {
          const saved = await saveVenue(
            meta.name,
            {
              // Keyed by name; the entries carry it too, but a lean server
              // (or the e2e mock) may leave it out.
              fixtures: Object.entries(current.fixtures).map(([name, f]) => ({
                ...f,
                name: f.name ?? name,
              })),
              focus_points: current.focus_points ?? {},
              source: current.source ?? null,
            },
            meta.dir ?? undefined,
            (file ? fileVersion : null) ?? fresh ?? undefined,
          );
          venueError = saved.venueError;
          if (file) {
            fileVersion = saved.version;
            fileView = {
              name: meta.name,
              venue: current,
              version: saved.version,
            };
          } else {
            // Optimistic: the broadcast metadata will confirm, but a client
            // the engine cannot reach (no DMX engine running) should still
            // see it.
            const fixtures = { ...shownFixtures };
            for (const [name, f] of Object.entries(current.fixtures)) {
              if (fixtures[name]) {
                fixtures[name] = {
                  ...fixtures[name],
                  position: f.position ?? null,
                  rotation: f.rotation ?? null,
                };
              }
            }
            metadataStore.set(fixtures);
            venueStore.set({
              ...meta,
              focus_points: current.focus_points ?? {},
            });
          }
          break;
        } catch (e: unknown) {
          if (!(e instanceof ConflictError) || file || attempt > 0) throw e;
          // The live plot's edit is a change to one thing: apply it again to
          // the file as it is now.
        }
      }
      if (venueError) {
        // Saved, and the venue no longer loads: said where the save was
        // made, and left up until a save fixes it.
        saveMsg = {
          ok: false,
          text: get(t)("lighting.venueError.saved", {
            values: { ...venueError },
          }),
        };
      } else {
        saveMsg = { ok: true, text: get(t)("stage.saved") };
        setTimeout(() => (saveMsg = null), 2000);
      }
      return true;
    } catch (e: unknown) {
      if (e instanceof ConflictError) {
        // Someone else changed the venue file: show it as it is now and
        // ask for the change to be made again.
        saveMsg = { ok: false, text: get(t)("stage.changedElsewhere") };
        if (file) void loadFileView(meta.name);
      } else {
        const message = e instanceof Error ? e.message : String(e);
        saveMsg = {
          ok: false,
          text: get(t)("stage.saveFailed", { values: { error: message } }),
        };
      }
      computeLayout(shownFixtures, shownVenue);
      return false;
    } finally {
      saving = false;
    }
  }

  /** Center of the shown stage, on the deck; a venue with no geometry yet
   *  gets its first pin at downstage-center. */
  function centerPoint(): Vec3 {
    return frame
      ? [
          Math.round(((frame.minX + frame.maxX) / 2) * 100) / 100,
          Math.round(((frame.minY + frame.maxY) / 2) * 100) / 100,
          0,
        ]
      : [0, 1, 0];
  }

  async function createFocusPoint(name: string, point: Vec3): Promise<boolean> {
    return persist((v) => {
      v.focus_points = { ...(v.focus_points ?? {}), [name]: point };
    });
  }

  async function addFocusPoint() {
    await createFocusPoint(nextFocusName(focusPoints), centerPoint());
  }

  /** A click while `placeFocus` is set: the pin goes where the click landed
   *  (off the plot, nowhere). */
  async function placeFocusAt(pt: Pt | null) {
    const name = placeFocus;
    if (!name) return;
    let point = centerPoint();
    if (pt) {
      if (frame) {
        if (!inPlot(pt)) return;
        const [x, y] = toStage(frame, pt);
        point = [Math.round(x * 100) / 100, Math.round(y * 100) / 100, 0];
      }
    }
    const ok = await createFocusPoint(name, point);
    onFocusPlaced?.(name, ok);
  }

  function onCanvasClick(e: MouseEvent) {
    const pt = canvasCoords(e);
    if (placeFocus) {
      void placeFocusAt(pt);
      return;
    }
    if (!onFixtureClick) return;
    const target = hit(pt.x, pt.y);
    if (target?.kind === "fixture") onFixtureClick(target.name);
  }

  // --- Nudging: arrow keys, only while the plan itself has focus. Each
  // burst of presses is one save, so holding a key is not a save per repeat.
  let pendingNudge: [number, number] = [0, 0];
  let nudgeTimer: ReturnType<typeof setTimeout> | null = null;

  function scheduleNudge(dx: number, dy: number) {
    pendingNudge = [pendingNudge[0] + dx, pendingNudge[1] + dy];
    if (frame) {
      // Move the discs now; the save redraws them from the file.
      for (const name of selection) {
        const at = layoutPositions[name];
        if (at && shownFixtures[name]?.position) {
          layoutPositions[name] = {
            x: at.x + dx * frame.scale,
            y: at.y - dy * frame.scale,
          };
        }
      }
    }
    if (nudgeTimer) clearTimeout(nudgeTimer);
    nudgeTimer = setTimeout(flushNudge, 250);
  }

  async function flushNudge() {
    nudgeTimer = null;
    const [dx, dy] = pendingNudge;
    if (!dx && !dy) return;
    if (saving) {
      // A save is in flight: keep what has piled up and go again shortly.
      nudgeTimer = setTimeout(flushNudge, 150);
      return;
    }
    pendingNudge = [0, 0];
    const names = [...selection];
    await persist((v) => {
      const items = names
        .filter((n) => v.fixtures[n]?.position)
        .map((n) => ({
          name: n,
          position: [...(v.fixtures[n].position as Vec3)] as Vec3,
        }));
      for (const moved of nudge(items, dx, dy)) {
        v.fixtures[moved.name].position = moved.position;
      }
    });
  }

  const ARROWS: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, 1],
    ArrowDown: [0, -1],
  };

  function onCanvasKeydown(e: KeyboardEvent) {
    if (placeFocus && (e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      void placeFocusAt(null);
      return;
    }
    if (!selectable || placeFocus) return;
    const arrow = ARROWS[e.key];
    if (arrow && selection.length > 0) {
      e.preventDefault();
      const step = e.shiftKey ? 1 : 0.1;
      scheduleNudge(arrow[0] * step, arrow[1] * step);
    }
  }

  /** Escape clears the selection wherever focus is. */
  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && selectable && selection.length > 0) {
      selection = [];
    }
  }

  /** Publishes the fixtures' pixel positions on the canvas when it is used
   *  for choosing or selecting, so a test can click a fixture without re-deriving the
   *  layout. */
  function publishLayout() {
    if (canvasEl && (onFixtureClick || editable)) {
      canvasEl.dataset.positions = JSON.stringify(layoutPositions);
    }
  }

  async function renameFocusPoint(from: string) {
    const to = (renaming[from] ?? "").trim();
    if (saving) {
      // Keep the draft; the input shows it until the save settles.
      return;
    }
    delete renaming[from];
    renaming = { ...renaming };
    if (to === from) return;
    // A refused rename says so: the pin keeps its name, and the message
    // says why rather than the typed name quietly reverting.
    if (!to || to in focusPoints) {
      saveMsg = {
        ok: false,
        text: get(t)(
          !to ? "lighting.renameRefused.empty" : "lighting.renameRefused.taken",
          { values: { name: !to ? from : to } },
        ),
      };
      return;
    }
    await persist((v) => {
      const points = { ...(v.focus_points ?? {}) };
      const point = points[from];
      if (!point) return;
      delete points[from];
      points[to] = point;
      v.focus_points = points;
    });
  }

  async function deleteFocusPoint(name: string) {
    await persist((v) => {
      const points = { ...(v.focus_points ?? {}) };
      delete points[name];
      v.focus_points = points;
    });
  }

  function onMouseDown(e: MouseEvent) {
    // The press is handled here, not by the browser, so focus the plan
    // ourselves: arrow keys nudge only while it has focus.
    // Read the press before focusing: focus can scroll the page.
    const pt = canvasCoords(e);
    if (selectable) canvasEl!.focus({ preventScroll: true });
    if (beginDrag(pt, e.shiftKey, true)) {
      canvasEl!.style.cursor = marquee ? "crosshair" : "grabbing";
      e.preventDefault();
    }
  }

  /** Ends a marquee: what it encloses is selected (added to the selection
   *  with shift); a box that never grew is a click on the empty deck. */
  function endMarquee() {
    const box = marquee;
    marquee = null;
    if (!box) return;
    const wandered =
      Math.abs(box.x1 - box.x0) > CLICK_SLOP ||
      Math.abs(box.y1 - box.y0) > CLICK_SLOP;
    if (!wandered) {
      if (!marqueeAdds) selection = [];
      return;
    }
    const [x0, x1] = [Math.min(box.x0, box.x1), Math.max(box.x0, box.x1)];
    const [y0, y1] = [Math.min(box.y0, box.y1), Math.max(box.y0, box.y1)];
    const inside = Object.entries(layoutPositions)
      .filter(([, p]) => p.x >= x0 && p.x <= x1 && p.y >= y0 && p.y <= y1)
      .map(([name]) => name);
    selection = marqueeAdds
      ? [...selection, ...inside.filter((n) => !selection.includes(n))]
      : inside;
  }

  function onMouseMove(e: MouseEvent) {
    const pt = canvasCoords(e);
    if (marquee) {
      marquee.x1 = pt.x;
      marquee.y1 = pt.y;
    } else if (drag) {
      moveDrag(pt);
    } else if (placeFocus) {
      canvasEl!.style.cursor = "crosshair";
    } else if (onFixtureClick) {
      canvasEl!.style.cursor = hit(pt.x, pt.y) ? "pointer" : "default";
    } else {
      canvasEl!.style.cursor =
        hit(pt.x, pt.y) && (editable || !frame) ? "grab" : "default";
    }
  }

  function onMouseUp() {
    if (marquee) {
      canvasEl!.style.cursor = "default";
      endMarquee();
    } else if (drag) {
      canvasEl!.style.cursor = "grab";
      void endDrag();
    }
  }

  function touchCoords(e: TouchEvent): Pt {
    const rect = canvasEl!.getBoundingClientRect();
    const touch = e.touches[0] || e.changedTouches[0];
    return { x: touch.clientX - rect.left, y: touch.clientY - rect.top };
  }

  function onTouchStart(e: TouchEvent) {
    if (e.touches.length !== 1) return;
    if (beginDrag(touchCoords(e))) e.preventDefault();
  }

  function onTouchMove(e: TouchEvent) {
    if (!drag || e.touches.length !== 1) return;
    moveDrag(touchCoords(e));
    e.preventDefault();
  }

  function onTouchEnd() {
    if (drag) void endDrag();
  }

  function onMouseLeave() {
    if (marquee) {
      marquee = null;
      canvasEl!.style.cursor = "default";
    }
    if (drag) {
      // Abandoned mid-drag: put things back where the file says.
      drag = null;
      canvasEl!.style.cursor = "default";
      if (frame) computeLayout(shownFixtures, shownVenue);
    }
  }

  // Lifecycle: the draw loop starts when the canvas appears and stops when
  // it goes. Nothing else restarts it. The first draw happens inside this
  // effect, and it reads the live state, the selection and the venue; read
  // untracked, so a change to any of them does not tear the loop down and
  // set the canvas up again (which blanked the stage for a frame on every
  // state message).
  $effect(() => {
    let observer: ResizeObserver | undefined;
    if (canvasEl) {
      untrack(() => {
        resizeCanvas();
        animLoop();
      });
      window.addEventListener("resize", resizeCanvas);
      // The plot also changes size when the inspector opens beside it.
      observer = new ResizeObserver(() => resizeCanvas());
      observer.observe(canvasEl);
    }
    return () => {
      window.removeEventListener("resize", resizeCanvas);
      observer?.disconnect();
      if (animFrame !== null) cancelAnimationFrame(animFrame);
    };
  });

  // Recompute layout when metadata or the venue changes
  $effect(() => {
    computeLayout(shownFixtures, shownVenue);
    publishLayout();
  });
</script>

<svelte:window onkeydown={onWindowKeydown} />

<section class="card stage-card" class:stage-card--geometry={geometryMode}>
  <header class="stage-card__head">
    <div>
      <div class="overline">{$t("stage.title")}</div>
      <div class="stage-card__title">
        {#if editable && venue && viewingFile}
          <span data-testid="stage-venue-label"
            >{$t("stage.venueFileNotLive", {
              values: { name: venue.name },
            })}</span
          >
          · {Object.keys(shownFixtures).length} fixtures
        {:else if editable && venue}
          <span data-testid="stage-venue-label"
            >{$t("stage.currentVenueLive", {
              values: { name: venue.name },
            })}</span
          >
          · {Object.keys(shownFixtures).length} fixtures
        {:else}
          {$t("stage.title")} · {Object.keys(shownFixtures).length} fixtures
        {/if}
        {#if geometryMode}
          <span class="stage-card__placed">
            · {$t("stage.placed", {
              values: {
                placed: placedCount,
                total: Object.keys(shownFixtures).length,
              },
            })}
          </span>
        {/if}
      </div>
    </div>
    <div class="stage-card__actions">
      {#if saveMsg}
        <span
          class="badge stage-card__reload"
          class:stage-card__reload--error={!saveMsg.ok}
        >
          {saveMsg.text}
        </span>
      {/if}
      {#if $reloadStore && !viewingFile}
        <span
          class="badge stage-card__reload"
          class:stage-card__reload--error={$reloadStore.status === "error"}
        >
          {$reloadStore.status === "ok"
            ? $t("stage.reloaded")
            : `Error: ${$reloadStore.error}`}
        </span>
      {/if}
      {#if editable && venue}
        <div
          class="stage-card__views"
          role="group"
          aria-label={$t("stage.viewLabel")}
        >
          <button
            class="btn btn-sm"
            class:stage-card__view--active={!is3d}
            type="button"
            aria-pressed={!is3d}
            data-testid="stage-view-plot"
            onclick={() => setView("plot")}>{$t("stage.viewPlot")}</button
          >
          <button
            class="btn btn-sm"
            class:stage-card__view--active={is3d}
            type="button"
            aria-pressed={is3d}
            data-testid="stage-view-3d"
            onclick={() => setView("3d")}>{$t("stage.view3d")}</button
          >
        </div>
      {:else if !editable}
        <!-- The dashboard's live plot opens the current venue in 3D on
             the Venues page. -->
        <a
          href={$venueStore
            ? venue3dHref($venueStore.name)
            : "#/lighting/venues"}
          class="btn btn-sm stage-card__3d"
          data-testid="stage-3d-link"
        >
          {$t("stage3d.open")}
        </a>
      {/if}
      {#if !editable}
        <a href="#/lighting/venues" class="btn btn-sm stage-card__edit">
          {$t("stage.editInVenues")}
        </a>
      {/if}
      {#if venue && editable}
        <button
          class="btn btn-sm stage-card__add-focus"
          type="button"
          disabled={saving}
          onclick={addFocusPoint}
        >
          {$t("stage.addFocus")}
        </button>
      {/if}
    </div>
  </header>
  {#if viewingFile && venue}
    <p class="stage-card__no-venue" data-testid="stage-file-hint">
      {$t("stage.venueFileHint")}
    </p>
  {/if}
  {#if editable && !venue && viewingFile}
    <p
      class="stage-card__no-venue"
      class:stage-card__no-venue--error={!!fileError}
      data-testid="stage-file-loading"
    >
      {fileError || $t("common.loading")}
    </p>
  {:else if editable && !venue}
    <p class="stage-card__no-venue" data-testid="stage-no-venue">
      {$t("stage.noCurrentVenue")}
    </p>
  {:else}
    <div
      class="stage-card__work"
      class:stage-card__work--inspect={selectable && !onFixtureClick}
    >
      <div class="stage-card__main">
        <div class="stage-card__viewport" class:stage-card__viewport--3d={is3d}>
          {#if is3d}
            {#await import("./stage/Stage3DView.svelte") then { default: Stage3DView }}
              <Stage3DView
                venue={viewingFile ? shownFile : null}
                {fixtureTypesDir}
                {venuesDir}
                previewFrame={previewing ? (feed ?? EMPTY_FRAME) : null}
                {selection}
                onSelect={(names) => (selection = names)}
                hint={$t("stage.hint3d")}
                oninfo={(i) => (info3d = i)}
              />
            {/await}
          {:else}
            <div class="stage-card__caption" aria-hidden="true">
              {$t("stage.label")}
            </div>
            <canvas
              bind:this={canvasEl}
              onmousedown={onMouseDown}
              onmousemove={onMouseMove}
              onmouseup={onMouseUp}
              onmouseleave={onMouseLeave}
              onclick={onCanvasClick}
              onkeydown={onCanvasKeydown}
              tabindex={placeFocus || selectable ? 0 : undefined}
              aria-label={placeFocus
                ? $t("stage.placeHint", { values: { name: placeFocus } })
                : selectable
                  ? $t("stage.planHint")
                  : undefined}
              ontouchstart={onTouchStart}
              ontouchmove={onTouchMove}
              ontouchend={onTouchEnd}
            ></canvas>
          {/if}
        </div>
        {#if is3d}
          <div class="stage-card__three" data-testid="stage-3d-under">
            {#if info3d && (info3d.stats?.generic || info3d.scenery || info3d.sceneryError)}
              <p class="stage-card__three-stats" data-testid="stage3d-stats">
                {#if info3d.stats && info3d.stats.generic > 0}
                  <span
                    >{$t("stage3d.generic", {
                      values: { count: info3d.stats.generic },
                    })}</span
                  >
                {/if}
                {#if info3d.sceneryError}
                  <span>{$t("stage3d.sceneryError")}</span>
                {/if}
                {#if info3d.scenery}
                  <span
                    >{$t("stage3d.scenery", {
                      values: { count: info3d.scenery.drawn },
                    })}
                    {#if info3d.scenery.skipped > 0}
                      {$t("stage3d.sceneryUndrawn", {
                        values: {
                          count: info3d.scenery.skipped,
                          formats: info3d.scenery.formats
                            .map((f) => "." + f)
                            .join(", "),
                        },
                      })}
                    {/if}</span
                  >
                {/if}
              </p>
            {/if}
            {#if viewingFile}
              <p
                class="stage-card__three-note"
                data-testid="stage3d-no-preview"
              >
                {$t("stage3d.noPreviewFile")}
                <a href="#/lighting/groups">{$t("stage3d.makeCurrent")}</a>
              </p>
            {:else}
              <div
                class="stage-card__views"
                role="group"
                aria-label={$t("stage3d.source")}
              >
                {#each [["live", "stage3d.modeLive"], ["preview", "stage3d.modePreview"]] as [key, label] (key)}
                  <button
                    class="btn btn-sm"
                    class:stage-card__view--active={previewMode === key}
                    type="button"
                    aria-pressed={previewMode === key}
                    data-testid="stage3d-mode-{key}"
                    onclick={() => setMode(key as "live" | "preview")}
                  >
                    {$t(label)}
                  </button>
                {/each}
              </div>
              {#if previewing}
                {#await import("./lighting/PreviewPanel.svelte") then { default: PreviewPanel }}
                  <PreviewPanel
                    initialSong={previewSong}
                    initialTime={previewTime}
                    onfeed={(frame) => (feed = frame)}
                    onmoment={rememberMoment}
                  />
                {/await}
              {/if}
            {/if}
          </div>
        {/if}
      </div>
      {#if selectable && !onFixtureClick}
        <VenueInspector
          {selection}
          onSelect={(names) => (selection = names)}
          {persist}
          {saving}
          {fixtureTypesDir}
          fixtures={shownFixtures}
          venue={shownVenue}
          live={!viewingFile}
        />
      {/if}
    </div>
  {/if}
  {#if editable && venue && (geometryMode || focusNames.length > 0)}
    <div class="stage-card__focus">
      <div class="overline">{$t("stage.focusPoints")}</div>
      {#if focusNames.length === 0}
        <div class="stage-card__focus-empty">{$t("stage.noFocus")}</div>
      {:else}
        <ul class="stage-card__focus-list">
          {#each focusNames as name (name)}
            <li class="stage-card__focus-item">
              <input
                class="stage-card__focus-name"
                type="text"
                aria-label={name}
                value={renaming[name] ?? name}
                disabled={saving}
                oninput={(e) =>
                  (renaming = {
                    ...renaming,
                    [name]: (e.currentTarget as HTMLInputElement).value,
                  })}
                onblur={() => renameFocusPoint(name)}
                onkeydown={(e) => {
                  if (e.key === "Enter")
                    (e.currentTarget as HTMLInputElement).blur();
                }}
              />
              <span class="stage-card__focus-coords">
                ({focusPoints[name][0]}, {focusPoints[name][1]}, {focusPoints[
                  name
                ][2]})
              </span>
              <button
                class="btn btn-danger btn-sm"
                type="button"
                disabled={saving}
                onclick={() => deleteFocusPoint(name)}
              >
                {$t("stage.deleteFocus")}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .stage-card {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 350px;
  }
  .stage-card__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--card-border);
    gap: 12px;
  }
  .stage-card__actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .stage-card__title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 16px;
    margin-top: 4px;
    color: var(--nc-fg-1);
  }
  .stage-card__placed {
    font-weight: 500;
    color: var(--nc-fg-3);
  }
  .stage-card__work {
    display: contents;
  }
  @media (min-width: 1000px) {
    .stage-card__work--inspect {
      display: grid;
      grid-template-columns: minmax(0, 1fr) 340px;
      align-items: start;
    }
    .stage-card__work--inspect :global(.inspector) {
      border-top: none;
      border-left: 1px solid var(--card-border);
    }
  }
  .stage-card__main {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .stage-card__views {
    display: inline-flex;
    gap: 4px;
  }
  .stage-card__view--active {
    background: var(--accent-subtle);
    border-color: var(--accent);
  }
  .stage-card__viewport--3d {
    border-style: solid;
  }
  .stage-card__three {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 0 16px 16px;
    min-width: 0;
  }
  .stage-card__three-stats,
  .stage-card__three-note {
    margin: 0;
    font-size: 13px;
    color: var(--nc-fg-3);
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
  }
  .stage-card__three-note {
    display: block;
  }
  .stage-card__three-note a {
    font-weight: 600;
    color: var(--accent);
  }
  .stage-card__viewport {
    position: relative;
    flex: 1;
    /* The card pads it with a margin, so size it to what is left; a
       max width keeps a wide card from stretching the plot past what fits. */
    width: calc(100% - 32px);
    max-width: 960px;
    align-self: center;
    box-sizing: border-box;
    min-height: 240px;
    height: 35vh;
    max-height: 450px;
    margin: 16px;
    border-radius: var(--nc-radius-md);
    border: 1px dashed var(--card-border);
    background: var(--inset-bg);
    overflow: hidden;
  }
  .stage-card--geometry .stage-card__viewport {
    height: 45vh;
    max-height: 560px;
  }
  .stage-card__no-venue {
    margin: 16px 20px;
    color: var(--nc-fg-3);
    font-size: 14px;
  }
  .stage-card__no-venue--error {
    color: var(--text-danger);
  }
  .stage-card__caption {
    position: absolute;
    top: 14px;
    left: 50%;
    transform: translateX(-50%);
    font-family: var(--nc-font-mono);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.18em;
    color: var(--nc-fg-4);
    pointer-events: none;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .stage-card__reload {
    background: rgba(77, 192, 138, 0.18);
    color: #2a8e5e;
    border-color: rgba(77, 192, 138, 0.4);
  }
  :global(.nc--dark) .stage-card__reload {
    color: #6bd9a4;
  }
  .stage-card__reload--error {
    background: rgba(232, 75, 75, 0.18);
    color: var(--nc-error);
    border-color: rgba(232, 75, 75, 0.4);
  }
  .stage-card__focus {
    padding: 0 20px 16px;
  }
  .stage-card__focus-empty {
    color: var(--nc-fg-3);
    font-size: 13px;
    margin-top: 6px;
  }
  .stage-card__focus-list {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .stage-card__focus-item {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .stage-card__focus-name {
    font-family: var(--nc-font-mono);
    font-size: 13px;
    min-width: 0;
    width: 180px;
  }
  .stage-card__focus-coords {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-fg-3);
    flex: 1;
  }
</style>
