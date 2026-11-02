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

// The player's tempo map and cue clock (src/tempo.rs), compiled to
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
}

let loaded: TempoExports | undefined;
let loading: Promise<void> | undefined;

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
    const { instance } = await WebAssembly.instantiate(bytes);
    loaded = instance.exports as unknown as TempoExports;
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

/** The first `count` numbers the last list-returning call left. */
function output(count: number): number[] {
  return Array.from(
    new Float64Array(api().memory.buffer, api().output_buffer(), count),
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

const maps = new FinalizationRegistry<number>((map) => loaded?.tm_free(map));

/** A tempo map, resolved as the player resolves one (`TempoMap::new`). */
export class TempoMap {
  readonly handle: number;

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
    const ptr = api().input_buffer(values.length);
    new Float64Array(api().memory.buffer, ptr, values.length).set(values);
    this.handle = api().tm_new(values.length);
    maps.register(this, this.handle);
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

const clocks = new FinalizationRegistry<number>((clock) =>
  loaded?.clock_free(clock),
);

/** The running offset of a list of cues, placing each in turn, as the
 *  player places a show's cues (`CueClock`). */
export class CueClock {
  readonly handle = api().clock_new();

  constructor() {
    clocks.register(this, this.handle);
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
    const placed = api().clock_place(
      this.handle,
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
