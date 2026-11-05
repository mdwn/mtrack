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
    BeatGrid,
    TempoConfig,
    TempoChangeConfig,
    MetronomeConfig,
    MetronomeChangeConfig,
    SubdivisionValue,
  } from "../../lib/api/songs";
  import { fetchTempoGuess, type GuessedTempo } from "../../lib/api/songs";
  import { rules } from "../../lib/rules";
  import { subdivisionIcon } from "../../lib/meter";
  import { sectionToConfigSnapped } from "../../lib/tempoConvert";
  import type { TempoSection } from "../../lib/lighting/types";
  import NoteIcon from "../NoteIcon.svelte";
  import NumberStepper from "../NumberStepper.svelte";
  import AccentPads from "./AccentPads.svelte";
  import MarkerDialog from "./MarkerDialog.svelte";
  import PositionPicker from "./PositionPicker.svelte";
  import PlayheadCapture from "./PlayheadCapture.svelte";

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
    /** The song's measure count, bounding the position picker. */
    maxMeasure?: number;
    /** The preview playhead in seconds, when this song is in the player. */
    playheadTime?: number | null;
    /** The song's beat grid, for snapping time-anchored imports. */
    beatGrid?: BeatGrid | null;
    /** Light shows with their own tempo maps (import sources). */
    lightShowTempos?: { file: string; tempo: TempoSection }[];
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
    maxMeasure = 9999,
    playheadTime = null,
    beatGrid = null,
    lightShowTempos = [],
    ontempochange,
    onmetronomechange,
    onmove,
    onclose,
  }: Props = $props();

  const SIG_DENOMS = [1, 2, 4, 8, 16, 32];
  const SUBDIVISIONS: SubdivisionValue[] = [1, 2, 3, 4, 6, "son", "rumba"];

  /** Shown for a signature the player's parser refuses: the steppers need
   * a value to start from. */
  const COMMON_TIME: [number, number] = [4, 4];

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

  let baseSig = $derived(
    rules.signature_parse(tempo.time_signature ?? "") ?? COMMON_TIME,
  );

  /** The signature the changes up to and including this marker set
   * (`sig_set`): the metre the marker's own accent pads are for, so a marker
   * that changes to 7/8 on beat 3 shows seven pads — the bar it sits in is
   * still counted in the old metre (`sig`), which the bar readouts keep.
   * The base signature while the tempo block makes no map. */
  let effectiveSig = $derived.by((): [number, number] => {
    if (target === "start") return baseSig;
    return (
      rules.tempo_query(
        { config: tempo },
        { measure: target.measure, beat: target.beat },
      )?.sig_set ?? baseSig
    );
  });

  /** Why this tempo map cannot be saved, as the player says it; nothing
   * while it can. */
  let verdict = $derived(rules.tempo_validate(tempo));
  let numerator = $derived(effectiveSig[0]);

  /** The player's tempo map answers for the measure the picker asks about;
   * 4/4 while the tempo block makes no map. */
  function meterAt(measure: number): [number, number] | undefined {
    return rules.tempo_query({ config: tempo }, { measure, beat: 1 })?.sig;
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
    } else if (tChangeIndex >= 0) {
      changes.splice(tChangeIndex, 1);
    }
    // The backend rejects a map that is not in ascending (measure, beat)
    // order, so a change added or moved out of turn is put in its place by
    // the player's own ordering (which also drops an empty `changes`).
    const updated = { ...tempo, changes };
    ontempochange(rules.tempo_normalize(updated)?.config ?? updated);
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

  /** On: the pattern in effect at this measure, as the renderer resolves
   * it, so the pads start from what already plays here. */
  function toggleAccents(on: boolean) {
    if (target === "start" || !metronome) return;
    patchMetroChange({
      accents: on
        ? rules.accent_pattern(metronome, target.measure, numerator)
        : undefined,
    });
  }

  /** On: the subdivision in effect at this measure, as the renderer
   * resolves it, so the change starts from what already plays here. */
  function toggleSubdivision(on: boolean) {
    if (target === "start" || !metronome) return;
    patchMetroChange({
      subdivision: on
        ? (rules.subdivision_at(metronome, target.measure) ?? 1)
        : undefined,
    });
  }

  /** The BPM in force at a measure's downbeat, as the player plays it
   * (inside a ramp, the ramp's value there), to the hundredth a `tempo:`
   * block is written to — a formatting choice for the new value, not a
   * rule; the base tempo while the tempo block makes no map. */
  function effectiveBpm(measure: number): number {
    const bpm = rules.tempo_query({ config: tempo }, { measure, beat: 1 })?.bpm;
    return bpm === undefined ? tempo.bpm : Math.round(bpm * 100) / 100;
  }

  // --- Position / meter edits ---

  /** The playhead's beat — a tempo change can only sit on one — as the
   * player snaps a time to its nearest beat. */
  let playheadTarget = $derived(
    playheadTime === null || !beatGrid
      ? null
      : (rules.grid_query(beatGrid, {
          secs: playheadTime,
          snap: { step: 1 },
        }) ?? null),
  );

  /** Whether another change already holds a position: the player's own
   * ordering, asked whether placing this marker's change there would cost
   * one change more than the others already cost among themselves (so a
   * repeat elsewhere in a hand-written config does not block every move).
   * `to_tempo_map` rejects a repeated position as firmly as a reversed one,
   * so an occupied position is refused rather than written. */
  function positionHeld(measure: number, beat: number): boolean {
    const others = (tempo.changes ?? []).filter((_, i) => i !== tChangeIndex);
    const moved = { ...(tChange ?? { measure }), measure, beat };
    const before = rules.tempo_normalize({ ...tempo, changes: others });
    const after = rules.tempo_normalize({
      ...tempo,
      changes: [...others, moved],
    });
    return (after?.duplicates ?? 0) > (before?.duplicates ?? 0);
  }

  /** Refused for the same reason a manual move would be: something is there. */
  let canCapture = $derived(
    playheadTarget !== null &&
      target !== "start" &&
      !positionHeld(playheadTarget.measure, playheadTarget.beat),
  );

  function moveTo(measure: number, beat: number) {
    if (target === "start") return;
    if (positionHeld(measure, beat)) return;
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
          accents: rules.accent_pattern({ ...metronome, changes: [] }, 1, n),
        });
      }
    } else {
      patchTempoChange({ time_signature: `${n}/${d}` });
      if (mChange?.accents && metronome) {
        patchMetroChange({
          accents: rules.accent_pattern(metronome, target.measure, n),
        });
      }
    }
  }

  // --- Base feel edits ---

  let baseAccentsOn = $derived((metronome?.accents?.length ?? 0) > 0);
  let baseSubdivisionOn = $derived(
    metronome?.subdivision !== undefined && metronome.subdivision !== 1,
  );

  function setBaseAccents(levels: number[]) {
    if (!metronome || !onmetronomechange) return;
    onmetronomechange({ ...metronome, accents: levels });
  }

  /** Off = no explicit pattern: back to the grouping / downbeat default. */
  function toggleBaseAccents(on: boolean) {
    if (!metronome || !onmetronomechange) return;
    const updated: MetronomeConfig = { ...metronome };
    // The pattern the grouping (or the plain downbeat) gives, as the
    // renderer plays it with no explicit pattern.
    if (on)
      updated.accents = rules.accent_pattern(
        { ...metronome, accents: [], changes: [] },
        1,
        numerator,
      );
    else delete updated.accents;
    onmetronomechange(updated);
  }

  function toggleBaseSubdivision(on: boolean) {
    if (!metronome || !onmetronomechange) return;
    const updated: MetronomeConfig = { ...metronome };
    if (on) updated.subdivision = 2;
    else delete updated.subdivision;
    onmetronomechange(updated);
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
    if (changes.length > 0) config.changes = changes;
    return rules.tempo_normalize(config)?.config ?? config;
  }

  let importNote = $state("");

  /** Imports a light show's tempo map as the song tempo. Time-anchored
   * changes snap to the nearest measure/beat on the grid; the note says what
   * the conversion cost, since a show can hold positions the song cannot. */
  function importFromLightShow(source: { file: string; tempo: TempoSection }) {
    const result = sectionToConfigSnapped(source.tempo, beatGrid);
    ontempochange(result.config);
    const parts: string[] = [];
    if (result.snapped > 0)
      parts.push(
        $t("tempo.marker.importSnapped", {
          values: { count: result.snapped },
        }),
      );
    if (result.dropped > 0)
      parts.push(
        $t("tempo.marker.importDropped", {
          values: { count: result.dropped },
        }),
      );
    if (result.duplicates > 0)
      parts.push(
        $t("tempo.marker.importDuplicates", {
          values: { count: result.duplicates },
        }),
      );
    importNote = parts.join(" · ");
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
      {@const icon = subdivisionIcon(sub, beatDen)}
      <button
        type="button"
        class="subdiv-option"
        class:active={value === sub}
        onclick={() => onpick(sub)}
      >
        <span class="subdiv-glyph">
          {#if icon.kind === "clave"}
            <svg class="clave-icon" viewBox="0 0 52 10" aria-hidden="true">
              {#each icon.pattern as hit (hit)}
                <circle cx={4 + hit * 3} cy="5" r="2.7" fill="currentColor" />
              {/each}
            </svg>
          {:else}
            <NoteIcon den={icon.den} count={icon.count} tuplet={icon.tuplet} />
          {/if}
        </span>
        <span class="subdiv-name">{$t(icon.nameKey)}</span>
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
  {#if verdict}
    <p class="verdict" role="alert" data-testid="tempo-verdict">{verdict}</p>
  {/if}
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
      <div class="dialog-section" class:section-off={!baseAccentsOn}>
        <label class="toggle-row">
          <input
            type="checkbox"
            checked={baseAccentsOn}
            onchange={(e) =>
              toggleBaseAccents((e.target as HTMLInputElement).checked)}
          />
          <span class="section-label">{$t("metronome.accents")}</span>
        </label>
        {#if baseAccentsOn && metronome.accents}
          <AccentPads
            levels={rules.accent_pattern(
              { ...metronome, changes: [] },
              1,
              numerator,
            ) ?? []}
            onchange={setBaseAccents}
          />
        {/if}
      </div>

      <div class="dialog-section" class:section-off={!baseSubdivisionOn}>
        <label class="toggle-row">
          <input
            type="checkbox"
            checked={baseSubdivisionOn}
            onchange={(e) =>
              toggleBaseSubdivision((e.target as HTMLInputElement).checked)}
          />
          <span class="section-label">{$t("metronome.subdivision")}</span>
        </label>
        {#if baseSubdivisionOn && metronome.subdivision !== undefined}
          {@render subdivisionSelect(
            metronome.subdivision,
            baseSig[1],
            setBaseSubdivision,
          )}
        {/if}
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

    {#if ((hasMidi || canGuess) && songName) || lightShowTempos.length > 0}
      <div class="dialog-section">
        <div class="stepper-row">
          {#if (hasMidi || canGuess) && songName}
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
          {/if}
          {#each lightShowTempos as source (source.file)}
            <button
              type="button"
              class="btn detect-btn"
              title={source.file}
              onclick={() => importFromLightShow(source)}
              >{$t("tempo.marker.importLighting", {
                values: {
                  file: source.file.split("/").pop() ?? source.file,
                },
              })}</button
            >
          {/each}
        </div>
        {#if importNote}
          <span class="import-note">{importNote}</span>
        {/if}
      </div>
    {/if}
  {:else}
    <div class="dialog-section">
      <span class="section-label">{$t("tempo.marker.position")}</span>
      <PositionPicker
        label={$t("position.marker")}
        value={{ kind: "beat", measure: target.measure, beat: target.beat }}
        stores="beat"
        {maxMeasure}
        {beatGrid}
        beatsIn={(m) => meterAt(m)?.[0] ?? COMMON_TIME[0]}
        sigOf={(m) => meterAt(m)?.join("/")}
        ghostTime={playheadTime}
        onchange={(v) => v.kind === "beat" && moveTo(v.measure, v.beat)}
      />
      {#if playheadTime !== null && playheadTarget}
        <PlayheadCapture
          time={playheadTime}
          position="m{playheadTarget.measure} · b{playheadTarget.beat}"
        >
          {#snippet actions()}
            <button
              type="button"
              class="btn btn-sm"
              disabled={!canCapture}
              onclick={() =>
                playheadTarget &&
                moveTo(playheadTarget.measure, playheadTarget.beat)}
              >{$t("position.usePlayhead")}</button
            >
          {/snippet}
        </PlayheadCapture>
      {/if}
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
        {@render sigSteppers(
          rules.signature_parse(tChange.time_signature) ?? COMMON_TIME,
        )}
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
            levels={rules.accent_pattern(
              metronome,
              target.measure,
              numerator,
            ) ?? []}
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
  .import-note {
    font-size: 12px;
    color: var(--text-muted);
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
    display: flex;
    align-items: center;
    justify-content: center;
    height: 28px;
  }
  .clave-icon {
    height: 10px;
    width: auto;
    display: block;
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
  .verdict {
    margin: 0 0 8px;
    font-size: 12px;
    color: var(--text-danger, #c33);
  }
</style>
