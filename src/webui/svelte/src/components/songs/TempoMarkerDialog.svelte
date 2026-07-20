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
  import type { TempoConfig, TempoChangeConfig } from "../../lib/api/songs";
  import { fetchTempoGuess, type GuessedTempo } from "../../lib/api/songs";
  import NumberStepper from "../NumberStepper.svelte";
  import MarkerDialog from "./MarkerDialog.svelte";

  interface Props {
    tempo: TempoConfig;
    /** "start" edits the base tempo marker; a number edits changes[n]. */
    target: "start" | number;
    songName?: string;
    hasMidi?: boolean;
    canGuess?: boolean;
    onchange: (tempo: TempoConfig | null) => void;
    onclose: () => void;
  }

  let {
    tempo,
    target,
    songName,
    hasMidi = false,
    canGuess = false,
    onchange,
    onclose,
  }: Props = $props();

  const SIG_DENOMS = [1, 2, 4, 8, 16, 32];

  function parseSig(raw: string | undefined): [number, number] {
    const match = /^\s*(\d+)\s*\/\s*(\d+)\s*$/.exec(raw ?? "");
    if (!match) return [4, 4];
    return [parseInt(match[1]), parseInt(match[2])];
  }

  let change = $derived(
    typeof target === "number" ? (tempo.changes ?? [])[target] : undefined,
  );

  // --- Base marker edits ---

  function patchBase(patch: Partial<TempoConfig>) {
    const updated = { ...tempo, ...patch };
    if (!updated.start) delete updated.start;
    onchange(updated);
  }

  let baseSig = $derived(parseSig(tempo.time_signature));

  // --- Change marker edits ---

  function patchChange(patch: Partial<TempoChangeConfig>) {
    if (typeof target !== "number") return;
    const changes = [...(tempo.changes ?? [])];
    const merged = { ...changes[target], ...patch };
    if (merged.beat === 1 || merged.beat === undefined) delete merged.beat;
    if (merged.bpm === undefined) delete merged.bpm;
    if (!merged.time_signature) delete merged.time_signature;
    if (!merged.transition) delete merged.transition;
    changes[target] = merged;
    onchange({ ...tempo, changes });
  }

  function deleteChange() {
    if (typeof target !== "number") return;
    const changes = (tempo.changes ?? []).filter((_, i) => i !== target);
    const updated: TempoConfig = { ...tempo, changes };
    if (changes.length === 0) delete updated.changes;
    onchange(updated);
    onclose();
  }

  function removeTempoMap() {
    onchange(null);
    onclose();
  }

  let changeSig = $derived(
    change?.time_signature ? parseSig(change.time_signature) : null,
  );

  let transitionKind = $derived.by(() => {
    if (!change?.transition) return "snap";
    return "measures" in change.transition ? "measures" : "beats";
  });

  let transitionAmount = $derived.by(() => {
    if (!change?.transition) return 1;
    return "measures" in change.transition
      ? change.transition.measures
      : change.transition.beats;
  });

  function setTransition(kind: "snap" | "beats" | "measures", amount: number) {
    if (kind === "snap") patchChange({ transition: undefined });
    else if (kind === "beats") patchChange({ transition: { beats: amount } });
    else patchChange({ transition: { measures: amount } });
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
    if (typeof target === "number") patchChange({ bpm });
    else patchBase({ bpm });
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
    return config;
  }

  async function detectTempo() {
    if (!songName) return;
    guessing = true;
    try {
      const result = await fetchTempoGuess(songName);
      if (result) onchange(guessToConfig(result.tempo));
    } finally {
      guessing = false;
    }
  }
</script>

<MarkerDialog
  title={typeof target === "number"
    ? $t("tempo.marker.changeTitle")
    : $t("tempo.marker.startTitle")}
  {onclose}
