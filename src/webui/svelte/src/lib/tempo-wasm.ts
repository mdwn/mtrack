// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

import { writable } from "svelte/store";

// The player's tempo map and cue clock (src/tempo.rs), and its light show
// parser for laying out the sequences a show plays, compiled to
// WebAssembly (crates/mtrack-tempo-wasm): where bars, beats, tempo changes,
// ramps and offsets fall, computed by the very code the player runs. Times
// here are in seconds, as the player has them.
//
// The module is generated into src/gen by `make gen-wasm` (or `npm run
// gen:wasm`). It is loaded on demand, by `loadTempoMap()`, only by the views
// that need it (the lighting timeline and the song's section editor), so a
// browser without WebAssembly still has the dashboard and the playback
// controls; those views say why they cannot open. Everything here throws
// until it has loaded.

interface TempoExports {
  memory: WebAssembly.Memory;
  input_buffer(len: number): number;
  output_buffer(): number;
  tm_new(len: number): number;
  tm_free(map: number): void;
  tm_start(map: number): number;
  tm_time_at_beats(map: number, beats: number, offset: number): number;
  tm_beats_at(map: number, at: number, offset: number): number;
  tm_beats_at_position(map: number, measure: number, beat: number): number;
  tm_position_at_beats(map: number, beats: number): number;
  tm_measures_at_beats(map: number, beats: number): number;
  tm_beats_at_measures(map: number, measures: number): number;
  tm_signature(map: number, measure: number): number;
  tm_bpm_at(map: number, at: number, offset: number): number;
  tm_ramp_target(map: number, at: number): number;
  tm_beats_to_duration(
    map: number,
    beats: number,
    at: number,
    offset: number,
  ): number;
  tm_measures_to_duration(
    map: number,
    measures: number,
    at: number,
    offset: number,
  ): number;
  tm_offset_duration(map: number, measures: number, anchor: number): number;
  tm_boundaries(map: number): number;
  ramp_duration(oldBpm: number, newBpm: number, beats: number): number;
  clock_new(): number;
  clock_free(clock: number): void;
  clock_place(
    clock: number,
    map: number,
    kind: number,
    a: number,
    b: number,
    offsetMeasures: number,
    offsets: number,
    reset: number,
  ): number;
  clock_reached(clock: number, at: number): void;
  text_buffer(len: number): number;
  text_output(): number;
  layout_sequences(len: number, map: number): number;
}

let loaded: TempoExports | undefined;
let loading: Promise<void> | undefined;
let compiled: WebAssembly.Module | undefined;
/** Which instance is live: a map or clock made in another is made again. */
let generation = 0;

/** Starts an instance of the compiled module, every export guarded: a trap
 *  (which leaves the instance in no state to trust) is logged, answered
 *  with NaN, and the module started afresh, so later calls still work. */
function start(module: WebAssembly.Module): void {
  const exports = new WebAssembly.Instance(module).exports;
  const guarded: Record<string, unknown> = {};
  for (const [name, value] of Object.entries(exports)) {
    if (typeof value !== "function") {
      guarded[name] = value;
      continue;
    }
    const f = value as (...args: number[]) => number;
    guarded[name] = (...args: number[]) => {
      try {
        return f(...args);
      } catch (e) {
        if (!(e instanceof WebAssembly.RuntimeError)) throw e;
        console.error(`the tempo map trapped in ${name}; starting it again`, e);
        start(module);
        return NaN;
      }
    };
  }
  loaded = guarded as unknown as TempoExports;
  generation++;
}

/** Loads the module, once: resolves when it is ready, rejects (and may be
 *  asked again) when it cannot be. */
export function loadTempoMap(): Promise<void> {
  loading ??= (async () => {
    if (typeof WebAssembly === "undefined") {
      throw new Error("this browser does not run WebAssembly");
    }
    // Its own chunk: the base64 module is downloaded only when needed.
    const { TEMPO_WASM } = await import("../gen/tempo-wasm");
    const bytes = Uint8Array.from(atob(TEMPO_WASM), (c) => c.charCodeAt(0));
    compiled = await WebAssembly.compile(bytes);
    start(compiled);
  })().catch((e: unknown) => {
    loading = undefined;
    throw e instanceof Error ? e : new Error(String(e));
  });
  return loading;
}

