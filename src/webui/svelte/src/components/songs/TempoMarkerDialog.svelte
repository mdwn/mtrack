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
  import { t } from "svelte-i18n";
  import type {
    TempoConfig,
    TempoChangeConfig,
    MetronomeConfig,
    MetronomeChangeConfig,
    SubdivisionValue,
  } from "../../lib/api/songs";
  import { fetchTempoGuess, type GuessedTempo } from "../../lib/api/songs";
  import { positionTaken, sortTempoChanges } from "../../lib/util/tempo";
  import { subdivisionParts } from "../../lib/meter";
  import NumberStepper from "../NumberStepper.svelte";
  import AccentPads from "./AccentPads.svelte";
  import MarkerDialog from "./MarkerDialog.svelte";

  /** A change marker position: measure/beat on the tempo map. */
  export interface MarkerPosition {
    measure: number;
    beat: number;
  }

  interface Props {
    tempo: TempoConfig;
    /** "start" edits the base marker; a position edits the change(s) there. */
    target: "start" | MarkerPosition;
    /** The song's metronome block; null hides the feel sections. */
    metronome?: MetronomeConfig | null;
    /** Whether this marker hosts the measure's accent/subdivision change
     * (the lowest-beat marker of its measure). */
    feelHost?: boolean;
    songName?: string;
    hasMidi?: boolean;
    canGuess?: boolean;
    ontempochange: (tempo: TempoConfig | null) => void;
    onmetronomechange?: (metronome: MetronomeConfig | null) => void;
    /** The marker was moved; the parent should retarget the dialog. */
    onmove?: (position: MarkerPosition) => void;
    onclose: () => void;
  }

  let {
    tempo,
    target,
    metronome = null,
    feelHost = true,
    songName,
    hasMidi = false,
    canGuess = false,
    ontempochange,
    onmetronomechange,
    onmove,
    onclose,
  }: Props = $props();

  const SIG_DENOMS = [1, 2, 4, 8, 16, 32];
  const SUBDIVISIONS: SubdivisionValue[] = [1, 2, 3, 4, 6, "son", "rumba"];

  function parseSig(raw: string | undefined): [number, number] {
    const match = /^\s*(\d+)\s*\/\s*(\d+)\s*$/.exec(raw ?? "");
    if (!match) return [4, 4];
    return [parseInt(match[1]), parseInt(match[2])];
  }

  // --- Entry lookup ---

  let tChangeIndex = $derived(
    target === "start"
      ? -1
      : (tempo.changes ?? []).findIndex(
          (c) => c.measure === target.measure && (c.beat ?? 1) === target.beat,
        ),
  );
  let tChange = $derived(
    tChangeIndex >= 0 ? (tempo.changes ?? [])[tChangeIndex] : undefined,
  );

  let mChangeIndex = $derived(
    target === "start" || !metronome
      ? -1
      : (metronome.changes ?? []).findIndex(
          (c) => c.measure === target.measure,
        ),
  );
  let mChange = $derived(
    mChangeIndex >= 0 ? (metronome?.changes ?? [])[mChangeIndex] : undefined,
  );

  // --- What's in effect at this marker ---

  let baseSig = $derived(parseSig(tempo.time_signature));

  /** Signature in effect at the target (including its own sig change). */
  let effectiveSig = $derived.by(() => {
    if (target === "start") return baseSig;
    let sig = baseSig;
    const sorted = [...(tempo.changes ?? [])].sort(
      (a, b) => a.measure - b.measure || (a.beat ?? 1) - (b.beat ?? 1),
    );
    for (const c of sorted) {
      const atOrBefore =
        c.measure < target.measure ||
        (c.measure === target.measure && (c.beat ?? 1) <= target.beat);
      if (!atOrBefore) break;
      if (c.time_signature) sig = parseSig(c.time_signature);
    }
    return sig;
  });
  let numerator = $derived(effectiveSig[0]);

  function defaultPattern(count: number): number[] {
    const groupStarts = [0];
    let acc = 0;
    for (const group of metronome?.accent ?? []) {
      acc += group;
      groupStarts.push(acc);
    }
    return Array.from({ length: count }, (_, i) =>
      groupStarts.includes(i) ? 3 : 1,
    );
  }

  function resizePattern(pattern: number[], count: number): number[] {
    return Array.from({ length: count }, (_, i) => pattern[i] ?? 1);
  }

  /** The accent pattern in effect just before/at a measure. */
  function patternAt(measure: number, count: number): number[] {
    let pattern = metronome?.accents;
    const sorted = [...(metronome?.changes ?? [])].sort(
      (a, b) => a.measure - b.measure,
    );
    for (const c of sorted) {
      if (c.measure <= measure && c.accents) pattern = c.accents;
    }
    return resizePattern(pattern ?? defaultPattern(count), count);
  }

  /** The subdivision in effect just before a measure. */
  function subdivisionAt(measure: number): SubdivisionValue {
    let sub: SubdivisionValue = metronome?.subdivision ?? 1;
    const sorted = [...(metronome?.changes ?? [])].sort(
      (a, b) => a.measure - b.measure,
    );
    for (const c of sorted) {
      if (c.measure < measure && c.subdivision !== undefined)
        sub = c.subdivision;
    }
    return sub;
  }

  // --- Tempo config writes ---

  function patchBase(patch: Partial<TempoConfig>) {
    const updated = { ...tempo, ...patch };
    if (!updated.start) delete updated.start;
    ontempochange(updated);
  }

  /** Replaces (or removes) this marker's tempo change entry. */
  function writeTempoChange(entry: TempoChangeConfig | null) {
    let changes = [...(tempo.changes ?? [])];
    if (entry) {
      const normalized = { ...entry };
      if (normalized.beat === 1 || normalized.beat === undefined)
        delete normalized.beat;
      if (normalized.bpm === undefined) delete normalized.bpm;
      if (!normalized.time_signature) delete normalized.time_signature;
      if (!normalized.transition) delete normalized.transition;
      if (tChangeIndex >= 0) changes[tChangeIndex] = normalized;
      else changes.push(normalized);
      // The backend rejects a map that is not in ascending (measure, beat)
      // order, so a change added or moved out of turn is re-sorted here.
      changes = sortTempoChanges(changes);
    } else if (tChangeIndex >= 0) {
      changes.splice(tChangeIndex, 1);
    }
    const updated: TempoConfig = { ...tempo, changes };
    if (changes.length === 0) delete updated.changes;
    ontempochange(updated);
  }

  function patchTempoChange(patch: Partial<TempoChangeConfig>) {
    if (target === "start") return;
    const merged: TempoChangeConfig = {
      measure: target.measure,
      beat: target.beat,
      ...tChange,
      ...patch,
    };
    // An entry that changes nothing is removed.
    const empty =
      merged.bpm === undefined && !merged.time_signature && !merged.transition;
    writeTempoChange(empty ? null : merged);
  }

  // --- Metronome config writes ---

  /** Replaces (or removes) this measure's metronome change entry. */
  function writeMetroChange(entry: MetronomeChangeConfig | null) {
    if (!metronome || !onmetronomechange) return;
    const changes = [...(metronome.changes ?? [])];
    if (entry) {
      if (mChangeIndex >= 0) changes[mChangeIndex] = entry;
      else changes.push(entry);
    } else if (mChangeIndex >= 0) {
      changes.splice(mChangeIndex, 1);
    }
    const updated: MetronomeConfig = { ...metronome, changes };
    if (changes.length === 0) delete updated.changes;
    onmetronomechange(updated);
  }

  function patchMetroChange(patch: Partial<MetronomeChangeConfig>) {
    if (target === "start") return;
    const merged: MetronomeChangeConfig = {
      measure: target.measure,
      ...mChange,
      ...patch,
    };
    if (merged.accents === undefined) delete merged.accents;
    if (merged.subdivision === undefined) delete merged.subdivision;
    const empty = !merged.accents && merged.subdivision === undefined;
    writeMetroChange(empty ? null : merged);
  }

  // --- Section toggles ---

  let bpmOn = $derived(tChange?.bpm !== undefined);
  let sigOn = $derived(!!tChange?.time_signature);
  let accentsOn = $derived(!!mChange?.accents);
  let subdivisionOn = $derived(mChange?.subdivision !== undefined);

  function toggleBpm(on: boolean) {
    if (target === "start") return;
    patchTempoChange({
      bpm: on ? effectiveBpm(target.measure) : undefined,
      transition: on ? tChange?.transition : undefined,
    });
  }

  function toggleSig(on: boolean) {
    patchTempoChange({
      time_signature: on ? `${numerator}/${baseSig[1]}` : undefined,
    });
  }

  function toggleAccents(on: boolean) {
    if (target === "start") return;
    patchMetroChange({
      accents: on ? patternAt(target.measure, numerator) : undefined,
    });
  }

  function toggleSubdivision(on: boolean) {
    if (target === "start") return;
    patchMetroChange({
      subdivision: on ? subdivisionAt(target.measure) : undefined,
    });
  }

  /** The BPM in effect at a measure (base tempo + preceding changes). */
  function effectiveBpm(measure: number): number {
    let bpm = tempo.bpm;
    const sorted = [...(tempo.changes ?? [])].sort(
      (a, b) => a.measure - b.measure,
    );
    for (const c of sorted) {
      if (c.measure <= measure && c.bpm !== undefined) bpm = c.bpm;
    }
    return bpm;
  }

  // --- Position / meter edits ---

  function moveTo(measure: number, beat: number) {
    if (target === "start") return;
    // A repeated position is rejected by the same validator as a reversed
    // one, so landing on an occupied marker is refused rather than written.
    if (positionTaken(tempo.changes ?? [], measure, beat, tChangeIndex)) return;
    if (
      mChange &&
      measure !== target.measure &&
      (metronome?.changes ?? []).some((c) => c.measure === measure)
    ) {
      return;
    }
    if (tChange) {
      patchTempoChange({ measure, beat });
    }
    if (mChange && measure !== target.measure) {
      patchMetroChange({ measure });
    }
    onmove?.({ measure, beat });
  }

  /** Sets the signature; a numerator change resizes the accent pads that
   * hang off this marker (or the base pattern). */
  function setSig(n: number, d: number) {
    if (target === "start") {
      patchBase({ time_signature: `${n}/${d}` });
      if (metronome?.accents && onmetronomechange) {
        onmetronomechange({
          ...metronome,
          accents: resizePattern(metronome.accents, n),
        });
      }
    } else {
      patchTempoChange({ time_signature: `${n}/${d}` });
      if (mChange?.accents) {
        patchMetroChange({ accents: resizePattern(mChange.accents, n) });
      }
    }
  }

  // --- Base feel edits ---

  function setBaseAccents(levels: number[]) {
    if (!metronome || !onmetronomechange) return;
    onmetronomechange({ ...metronome, accents: levels });
  }

  function setBaseSubdivision(value: SubdivisionValue) {
    if (!metronome || !onmetronomechange) return;
    const updated: MetronomeConfig = { ...metronome, subdivision: value };
    if (value === 1) delete updated.subdivision;
    onmetronomechange(updated);
  }

  // --- Transition ---

  let transitionKind = $derived.by(() => {
    if (!tChange?.transition) return "snap";
    return "measures" in tChange.transition ? "measures" : "beats";
  });

  let transitionAmount = $derived.by(() => {
    if (!tChange?.transition) return 1;
    return "measures" in tChange.transition
      ? tChange.transition.measures
      : tChange.transition.beats;
  });

  function setTransition(kind: "snap" | "beats" | "measures", amount: number) {
    if (kind === "snap") patchTempoChange({ transition: undefined });
    else if (kind === "beats")
      patchTempoChange({ transition: { beats: amount } });
    else patchTempoChange({ transition: { measures: amount } });
  }

  // --- Delete / remove ---

  function deleteMarker() {
    writeTempoChange(null);
    writeMetroChange(null);
    onclose();
  }

  function removeTempoMap() {
    ontempochange(null);
    onclose();
  }

  // --- Tap tempo ---

  let taps: number[] = [];
  let tappedBpm = $state<number | null>(null);

  function tapTempo() {
    const now = performance.now();
    if (taps.length > 0 && now - taps[taps.length - 1] > 2500) {
      taps = [];
    }
    taps.push(now);
    if (taps.length < 2) return;
    const recent = taps.slice(-9);
    const avgMs = (recent[recent.length - 1] - recent[0]) / (recent.length - 1);
    const bpm = Math.round((60000 / avgMs) * 10) / 10;
    tappedBpm = bpm;
    if (target === "start") patchBase({ bpm });
    else patchTempoChange({ bpm });
  }

  // --- Detect / guess ---

  let guessing = $state(false);

  function guessToConfig(guessed: GuessedTempo): TempoConfig {
    const config: TempoConfig = {
      bpm: guessed.bpm,
      time_signature: `${guessed.time_signature[0]}/${guessed.time_signature[1]}`,
    };
    if (guessed.start_seconds > 0) config.start = guessed.start_seconds;
    const changes = guessed.changes.map((c) => {
      const entry: TempoChangeConfig = { measure: c.measure };
      if (c.beat > 1) entry.beat = c.beat;
      entry.bpm = c.bpm;
      entry.time_signature = `${c.time_signature[0]}/${c.time_signature[1]}`;
      if (c.transition_beats) entry.transition = { beats: c.transition_beats };
      return entry;
    });
    if (changes.length > 0) config.changes = sortTempoChanges(changes);
    return config;
  }

  async function detectTempo() {
    if (!songName) return;
    guessing = true;
    try {
      const result = await fetchTempoGuess(songName);
      if (result) ontempochange(guessToConfig(result.tempo));
    } finally {
      guessing = false;
    }
  }
