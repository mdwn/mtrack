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
  import { untrack } from "svelte";
  import { t } from "svelte-i18n";
  import {
    type BeatGrid,
    formatSeconds,
    formatSecondsShort,
    parseSeconds,
    positionAtTime,
    timeAtPosition,
  } from "../../lib/util/beatGrid";
  import UnitToggle from "./UnitToggle.svelte";

  export interface Position {
    measure: number;
    /** 1-based within the measure; halves allowed when `step` is 0.5. */
    beat: number;
  }

  /** What the owner keeps: a musical position, or a time in seconds. */
  export type PositionValue =
    | ({ kind: "beat" } & Position)
    | { kind: "time"; time: number };

  interface Props {
    /** Names every control, e.g. "Start" → "Start: next beat". */
    label: string;
    value: PositionValue;
    /** Beat granularity: whole beats, or halves for section boundaries. */
    step?: 1 | 0.5;
    /** "beat" can only keep a musical position, so time entry snaps to the
     * grid; "either" lets the value come off the grid entirely. */
    stores?: "beat" | "either";
    maxMeasure?: number;
    /** Beats per measure — the meter in effect there. */
    beatsIn?: (measure: number) => number;
    /** Time signature shown on a measure, e.g. "7/4". */
    sigOf?: (measure: number) => string | undefined;
    /** Beat times, for the time readout and for converting between the two.
     * Without it the time mode is unavailable. */
    beatGrid?: BeatGrid | null;
    /** A second, dimmed marker: the preview playhead, in seconds. Drawn at
     * its true position, between beats when that is where it is. */
    ghostTime?: number | null;
    /** Units the readout is driven in. Bindable so sibling pickers in one
     * dialog stay in the same unit rather than drifting apart. */
    unit?: "beat" | "time";
    /** False when the owner renders the toggle itself, for several pickers. */
    showToggle?: boolean;
    onchange: (value: PositionValue) => void;
  }

  let {
    label,
    value,
    step = 1,
    stores = "beat",
    maxMeasure = 9999,
    beatsIn = () => 4,
    sigOf,
    beatGrid = null,
    ghostTime = null,
    unit = $bindable("beat"),
    showToggle = true,
    onchange,
  }: Props = $props();

  /** Measures visible in the ruler at once. */
  const WINDOW = 3;
  /** How close to an end a held drag starts walking the window along. */
  const EDGE_PX = 24;

  /** Seconds the ◀ ▶ keys move by in a decoupled time value. */
  const TIME_STEPS = [0.01, 0.1, 1];

  /** Half-beat mode only: whether ◀ ▶ move by ½ or a whole beat. */
  let halfSteps = $state(true);
  let timeStep = $state(0.1);

  /** A value that keeps a time is always shown in time. */
  let mode = $derived(
    value.kind === "time" ? "time" : beatGrid ? unit : "beat",
  );
  /** Where the value is a time, nothing snaps: the cursor leaves the grid. */
  let decoupled = $derived(value.kind === "time");

  /** The musical position, snapped, whatever the value's kind. */
  let position = $derived.by((): Position => {
    if (value.kind === "beat")
      return { measure: value.measure, beat: value.beat };
    const at = positionAtTime(beatGrid, value.time);
    return at ?? { measure: 1, beat: 1 };
  });
  let measure = $derived(position.measure);
  let beat = $derived(position.beat);
  /** Song time of the value, when the grid can say. */
  let seconds = $derived(
    value.kind === "time"
      ? value.time
      : (timeAtPosition(beatGrid, value.measure, value.beat) ?? null),
  );
  let beatStep = $derived(step === 1 || !halfSteps ? 1 : 0.5);
  let first = $state(1);
  let laneEl = $state<HTMLDivElement>();
  let readoutEl = $state<HTMLDivElement>();
  let typing = $state(false);
  let edgeDir = $state(0);

  let lastMeasure = $derived(Math.max(1, maxMeasure));
  let windowStart = $derived(
    Math.max(1, Math.min(first, Math.max(1, lastMeasure - WINDOW + 1))),
  );
  let windowMeasures = $derived.by(() => {
    const out: number[] = [];
    for (let m = windowStart; m < windowStart + WINDOW && m <= lastMeasure; m++)
      out.push(m);
    return out;
  });

  // The window follows the cursor whenever it steps outside.
  $effect(() => {
    const m = measure;
    const f = untrack(() => first);
    if (m < f) first = m;
    else if (m > f + WINDOW - 1) first = m - WINDOW + 1;
  });

  let ghost = $derived(
    ghostTime === null ? null : positionAtTime(beatGrid, ghostTime),
  );

  function fmtBeat(b: number): string {
    if (b % 1 === 0) return `${b}`;
    return b % 1 === 0.5 ? `${Math.floor(b)}½` : b.toFixed(2);
  }
  // An approximate position is stated to the half beat: "b1.20" is precision
  // no one asked for.
  let musical = $derived(
    `m${measure} · b${fmtBeat(decoupled ? Math.round(beat * 2) / 2 : beat)}`,
  );
  let clock = $derived(seconds === null ? "" : formatSeconds(seconds));
  let readout = $derived(mode === "time" ? clock : musical);
  /** The line under the readout: the other unit, and how exact it is. */
  let readoutSub = $derived(
    mode === "time" ? (decoupled ? `≈ ${musical}` : `= ${musical}`) : clock,
  );
  let typedValue = $derived(
    mode === "time"
      ? clock
      : beat % 1
        ? `${measure}.${Math.floor(beat)}.5`
        : `${measure}.${beat}`,
  );
  let spoken = $derived(
    mode === "time" && seconds !== null
      ? $t("position.valueTime", {
          values: { time: formatSeconds(seconds), position: musical },
        })
      : $t("position.value", { values: { measure, beat: fmtBeat(beat) } }),
  );

  /** Whole seconds inside each visible measure, computed once per render:
   * both the labels and the lines read the same list. */
  let secondsByMeasure = $derived.by(() => {
    const out: Record<number, { time: number; pct: number }[]> = {};
    if (mode !== "time") return out;
    for (const m of windowMeasures) out[m] = secondsIn(m);
    return out;
  });

  /** Whole seconds inside a measure, placed where they fall in the music —
   * under a tempo change they bunch up, which is the point. */
  function secondsIn(m: number): { time: number; pct: number }[] {
    const from = timeAtPosition(beatGrid, m, 1);
    const to = timeAtPosition(beatGrid, m + 1, 1);
    if (from === null || to === null || to <= from) return [];
    const out: { time: number; pct: number }[] = [];
    for (let s = Math.ceil(from); s < to; s += 1) {
      const at = positionAtTime(beatGrid, s);
      if (!at || at.measure !== m) continue;
      out.push({ time: s, pct: ((at.beat - 1) / beatsIn(m)) * 100 });
    }
    return out;
  }

  function clampBeat(m: number, b: number): number {
    const highest = beatsIn(m) + 1 - step;
    return Math.max(1, Math.min(highest, Math.round(b / step) * step));
  }

  function emit(m: number, b: number) {
    const mm = Math.max(1, Math.min(lastMeasure, Math.round(m)));
    const bb = clampBeat(mm, b);
    if (decoupled) {
      const t = timeAtPosition(beatGrid, mm, bb);
      if (t !== null) emitTime(t);
      return;
    }
    if (value.kind === "beat" && mm === measure && bb === beat) return;
    onchange({ kind: "beat", measure: mm, beat: bb });
  }

  /** Emits a decoupled time, clamped to the grid. */
  function emitTime(t: number) {
    const last = beatGrid?.beats[beatGrid.beats.length - 1] ?? t;
    const clamped = Math.max(0, Math.min(last, Math.round(t * 1000) / 1000));
    if (value.kind === "time" && clamped === value.time) return;
    onchange({ kind: "time", time: clamped });
  }

  function stepTime(delta: number) {
    if (seconds === null) return;
    emitTime(seconds + delta * timeStep);
  }

  /** Switches units, converting so the value on screen does not move. */
  function setMode(next: "beat" | "time") {
    if (mode === next) return;
    unit = next;
    if (stores === "beat") return;
    if (next === "time") {
      const t = timeAtPosition(beatGrid, measure, beat);
      if (t !== null) onchange({ kind: "time", time: t });
    } else {
      onchange({ kind: "beat", measure, beat: clampBeat(measure, beat) });
    }
  }

  /** Steps `delta` beat-steps, rolling over measure lines. */
  function stepBeats(delta: number) {
    if (decoupled) {
      stepTime(delta);
      return;
    }
    let m = measure;
    let b = beat + delta * beatStep;
    while (b >= beatsIn(m) + 1 && m < lastMeasure) {
      b -= beatsIn(m);
      m += 1;
    }
    while (b < 1 && m > 1) {
      m -= 1;
      b += beatsIn(m);
    }
    emit(m, b);
  }

  function stepMeasures(delta: number) {
    if (decoupled) {
      const t = timeAtPosition(beatGrid, measure + delta, beat);
      if (t !== null) emitTime(t);
      return;
    }
    emit(measure + delta, beat);
  }

  function shiftWindow(delta: number) {
    const next = Math.max(
      1,
      Math.min(Math.max(1, lastMeasure - WINDOW + 1), windowStart + delta),
    );
    if (next === windowStart) return;
    first = next;
    // Drag the cursor along rather than letting it scroll out of sight.
    if (measure < next) emit(next, beat);
    else if (measure > next + WINDOW - 1) emit(next + WINDOW - 1, beat);
  }

  // --- hold-to-repeat -----------------------------------------------------

  function holdable(node: HTMLButtonElement, fn: () => void) {
    let delay: ReturnType<typeof setTimeout> | null = null;
    let repeat: ReturnType<typeof setInterval> | null = null;
    const stop = () => {
      if (delay) clearTimeout(delay);
      if (repeat) clearInterval(repeat);
      delay = repeat = null;
    };
    const start = (e: PointerEvent) => {
      if (node.disabled) return;
      e.preventDefault();
      node.setPointerCapture(e.pointerId);
      fn();
      delay = setTimeout(() => (repeat = setInterval(fn, 90)), 420);
    };
    node.addEventListener("pointerdown", start);
    for (const ev of ["pointerup", "pointercancel", "lostpointercapture"])
      node.addEventListener(ev, stop);
    return {
      destroy: () => (node.removeEventListener("pointerdown", start), stop()),
    };
  }

  // --- ruler: tap, drag, and edge walking ---------------------------------

  function posFromX(clientX: number): Position | null {
    if (!laneEl) return null;
    const cells = laneEl.querySelectorAll<HTMLElement>(".pp-measure");
    for (const cell of cells) {
      const m = Number(cell.dataset.measure);
      const r = cell.getBoundingClientRect();
      if (clientX >= r.left && clientX <= r.right) {
        const n = beatsIn(m);
        const rel = Math.max(0, Math.min(0.9999, (clientX - r.left) / r.width));
        // Snapped values land on a tick; a decoupled one lands where the
        // finger is, between them.
        const raw = 1 + rel * n;
        return {
          measure: m,
          beat: decoupled ? raw : Math.round(raw / step) * step,
        };
      }
    }
    return null;
  }

  let scrubbing = false;
  let edgeTimer: ReturnType<typeof setTimeout> | null = null;
  let edgeDepth = 0;

  function edgeTick() {
    const before = decoupled ? `${seconds}` : `${measure}:${beat}`;
    stepBeats(edgeDir);
    if ((decoupled ? `${seconds}` : `${measure}:${beat}`) === before) {
      stopEdge();
      return;
    }
    edgeTimer = setTimeout(edgeTick, Math.max(70, 210 - edgeDepth * 2.5));
  }

  function startEdge(dir: number, depth: number) {
    edgeDepth = depth;
    if (edgeDir === dir) return;
    stopEdge();
    edgeDir = dir;
    edgeTick();
  }

  function stopEdge() {
    if (edgeTimer) clearTimeout(edgeTimer);
    edgeTimer = null;
    edgeDir = 0;
  }

  function applyPointer(p: Position) {
    if (decoupled) {
      const t = timeAtPosition(beatGrid, p.measure, p.beat);
      if (t !== null) emitTime(t);
    } else {
      emit(p.measure, p.beat);
    }
  }

  function laneDown(e: PointerEvent) {
    const p = posFromX(e.clientX);
    if (!p) return;
    // Swallow the browser's own drag/select gesture before it starts.
    e.preventDefault();
    scrubbing = true;
    laneEl?.setPointerCapture(e.pointerId);
    applyPointer(p);
  }

  function laneMove(e: PointerEvent) {
    if (!scrubbing || !laneEl) return;
    const r = laneEl.getBoundingClientRect();
    if (e.clientX < r.left + EDGE_PX) {
      startEdge(-1, r.left + EDGE_PX - e.clientX);
    } else if (e.clientX > r.right - EDGE_PX) {
      startEdge(1, e.clientX - (r.right - EDGE_PX));
    } else {
      stopEdge();
      const p = posFromX(e.clientX);
      if (p) applyPointer(p);
    }
  }

  function laneUp() {
    scrubbing = false;
    stopEdge();
  }

  function laneKey(e: KeyboardEvent) {
    const keys: Record<string, () => void> = {
      ArrowRight: () => stepBeats(1),
      ArrowUp: () => stepBeats(1),
      ArrowLeft: () => stepBeats(-1),
      ArrowDown: () => stepBeats(-1),
      PageUp: () => stepMeasures(1),
      PageDown: () => stepMeasures(-1),
      Home: () => emit(measure, 1),
      End: () => emit(measure, beatsIn(measure)),
      // "b"/"t" flip units without leaving the ruler.
      b: () => setMode("beat"),
      t: () => setMode("time"),
    };
    const fn = keys[e.key];
    if (!fn) return;
    e.preventDefault();
    fn();
  }

  $effect(() => () => stopEdge());

  // --- typing a position --------------------------------------------------

  function beginType() {
    typing = true;
    queueMicrotask(() => {
      const input = readoutEl?.querySelector("input");
      input?.focus();
      input?.select();
    });
  }

  function commitTyped(raw: string) {
    typing = false;
    const text = raw.trim();
    // A colon is unambiguously a time; without one, the mode decides.
    if (text.includes(":") || mode === "time") {
      const t = parseSeconds(text);
      if (t !== null) {
        if (decoupled) emitTime(t);
        else {
          const at = positionAtTime(beatGrid, t);
          if (at) emit(at.measure, at.beat);
        }
        return;
      }
      if (text.includes(":")) return;
    }
    const parts = text
      .split(/[.,\s]+/)
      .map(Number)
      .filter((n) => !isNaN(n));
    if (!parts.length) return;
    emit(parts[0] || 1, (parts[1] || 1) + (parts[2] ? 0.5 : 0));
  }