/** Whether the module has loaded. */
export function tempoMapLoaded(): boolean {
  return loaded !== undefined;
}

function api(): TempoExports {
  if (!loaded) {
    throw new Error("the tempo map is not loaded: await loadTempoMap() first");
  }
  return loaded;
}

/** A pointer into the module's memory: an i32 to JS, unsigned to it. */
function pointer(value: number): number {
  return value >>> 0;
}

/** The first `count` numbers the last list-returning call left. */
function output(count: number): number[] {
  return Array.from(
    new Float64Array(
      api().memory.buffer,
      pointer(api().output_buffer()),
      count,
    ),
  );
}

/** Where a tempo change is written: at a time, or at a bar and beat. */
export type TempoMapPosition =
  | { secs: number }
  | { measure: number; beat: number };

/** A gradual change's length: in beats or in measures. */
export type TempoMapTransition = { beats: number } | { measures: number };

export interface TempoMapChange {
  at: TempoMapPosition;
  bpm?: number;
  signature?: [number, number];
  /** None for a snap */
  transition?: TempoMapTransition;
}

/** A tempo map as written: its start, starting tempo and time signature,
 *  and its changes, in any order. */
export interface TempoMapInput {
  startSecs: number;
  bpm: number;
  signature: [number, number];
  changes: TempoMapChange[];
}

const CHANGE_LEN = 8;

const maps = new FinalizationRegistry<{ handle: number; generation: number }>(
  (held) => {
    if (held.generation === generation) loaded?.tm_free(held.handle);
  },
);

/** The tempo map could not be built, or could not carry on, in the module
 *  (it trapped): what uses it draws without it, and says so. */
export class TempoMapFailed extends Error {}

/** A tempo map, resolved as the player resolves one (`TempoMap::new`). */
export class TempoMap {
  readonly #values: number[];
  #handle = 0;
  #generation = -1;

  constructor(input: TempoMapInput) {
    const values = [
      input.startSecs,
      input.bpm,
      input.signature[0],
      input.signature[1],
      input.changes.length,
    ];
    for (const c of input.changes) {
      const t = c.transition;
      values.push(
        "secs" in c.at ? 0 : 1,
        "secs" in c.at ? c.at.secs : c.at.measure,
        "secs" in c.at ? 1 : c.at.beat,
        c.bpm ?? NaN,
        c.signature?.[0] ?? 0,
        c.signature?.[1] ?? 0,
        !t ? 0 : "beats" in t ? 1 : 2,
        !t ? 0 : "beats" in t ? t.beats : t.measures,
      );
    }
    console.assert(values.length === 5 + input.changes.length * CHANGE_LEN);
    this.#values = values;
    void this.handle;
  }