>
  {#if typeof target === "number"}
    {#if change}
      <div class="field-block">
        <span class="field-label">{$t("tempo.marker.position")}</span>
        <div class="stepper-row">
          <div class="labeled-stepper">
            <span class="mini-label">{$t("pilot.measure")}</span>
            <NumberStepper
              value={change.measure}
              min={1}
              max={9999}
              ariaLabel={$t("pilot.measure")}
              onchange={(v) => patchChange({ measure: v })}
            />
          </div>
          <div class="labeled-stepper">
            <span class="mini-label">{$t("pilot.beat")}</span>
            <NumberStepper
              value={change.beat ?? 1}
              min={1}
              max={32}
              ariaLabel={$t("pilot.beat")}
              onchange={(v) => patchChange({ beat: v })}
            />
          </div>
        </div>
      </div>

      <div class="field-block">
        <label class="toggle-row">
          <input
            type="checkbox"
            checked={change.bpm !== undefined}
            disabled={change.bpm !== undefined && !change.time_signature}
            onchange={(e) =>
              patchChange({
                bpm: (e.target as HTMLInputElement).checked
                  ? tempo.bpm
                  : undefined,
              })}
          />
          <span class="field-label">{$t("tempo.bpm")}</span>
        </label>
        {#if change.bpm !== undefined}
          <div class="stepper-row">
            <NumberStepper
              value={change.bpm}
              min={1}
              max={999}
              ariaLabel={$t("tempo.bpm")}
              onchange={(v) => patchChange({ bpm: v })}
            />
            <button type="button" class="btn tap-btn" onclick={tapTempo}
              >{$t("tempo.marker.tap")}{tappedBpm !== null
                ? ` · ${tappedBpm}`
                : ""}</button
            >
          </div>
        {/if}
      </div>

      <div class="field-block">
        <label class="toggle-row">
          <input
            type="checkbox"
            checked={!!change.time_signature}
            disabled={!!change.time_signature && change.bpm === undefined}
            onchange={(e) =>
              patchChange({
                time_signature: (e.target as HTMLInputElement).checked
                  ? `${baseSig[0]}/${baseSig[1]}`
                  : undefined,
              })}
          />
          <span class="field-label">{$t("tempo.timeSignature")}</span>
        </label>
        {#if changeSig}
          <div class="stepper-row sig-row">
            <NumberStepper
              value={changeSig[0]}
              min={1}
              max={32}
              ariaLabel="numerator"
              onchange={(v) =>
                patchChange({ time_signature: `${v}/${changeSig![1]}` })}
            />
            <span class="sig-sep">/</span>
            <NumberStepper
              value={changeSig[1]}
              values={SIG_DENOMS}
              ariaLabel="denominator"
              onchange={(v) =>
                patchChange({ time_signature: `${changeSig![0]}/${v}` })}
            />
          </div>
        {/if}
      </div>

      <div class="field-block">
        <span class="field-label">{$t("tempo.transition")}</span>
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
          <div class="stepper-row">
            <NumberStepper
              value={transitionAmount}
              min={1}
              max={64}
              ariaLabel={$t("tempo.transition")}
              onchange={(v) =>
                setTransition(transitionKind as "beats" | "measures", v)}
            />
          </div>
        {/if}
      </div>
    {/if}
  {:else}
    <div class="field-block">
      <span class="field-label">{$t("tempo.bpm")}</span>
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

    <div class="field-block">
      <span class="field-label">{$t("tempo.timeSignature")}</span>
      <div class="stepper-row sig-row">
        <NumberStepper
          value={baseSig[0]}
          min={1}
          max={32}
          ariaLabel="numerator"
          onchange={(v) => patchBase({ time_signature: `${v}/${baseSig[1]}` })}
        />
        <span class="sig-sep">/</span>
        <NumberStepper
          value={baseSig[1]}
          values={SIG_DENOMS}
          ariaLabel="denominator"
          onchange={(v) => patchBase({ time_signature: `${baseSig[0]}/${v}` })}
        />
      </div>
    </div>

    <div class="field-block">
      <span class="field-label">{$t("tempo.startOffset")}</span>
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
  {/if}

  {#snippet actions()}
    {#if typeof target === "number"}
      <button type="button" class="btn btn-danger" onclick={deleteChange}
        >{$t("tempo.marker.deleteMarker")}</button
      >
    {:else}
      <button type="button" class="btn btn-danger" onclick={removeTempoMap}
        >{$t("tempo.marker.removeMap")}</button
      >
    {/if}
  {/snippet}
</MarkerDialog>

<style>
  .field-block {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .field-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
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
    gap: 8px;
    cursor: pointer;
    min-height: 28px;
  }
  .toggle-row input[type="checkbox"] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
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
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    align-self: flex-start;
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
</style>