</script>

<div class="position-picker">
  {#if beatGrid && showToggle}
    <UnitToggle unit={mode} onchange={setMode} />
  {/if}
  <div class="pp-transport">
    <button
      type="button"
      class="pp-btn"
      aria-label={$t("position.prevMeasure", { values: { label } })}
      use:holdable={() => stepMeasures(-1)}>&#9668;&#9668;</button
    >
    <button
      type="button"
      class="pp-btn"
      aria-label={$t("position.prevBeat", { values: { label } })}
      use:holdable={() => stepBeats(-1)}>&#9668;</button
    >
    <div
      class="pp-readout"
      bind:this={readoutEl}
      role="button"
      tabindex="0"
      aria-label={$t("position.type", { values: { label } })}
      onclick={() => !typing && beginType()}
      onkeydown={(e) => {
        if (!typing && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          beginType();
        }
      }}
    >
      {#if typing}
        <input
          type="text"
          inputmode="decimal"
          value={typedValue}
          aria-label={$t("position.type", { values: { label } })}
          onblur={(e) => commitTyped((e.target as HTMLInputElement).value)}
          onkeydown={(e) => {
            if (e.key !== "Enter" && e.key !== "Escape") return;
            // Without this the same keypress bubbles to the wrapper, which
            // sees typing already false again and reopens the field.
            e.preventDefault();
            e.stopPropagation();
            if (e.key === "Enter")
              commitTyped((e.target as HTMLInputElement).value);
            else typing = false;
          }}
        />
      {:else}
        <span class="pp-value">{readout}</span>
        {#if readoutSub}<span class="pp-sub">{readoutSub}</span>{/if}
      {/if}
    </div>
    <button
      type="button"
      class="pp-btn"
      aria-label={$t("position.nextBeat", { values: { label } })}
      use:holdable={() => stepBeats(1)}>&#9658;</button
    >
    <button
      type="button"
      class="pp-btn"
      aria-label={$t("position.nextMeasure", { values: { label } })}
      use:holdable={() => stepMeasures(1)}>&#9658;&#9658;</button
    >
  </div>

  <div class="pp-strip">
    <button
      type="button"
      class="pp-nav"
      class:pp-nav--active={edgeDir < 0}
      aria-label={$t("position.earlier", { values: { label } })}
      use:holdable={() => shiftWindow(-1)}>&#9668;</button
    >
    <div
      class="pp-lane"
      bind:this={laneEl}
      role="slider"
      tabindex="0"
      aria-label={$t("position.ruler", { values: { label } })}
      aria-valuemin={1}
      aria-valuemax={lastMeasure}
      aria-valuenow={measure}
      aria-valuetext={spoken}
      onpointerdown={laneDown}
      onpointermove={laneMove}
      onpointerup={laneUp}
      onpointercancel={laneUp}
      onkeydown={laneKey}
    >
      {#each windowMeasures as m (m)}
        {@const beats = beatsIn(m)}
        {@const sig = sigOf?.(m)}
        <div class="pp-measure" data-measure={m} style:flex={beats}>
          <span class="pp-mlabel">m{m}</span>
          {#if sig}
            <span class="pp-sig" class:pp-sig--changed={sig !== sigOf?.(m - 1)}
              >{sig}</span
            >
          {/if}
          <!-- Labels ride above the ticks: on a phone the tick row is under
               your fingertip. -->
          <div class="pp-nums">
            {#if mode === "time"}
              {#each secondsByMeasure[m] ?? [] as s (s.time)}
                <span class="pp-secnum" style:left="{s.pct}%"
                  >{formatSecondsShort(s.time)}</span
                >
              {/each}
              <!-- The value itself, above the fingertip, as in beat mode. -->
              {#if measure === m && seconds !== null}
                <span
                  class="pp-num pp-num--selected"
                  style:left="{((beat - 1) / beats) * 100}%"
                  >{formatSeconds(seconds)}</span
                >
              {/if}
            {:else}
              {#each Array.from({ length: beats }, (_, i) => i + 1) as b (b)}
                {@const selected = measure === m && Math.floor(beat) === b}
                <span
                  class="pp-num"
                  class:pp-num--selected={selected}
                  style:left="{((b - 1) / beats) * 100}%"
                  >{selected ? fmtBeat(beat) : b}</span
                >
              {/each}
            {/if}
          </div>
          <div class="pp-ticks">
            {#each Array.from({ length: step === 1 ? beats : beats * 2 - 1 }, (_, i) => 1 + i * step) as b (b)}
              <span
                class="pp-tick"
                class:pp-tick--downbeat={b === 1}
                class:pp-tick--half={b % 1 !== 0}
                style:left="{((b - 1) / beats) * 100}%"
              ></span>
            {/each}
            {#if mode === "time"}
              {#each secondsByMeasure[m] ?? [] as s (s.time)}
                <span class="pp-secline" style:left="{s.pct}%"></span>
              {/each}
            {/if}
            {#if ghost && ghost.measure === m}
              <span
                class="pp-ghost"
                style:left="{((ghost.beat - 1) / beats) * 100}%"
              ></span>
            {/if}
            {#if measure === m}
              <span class="pp-cursor" style:left="{((beat - 1) / beats) * 100}%"
              ></span>
            {/if}
          </div>
        </div>
      {/each}
    </div>
    <button
      type="button"
      class="pp-nav"
      class:pp-nav--active={edgeDir > 0}
      aria-label={$t("position.later", { values: { label } })}
      use:holdable={() => shiftWindow(1)}>&#9658;</button
    >
  </div>

  {#if decoupled}
    <div class="pp-chips">
      <span class="pp-chips-label">{$t("position.timeStep")}</span>
      {#each TIME_STEPS as s (s)}
        <button
          type="button"
          class="pp-chip"
          class:pp-chip--on={timeStep === s}
          onclick={() => (timeStep = s)}>{s}s</button
        >
      {/each}
    </div>
  {:else if step !== 1}
    <div class="pp-chips">
      <span class="pp-chips-label">{$t("position.beatStep")}</span>
      <button
        type="button"
        class="pp-chip"
        class:pp-chip--on={!halfSteps}
        onclick={() => (halfSteps = false)}>1</button
      >
      <button
        type="button"
        class="pp-chip"
        class:pp-chip--on={halfSteps}
        onclick={() => (halfSteps = true)}>&frac12;</button
      >
    </div>
  {/if}
</div>

<style>
  .position-picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* The ruler is the touch surface, so it sits below everything readable. */
  .pp-transport {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .pp-btn {
    flex: 0 0 auto;
    min-width: 44px;
    min-height: 44px;
    font-size: 14px;
    color: var(--text);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    cursor: pointer;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }
  .pp-btn:active {
    background: var(--bg-raised);
  }
  .pp-readout {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    align-items: center;
    justify-content: center;
    min-height: 44px;
    padding: 0 8px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-variant-numeric: tabular-nums;
    font-size: 15px;
    cursor: text;
  }
  .pp-value {
    line-height: 1.1;
  }
  .pp-sub {
    font-size: 10px;
    color: var(--text-dim);
  }
  .pp-readout input {
    width: 100%;
    text-align: center;
    font: inherit;
    color: inherit;
    background: none;
    border: none;
    outline: none;
  }

  .pp-strip {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .pp-nav {
    flex: 0 0 auto;
    /* A thumb is about 9mm; 28px was a mouse-sized target. */
    width: 36px;
    padding: 0;
    font-size: 11px;
    color: var(--text-dim);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 8px;
    cursor: pointer;
    touch-action: none;
  }
  .pp-nav--active {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-subtle);
  }

  .pp-lane {
    flex: 1;
    display: flex;
    gap: 3px;
    /* Horizontal drags scrub; vertical ones still scroll the sheet. */
    touch-action: pan-y;
    cursor: pointer;
    border-radius: 8px;
    /* A drag across the ruler is a scrub, never a text selection. */
    user-select: none;
    -webkit-user-select: none;
  }
  .pp-measure {
    position: relative;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 15px 7px 7px;
    background: var(--bg-input);
    border-radius: 6px;
  }
  .pp-mlabel,
  .pp-sig {
    position: absolute;
    top: 3px;
    font-size: 9.5px;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
  .pp-mlabel {
    left: 7px;
  }
  .pp-sig {
    right: 7px;
  }
  .pp-sig--changed {
    color: var(--accent);
  }
  .pp-nums {
    position: relative;
    height: 12px;
  }
  .pp-num {
    position: absolute;
    top: 0;
    transform: translateX(-50%);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .pp-num--selected {
    top: -1px;
    font-size: 10.5px;
    font-weight: 600;
    color: var(--accent);
    /* Sits over the second labels in time mode. */
    background: var(--bg-input);
    padding: 0 3px;
    border-radius: 3px;
    white-space: nowrap;
    z-index: 1;
  }
  .pp-secnum {
    position: absolute;
    top: 0;
    transform: translateX(-50%);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .pp-secline {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    margin-left: -0.5px;
    background: var(--text-dim);
    opacity: 0.3;
  }
  .pp-ticks {
    position: relative;
    height: 30px;
  }
  .pp-tick {
    position: absolute;
    top: 7px;
    width: 2px;
    height: 17px;
    margin-left: -1px;
    background: var(--text-dim);
    opacity: 0.55;
    border-radius: 1px;
  }
  .pp-tick--downbeat {
    top: 3px;
    height: 21px;
    opacity: 0.8;
  }
  .pp-tick--half {
    top: 13px;
    height: 8px;
    opacity: 0.3;
  }
  .pp-ghost {
    position: absolute;
    top: 5px;
    width: 2px;
    height: 20px;
    margin-left: -1px;
    background: var(--text-muted);
    border-radius: 1px;
  }
  .pp-cursor {
    position: absolute;
    top: 1px;
    width: 4px;
    height: 25px;
    margin-left: -2px;
    background: var(--accent);
    border-radius: 2px;
    box-shadow: 0 0 0 3px var(--accent-subtle);
  }
  .pp-cursor::after {
    content: "";
    position: absolute;
    left: 50%;
    top: -5px;
    transform: translateX(-50%);
    border-left: 4px solid transparent;
    border-right: 4px solid transparent;
    border-top: 5px solid var(--accent);
  }
  @media (prefers-reduced-motion: no-preference) {
    .pp-cursor {
      transition: left 90ms linear;
    }
  }

  .pp-chips {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .pp-chips-label {
    font-size: 11px;
    color: var(--text-dim);
  }
  .pp-chip {
    min-height: 28px;
    padding: 2px 12px;
    font-size: 12px;
    color: var(--text);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 99px;
    cursor: pointer;
  }
  .pp-chip--on {
    color: var(--bg);
    background: var(--accent);
    border-color: var(--accent);
  }
</style>