  /** The map in the module; made again in a module started afresh. */
  get handle(): number {
    if (this.#generation !== generation) {
      const ptr = pointer(api().input_buffer(this.#values.length));
      new Float64Array(api().memory.buffer, ptr, this.#values.length).set(
        this.#values,
      );
      const handle = api().tm_new(this.#values.length);
      // A trap answers NaN: a map that silently had no tempo would draw
      // every bar wrong, so it is a failure, not a map.
      if (Number.isNaN(handle)) {
        throw new TempoMapFailed(
          "the tempo map could not be built (it trapped)",
        );
      }
      this.#handle = handle;
      this.#generation = generation;
      maps.register(this, { handle: this.#handle, generation });
    }
    return this.#handle;
  }

  /** When bar 1 starts. */
  get startSecs(): number {
    return api().tm_start(this.handle);
  }

  /** The time of a quarter-note beat position (0 at bar 1 beat 1), under
   *  an offset; before bar 1, at the starting tempo. */
  timeAtBeats(beats: number, offsetSecs = 0): number {
    return api().tm_time_at_beats(this.handle, beats, offsetSecs);
  }

  /** The quarter-note beat position of a time, under an offset. */
  beatsAt(secs: number, offsetSecs = 0): number {
    return api().tm_beats_at(this.handle, secs, offsetSecs);
  }

  /** The quarter-note beat position of a bar and (quarter-note) beat. */
  beatsAtPosition(measure: number, beat: number): number {
    return api().tm_beats_at_position(this.handle, measure, beat);
  }

  /** The bar and (quarter-note) beat of a beat position; bar 1 for one
   *  before it. */
  positionAtBeats(beats: number): { measure: number; beat: number } {
    api().tm_position_at_beats(this.handle, beats);
    const [measure, beat] = output(2);
    return { measure, beat };
  }

  /** Measures elapsed (0 at bar 1 beat 1) at a beat position. */
  measuresAtBeats(beats: number): number {
    return api().tm_measures_at_beats(this.handle, beats);
  }

  /** The beat position at a number of measures elapsed. */
  beatsAtMeasures(measures: number): number {
    return api().tm_beats_at_measures(this.handle, measures);
  }

  /** The time signature bar `measure` is in: a change written mid-bar
   *  governs from the next downbeat. */
  signatureInMeasure(measure: number): [number, number] {
    api().tm_signature(this.handle, measure);
    const [numerator, denominator] = output(2);
    return [numerator, denominator];
  }

  /** The tempo at a time (mid-ramp, the ramp's), under an offset. */
  bpmAt(secs: number, offsetSecs = 0): number {
    return api().tm_bpm_at(this.handle, secs, offsetSecs);
  }

  /** The tempo the ramp in progress at a time heads for, if one is. */
  rampTargetAt(secs: number): number | undefined {
    const target = api().tm_ramp_target(this.handle, secs);
    return Number.isNaN(target) ? undefined : target;
  }

  /** How long `beats` beats from `atSecs` last, under an offset. */
  beatsToDuration(beats: number, atSecs: number, offsetSecs = 0): number {
    return api().tm_beats_to_duration(this.handle, beats, atSecs, offsetSecs);
  }

  /** How long `measures` measures from `atSecs` last, under an offset. */
  measuresToDuration(measures: number, atSecs: number, offsetSecs = 0): number {
    return api().tm_measures_to_duration(
      this.handle,
      measures,
      atSecs,
      offsetSecs,
    );
  }

  /** How long an `offset N measures` issued at `anchorSecs` lasts. */
  offsetDuration(measures: number, anchorSecs: number): number {
    return api().tm_offset_duration(this.handle, measures, anchorSecs);
  }

  /** The times the tempo or the time signature in effect changes. */
  boundaries(): number[] {
    return output(api().tm_boundaries(this.handle));
  }
}

/** A key a resolved map is kept with: the values it was resolved from. */
export type TempoMapKey = readonly (number | string | undefined)[];

function sameKey(a: TempoMapKey, b: TempoMapKey): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/** Resolved maps by what they were resolved from (a tempo section, a
 *  song's tempo config). It can be edited in place, so each map is kept
 *  with the values it was resolved from and checked against them: far
 *  cheaper than resolving it again. */
const mapCache = new WeakMap<object, { key: TempoMapKey; map: TempoMap }>();

export { DEFAULT_BPM } from "../gen/tempo-defaults";

/** Why a map cannot be used, by what it was resolved from, by the kind of
 *  failure: it cannot be made (`map`), or a cue walk through it cannot
 *  finish (`clock`). Each is cleared only when that kind stops failing: the
 *  map made, a walk finished. */
type Failure = "map" | "clock";
const failing = new WeakMap<object, Partial<Record<Failure, string>>>();

/** Bumped whenever a map starts or stops failing: for a view to ask again
 *  (`tempoProblemOf`) about the map it draws. */
export const tempoFailures = writable(0);
let bumpQueued = false;

function bump(): void {
  if (bumpQueued) return;
  bumpQueued = true;
  // After the render that found it, as it is found while drawing.
  queueMicrotask(() => {
    bumpQueued = false;
    tempoFailures.update((n) => n + 1);
  });
}

function setFailure(owner: object, kind: Failure, why: string | undefined) {
  const now = failing.get(owner);
  if (now?.[kind] === why) return;
  if (why === undefined && !now) return;
  failing.set(owner, { ...now, [kind]: why });
  bump();
}

/** Why the map `owner` resolves to cannot be used (it trapped), while it
 *  cannot: what uses it is drawn as without a tempo meanwhile. */
export function tempoProblemOf(owner: object | undefined): string | undefined {
  const now = owner && failing.get(owner);
  return now ? (now.clock ?? now.map) : undefined;
}

/** The map `owner` resolves to (`input`, for `key`): kept, and made again
 *  in a module started afresh before it is handed out. Throws
 *  `TempoMapFailed` when it cannot be made; once it is made, a failure to
 *  make it is over. */
export function cachedTempoMap(
  owner: object,
  key: TempoMapKey,
  input: () => TempoMapInput,
): TempoMap {
  // One that could not be made is not tried again (each try traps, and
  // starts the module again) until what it is made from, or the module,
  // changes.
  const failed = buildFailures.get(owner);
  if (failed && failed.generation === generation && sameKey(failed.key, key)) {
    throw new TempoMapFailed(failed.message);
  }
  const hit = mapCache.get(owner);
  let map: TempoMap;
  try {
    if (hit && sameKey(hit.key, key)) {
      void hit.map.handle;
      map = hit.map;
    } else {
      map = new TempoMap(input());
      mapCache.set(owner, { key, map });
    }
  } catch (e) {
    if (e instanceof TempoMapFailed) {
      buildFailures.set(owner, { key, generation, message: e.message });
    }
    throw e;
  }
  buildFailures.delete(owner);
  setFailure(owner, "map", undefined);
  return map;
}

/** The maps that could not be made, by what they are made from: with what
 *  they were made from and the module they were tried in (as it was left
 *  after the try). */
const buildFailures = new WeakMap<
  object,
  { key: TempoMapKey; generation: number; message: string }
>();

/** Which start of the module is live: it changes whenever a trap starts it
 *  again. */
export function tempoModuleGeneration(): number {
  return generation;
}

/** `use` with the map `owner` resolves to; `failed` when it could not be
 *  made or failed while used (a trap, again on a second try), rather than
 *  failing the caller: the failure is logged and kept (`tempoProblemOf`)
 *  until that kind of failure stops. `walk` says `use` is a cue walk:
 *  finishing it ends a walk's failure. */
export function withTempoMap<T>(
  owner: object,
  key: TempoMapKey,
  input: () => TempoMapInput,
  use: (map: TempoMap) => T,
  failed: () => T,
  walk = false,
): T {
  try {
    const answer = use(cachedTempoMap(owner, key, input));
    if (walk) setFailure(owner, "clock", undefined);
    return answer;
  } catch (e) {
    if (!(e instanceof TempoMapFailed)) throw e;
    const kind: Failure = e instanceof CueClockRestarted ? "clock" : "map";
    if (failing.get(owner)?.[kind] === undefined) {
      console.error("the tempo map failed; drawing without it", e);
    }
    setFailure(owner, kind, e.message);
    return failed();
  }
}

/** How long a ramp from `oldBpm` to `newBpm` written over `beats` beats
 *  lasts, in seconds. */
export function rampDuration(
  oldBpm: number,
  newBpm: number,
  beats: number,
): number {
  return api().ramp_duration(oldBpm, newBpm, beats);
}

/** Where a cue plays, as `CueClock::place` resolves it. Seconds. */
export interface PlacedCue {
  time: number;
  /** Its position in the tempo map before any offset (for a cue at a bar
   *  and beat) */
  unshifted?: number;
  /** The offset in force at the cue */
  offset: number;
  /** How long the offset the cue issues lasts (0 for none) */
  issued: number;
  /** The offset in force after it */
  after: number;
}

const clocks = new FinalizationRegistry<{
  handle: number;
  generation: number;
}>((held) => {
  if (held.generation === generation) loaded?.clock_free(held.handle);
});

/** A cue clock's module was started afresh part way through its cues. */
export class CueClockRestarted extends TempoMapFailed {
  constructor() {
    super("the tempo map was started again part way through placing cues");
  }
}

/** The running offset of a list of cues, placing each in turn, as the
 *  player places a show's cues (`CueClock`). */
export class CueClock {
  #handle = 0;
  #generation = -1;

  constructor() {
    void this.handle;
  }

  /** Whether it has placed a cue: its offsets are then its own. */
  #placed = false;

  /** The clock in the module. A clock part way through its cues cannot
   *  carry on in a module started afresh (its offsets are gone): that is a
   *  `CueClockRestarted`, for the walk to start over. */
  get handle(): number {
    if (this.#generation !== generation) {
      if (this.#placed) throw new CueClockRestarted();
      this.#handle = api().clock_new() || 0;
      this.#generation = generation;
      clocks.register(this, { handle: this.#handle, generation });
    }
    return this.#handle;
  }

  /** Places the next cue, which issues `offsetMeasures` measures of
   *  offset when `offsets` and drops the running offset first when
   *  `reset`. Undefined when it cannot be placed (a bar without a map). */
  place(
    map: TempoMap | undefined,
    at: TempoMapPosition,
    offsetMeasures: number,
    offsets: boolean,
    reset: boolean,
  ): PlacedCue | undefined {
    const clock = this.handle;
    this.#placed = true;
    const placed = api().clock_place(
      clock,
      map?.handle ?? 0,
      "secs" in at ? 0 : 1,
      "secs" in at ? at.secs : at.measure,
      "secs" in at ? 1 : at.beat,
      Math.max(0, Math.round(offsetMeasures)),
      offsets ? 1 : 0,
      reset ? 1 : 0,
    );
    if (!placed) return undefined;
    const [time, unshifted, offset, issued, after] = output(5);
    return {
      time,
      unshifted: Number.isNaN(unshifted) ? undefined : unshifted,
      offset,
      issued,
      after,
    };
  }

  /** Records that the cue just placed reaches on to `secs`. */
  reached(secs: number): void {
    api().clock_reached(this.handle, secs);
  }
}

/** One of the effects a laid-out call's cues play, as the player has it. */
export interface LaidOutEffect {
  groups: string[];
  layer: "background" | "midground" | "foreground" | null;
  blend_mode: string | null;
  /** The DSL keyword: `static`, `cycle`, ... */
  type: string;
  /** The colours it shows, `#rrggbb` */
  colors: string[];
}

/** One sequence played from a cue: the cues it expands to, every pass. */
export interface LaidOutCall {
  /** The cue it is played from, by its place among the show's cues */
  cue: number;
  /** Its place among that cue's sequence references */
  call: number;
  name: string;
  /** The offset in force at the cue */
  offset: number;
  /** Where it is played from */
  start: number;
  /** Where its last pass laid out ends */
  end: number;
  /** Where each pass laid out ends */
  pass_ends: number[];
  /** Whether the cap on cues laid out stopped it before its last cue */
  truncated: boolean;
  /** How many of its cues land past a pass's `length:` and do not play */
  dropped: number;
  /** Why it cannot be played, when it cannot (it then has no cues) */
  error?: string;
  /** Its distinct effects, which its cues name by index */
  effects: LaidOutEffect[];
  /** Each cue: when, and its effects as `[index, seconds it lasts]` */
  cues: { time: number; effects: [number, number][] }[];
}

/** The sequences each show of a file plays, or why the file did not parse. */
export interface SequenceLayout {
  shows: {
    name: string;
    calls: LaidOutCall[];
    /** The cues passed over, with why: `[cue, error]` */
    cue_errors: [number, string][];
  }[];
  /** Why nothing could be laid out (the file does not parse at all) */
  error?: string;
  /** Why the player would not load the file, though it lays out */
  problem?: string;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** Lays out the sequences the shows of a light show `source` play, as the
 *  player parses and expands them (`layout_sequence_calls`), timed through
 *  `map` where the source has no `tempo` block. Times in seconds. A call
 *  that cannot be played, or a cue that does not parse, is passed over and
 *  says why; a source that does not parse at all lays out nothing. */
export function layoutSequences(
  source: string,
  map?: TempoMap,
): SequenceLayout {
  try {
    const bytes = encoder.encode(source);
    const handle = map?.handle ?? 0;
    const ptr = pointer(api().text_buffer(bytes.length));
    new Uint8Array(api().memory.buffer, ptr, bytes.length).set(bytes);
    const len = api().layout_sequences(bytes.length, handle);
    if (!Number.isFinite(len)) {
      return { shows: [], error: "the tempo map trapped laying this out" };
    }
    const json = decoder.decode(
      new Uint8Array(api().memory.buffer, pointer(api().text_output()), len),
    );
    const parsed = JSON.parse(json) as Partial<SequenceLayout>;
    return {
      shows: parsed.shows ?? [],
      error: parsed.error,
      problem: parsed.problem,
    };
  } catch (e) {
    return { shows: [], error: e instanceof Error ? e.message : String(e) };
  }
}