</script>

{#snippet sigSteppers(sig: [number, number])}
  <div class="stepper-row sig-row">
    <NumberStepper
      value={sig[0]}
      min={1}
      max={32}
      ariaLabel="numerator"
      onchange={(v) => setSig(v, sig[1])}
    />
    <span class="sig-sep">/</span>
    <NumberStepper
      value={sig[1]}
      values={SIG_DENOMS}
      ariaLabel="denominator"
      onchange={(v) => setSig(sig[0], v)}
    />
  </div>
{/snippet}

{#snippet subdivisionSelect(
  value: SubdivisionValue,
  beatDen: number,
  onpick: (v: SubdivisionValue) => void,
)}
  <div class="subdiv-options">
    {#each SUBDIVISIONS as sub (sub)}
      {@const parts = subdivisionParts(sub, beatDen)}
      <button
        type="button"
        class="subdiv-option"
        class:active={value === sub}
        onclick={() => onpick(sub)}
      >
        <span class="subdiv-glyph">{parts.glyph}</span>
        <span class="subdiv-name">{$t(parts.nameKey)}</span>
      </button>
    {/each}
  </div>
{/snippet}

<MarkerDialog
  title={target === "start"
    ? $t("tempo.marker.startTitle")
    : $t("tempo.marker.changeTitle")}
  {onclose}
>
  {#if target === "start"}
    <div class="dialog-section">
      <span class="section-label">{$t("tempo.bpm")}</span>
      <div class="stepper-row">
        <NumberStepper
          value={tempo.bpm}
          min={1}
          max={999}
          ariaLabel={$t("tempo.bpm")}
          onchange={(v) => patchBase({ bpm: v })}
        />
        <button type="button" class="btn tap-btn" onclick={tapTempo}
          >{$t("tempo.marker.tap")}{tappedBpm !== null
            ? ` · ${tappedBpm}`
            : ""}</button
        >
      </div>
    </div>

    <div class="dialog-section">
      <span class="section-label">{$t("tempo.timeSignature")}</span>
      {@render sigSteppers(baseSig)}
    </div>

    {#if metronome}
      <div class="dialog-section">
        <span class="section-label">{$t("metronome.accents")}</span>
        <AccentPads
          levels={resizePattern(
            metronome.accents ?? defaultPattern(numerator),
            numerator,
          )}
          onchange={setBaseAccents}
        />
      </div>

      <div class="dialog-section">
        <span class="section-label">{$t("metronome.subdivision")}</span>
        {@render subdivisionSelect(
          metronome.subdivision ?? 1,
          baseSig[1],
          setBaseSubdivision,
        )}
      </div>
    {/if}

    <div class="dialog-section">
      <span class="section-label">{$t("tempo.startOffset")}</span>
      <div class="stepper-row">
        <NumberStepper
          value={tempo.start ?? 0}
          min={0}
          max={600}
          step={0.05}
          decimals={2}
          suffix="s"
          ariaLabel={$t("tempo.startOffset")}
          onchange={(v) => patchBase({ start: v })}
        />
      </div>
    </div>

    {#if (hasMidi || canGuess) && songName}
      <div class="dialog-section">
        <button
          type="button"
          class="btn detect-btn"
          onclick={detectTempo}
          disabled={guessing}
          >{guessing
            ? $t("common.loading")
            : hasMidi
              ? $t("tempo.marker.detectMidi")
              : $t("tempo.marker.guessGrid")}</button
        >
      </div>
    {/if}
  {:else}
    <div class="dialog-section">
      <span class="section-label">{$t("tempo.marker.position")}</span>
      <div class="stepper-row">
        <div class="labeled-stepper">
          <span class="mini-label">{$t("pilot.measure")}</span>
          <NumberStepper
            value={target.measure}
            min={1}
            max={9999}
            ariaLabel={$t("pilot.measure")}
            onchange={(v) => moveTo(v, target.beat)}
          />
        </div>
        {#if tChange}
          <div class="labeled-stepper">
            <span class="mini-label">{$t("pilot.beat")}</span>
            <NumberStepper
              value={target.beat}
              min={1}
              max={32}
              ariaLabel={$t("pilot.beat")}
              onchange={(v) => moveTo(target.measure, v)}
            />
          </div>
        {/if}
      </div>
    </div>

    <div class="dialog-section" class:section-off={!bpmOn}>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={bpmOn}
          onchange={(e) => toggleBpm((e.target as HTMLInputElement).checked)}
        />
        <span class="section-label">{$t("tempo.bpm")}</span>
      </label>
      {#if bpmOn && tChange?.bpm !== undefined}
        <div class="stepper-row">
          <NumberStepper
            value={tChange.bpm}
            min={1}
            max={999}
            ariaLabel={$t("tempo.bpm")}
            onchange={(v) => patchTempoChange({ bpm: v })}
          />
          <button type="button" class="btn tap-btn" onclick={tapTempo}
            >{$t("tempo.marker.tap")}{tappedBpm !== null
              ? ` · ${tappedBpm}`
              : ""}</button
          >
        </div>
        <div class="transition-row">
          <span class="mini-label">{$t("tempo.transition")}</span>
          <div class="segmented">
            <button
              type="button"
              class="seg-btn"
              class:active={transitionKind === "snap"}
              onclick={() => setTransition("snap", 1)}
              >{$t("tempo.marker.transitionSnap")}</button
            >
            <button
              type="button"
              class="seg-btn"
              class:active={transitionKind === "beats"}
              onclick={() => setTransition("beats", transitionAmount)}
              >{$t("tempo.marker.transitionBeats")}</button
            >
            <button
              type="button"
              class="seg-btn"
              class:active={transitionKind === "measures"}
              onclick={() => setTransition("measures", transitionAmount)}
              >{$t("tempo.marker.transitionMeasures")}</button
            >
          </div>
          {#if transitionKind !== "snap"}
            <NumberStepper
              value={transitionAmount}
              min={1}
              max={64}
              ariaLabel={$t("tempo.transition")}
              onchange={(v) =>
                setTransition(transitionKind as "beats" | "measures", v)}
            />
          {/if}
        </div>
      {/if}
    </div>

    <div class="dialog-section" class:section-off={!sigOn}>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={sigOn}
          onchange={(e) => toggleSig((e.target as HTMLInputElement).checked)}
        />
        <span class="section-label">{$t("tempo.timeSignature")}</span>
      </label>
      {#if sigOn && tChange?.time_signature}
        {@render sigSteppers(parseSig(tChange.time_signature))}
      {/if}
    </div>

    {#if metronome && feelHost}
      <div class="dialog-section" class:section-off={!accentsOn}>
        <label class="toggle-row">
          <input
            type="checkbox"
            checked={accentsOn}
            onchange={(e) =>
              toggleAccents((e.target as HTMLInputElement).checked)}
          />
          <span class="section-label">{$t("metronome.accents")}</span>
        </label>
        {#if accentsOn && mChange?.accents}
          <AccentPads
            levels={resizePattern(mChange.accents, numerator)}
            onchange={(levels) => patchMetroChange({ accents: levels })}
          />
        {/if}
      </div>

      <div class="dialog-section" class:section-off={!subdivisionOn}>
        <label class="toggle-row">
          <input
            type="checkbox"
            checked={subdivisionOn}
            onchange={(e) =>
              toggleSubdivision((e.target as HTMLInputElement).checked)}
          />
          <span class="section-label">{$t("metronome.subdivision")}</span>
        </label>
        {#if subdivisionOn && mChange?.subdivision !== undefined}
          {@render subdivisionSelect(
            mChange.subdivision,
            effectiveSig[1],
            (v) => patchMetroChange({ subdivision: v }),
          )}
        {/if}
      </div>
    {:else if !metronome}
      <p class="feel-note">{$t("tempo.marker.noMetronome")}</p>
    {/if}
  {/if}

  {#snippet actions()}
    {#if target === "start"}
      <button type="button" class="btn btn-danger" onclick={removeTempoMap}
        >{$t("tempo.marker.removeMap")}</button
      >
    {:else}
      <button type="button" class="btn btn-danger" onclick={deleteMarker}
        >{$t("tempo.marker.deleteMarker")}</button
      >
    {/if}
  {/snippet}
</MarkerDialog>

<style>
  .dialog-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-bottom: 14px;
  }
  .dialog-section:not(:last-child) {
    border-bottom: 1px solid var(--border);
  }
  .section-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    font-weight: 600;
  }
  .section-off {
    padding-bottom: 10px;
  }
  .mini-label {
    font-size: 10px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .labeled-stepper {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .stepper-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .sig-row {
    gap: 6px;
  }
  .sig-sep {
    font-size: 20px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
    min-height: 24px;
  }
  .toggle-row input[type="checkbox"] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
    margin: 0;
  }
  .tap-btn {
    min-height: 44px;
    min-width: 72px;
    font-weight: 600;
  }
  .detect-btn {
    min-height: 40px;
    align-self: flex-start;
  }
  .transition-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }
  .seg-btn {
    border: none;
    background: var(--bg-input);
    color: var(--text-muted);
    padding: 0 14px;
    min-height: 40px;
    font-size: 13px;
    cursor: pointer;
  }
  .seg-btn + .seg-btn {
    border-left: 1px solid var(--border);
  }
  .seg-btn.active {
    background: var(--accent);
    color: var(--bg);
    font-weight: 600;
  }
  .subdiv-options {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .subdiv-option {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text);
    border-radius: 10px;
    padding: 6px 12px;
    min-height: 52px;
    min-width: 64px;
    cursor: pointer;
    touch-action: manipulation;
  }
  .subdiv-option.active {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
    font-weight: 600;
  }
  .subdiv-glyph {
    font-size: 17px;
    line-height: 1.1;
  }
  .subdiv-name {
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }
  .subdiv-option.active .subdiv-name {
    color: var(--bg);
  }
  .feel-note {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
    margin: 0;
  }
</style>
