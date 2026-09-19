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
  import { fetchVenue, saveVenue } from "../lib/api/config";
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
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

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

  // --- Editing feedback
  let saveMsg = $state<{ ok: boolean; text: string } | null>(null);
  let saving = $state(false);
  let renaming = $state<Record<string, string>>({});

  let venue = $derived($venueStore);
  let focusPoints = $derived(venue?.focus_points ?? {});
  let geometryMode = $derived(hasGeometry($metadataStore, focusPoints));
  let placedCount = $derived(
    Object.values($metadataStore).filter((f) => f.position != null).length,
  );
  let focusNames = $derived(Object.keys(focusPoints).sort());

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

    for (const name of Object.keys($metadataStore)) {
      const pos = layoutPositions[name];
      if (!pos) continue;
      const meta = $metadataStore[name];
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
            width: 3,
            dot: 9,
          },
        );
      }
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

  function animLoop() {
    draw($fixtureStore);
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
          fromTray: frame !== null && $metadataStore[name]?.position == null,
        };
      }
    }
    return null;
  }

  function canvasCoords(e: MouseEvent): Pt {
    const rect = canvasEl!.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }

  function beginDrag(pt: Pt): boolean {
    // In geometry mode a drag ends in a save; one at a time, so nothing
    // is silently lost while the previous save is in flight.
    if (frame && saving) return false;
    const target = hit(pt.x, pt.y);
    if (!target) return false;
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
    const activeFrame = frame;
    if (finished.kind === "focus") {
      if (!inPlot(focusPositions[finished.name])) {
        // Dropped off the stage: put it back where the file says.
        computeLayout($metadataStore, $venueStore);
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
      computeLayout($metadataStore, $venueStore);
      return;
    }
    const [x, y] = toStage(activeFrame, dropped);
    const z = $metadataStore[finished.name]?.position?.[2] ?? DEFAULT_TRIM_M;
    await persist((v) => {
      const fixture = v.fixtures[finished.name];
      if (fixture) fixture.position = [x, y, z];
    });
  }

  // --- Persistence: the venue file is the truth. Read it, change the one
  // thing, write it back; the server reloads the running venue and pushes
  // fresh metadata, which redraws everything from the file's numbers.
  // Last write wins: two editors on the same venue (two tabs, or the web
  // UI racing an MCP patch) can overwrite each other's latest change. A
  // single operator designing a show is the case this serves.
  async function persist(
    update: (venue: {
      fixtures: Record<
        string,
        {
          name: string;
          fixture_type: string;
          universe: number;
          start_channel: number;
          tags: string[];
          position?: Vec3 | null;
          rotation?: Vec3 | null;
        }
      >;
      focus_points?: Record<string, Vec3>;
      source?: { mvr: string; origin: Vec3 } | null;
    }) => void,
  ) {
    const meta = $venueStore;
    if (!meta) return;
    if (saving) {
      // Never silently: the caller's edit did not happen.
      saveMsg = {
        ok: false,
        text: get(t)("stage.saveFailed", {
          values: { error: get(t)("stage.busy") },
        }),
      };
      computeLayout($metadataStore, $venueStore);
      return;
    }
    saving = true;
    saveMsg = null;
    try {
      const { venue: current } = await fetchVenue(
        meta.name,
        meta.dir ?? undefined,
      );
      update(current);
      await saveVenue(
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
      );
      // Optimistic: the broadcast metadata will confirm, but a client the
      // engine cannot reach (no DMX engine running) should still see it.
      const fixtures = { ...$metadataStore };
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
      venueStore.set({ ...meta, focus_points: current.focus_points ?? {} });
      saveMsg = { ok: true, text: get(t)("stage.saved") };
      setTimeout(() => (saveMsg = null), 2000);
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      saveMsg = {
        ok: false,
        text: get(t)("stage.saveFailed", { values: { error: message } }),
      };
      computeLayout($metadataStore, $venueStore);
    } finally {
      saving = false;
    }
  }

  async function addFocusPoint() {
    const name = nextFocusName(focusPoints);
    // Center of the shown stage, on the deck; a venue with no geometry yet
    // gets its first pin at downstage-center.
    const point: Vec3 = frame
      ? [
          Math.round(((frame.minX + frame.maxX) / 2) * 100) / 100,
          Math.round(((frame.minY + frame.maxY) / 2) * 100) / 100,
          0,
        ]
      : [0, 1, 0];
    await persist((v) => {
      v.focus_points = { ...(v.focus_points ?? {}), [name]: point };
    });
  }

  async function renameFocusPoint(from: string) {
    const to = (renaming[from] ?? "").trim();
    if (saving) {
      // Keep the draft; the input shows it until the save settles.
      return;
    }
    delete renaming[from];
    renaming = { ...renaming };
    if (!to || to === from || to in focusPoints) return;
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
    if (beginDrag(canvasCoords(e))) {
      canvasEl!.style.cursor = "grabbing";
      e.preventDefault();
    }
  }

  function onMouseMove(e: MouseEvent) {
    const pt = canvasCoords(e);
    if (drag) {
      moveDrag(pt);
    } else {
      canvasEl!.style.cursor = hit(pt.x, pt.y) ? "grab" : "default";
    }
  }

  function onMouseUp() {
    if (drag) {
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
    if (drag) {
      // Abandoned mid-drag: put things back where the file says.
      drag = null;
      canvasEl!.style.cursor = "default";
      if (frame) computeLayout($metadataStore, $venueStore);
    }
  }

  // Lifecycle
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

  // Recompute layout when metadata or the venue changes
  $effect(() => {
    computeLayout($metadataStore, $venueStore);
  });
</script>

<section class="card stage-card" class:stage-card--geometry={geometryMode}>
  <header class="stage-card__head">
    <div>
      <div class="overline">{$t("stage.title")}</div>
      <div class="stage-card__title">
        {$t("stage.title")} · {Object.keys($metadataStore).length} fixtures
        {#if geometryMode}
          <span class="stage-card__placed">
            · {$t("stage.placed", {
              values: {
                placed: placedCount,
                total: Object.keys($metadataStore).length,
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
      {#if $reloadStore}
        <span
          class="badge stage-card__reload"
          class:stage-card__reload--error={$reloadStore.status === "error"}
        >
          {$reloadStore.status === "ok"
            ? $t("stage.reloaded")
            : `Error: ${$reloadStore.error}`}
        </span>
      {/if}
      <a href="#/stage" class="btn btn-sm stage-card__3d">
        {$t("stage3d.open")}
      </a>
      {#if venue}
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
  <div class="stage-card__viewport">
    <div class="stage-card__caption" aria-hidden="true">
      {$t("stage.label")}
    </div>
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
  </div>
  {#if venue && (geometryMode || focusNames.length > 0)}
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
  .stage-card__viewport {
    position: relative;
    flex: 1;
    width: 100%;
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
