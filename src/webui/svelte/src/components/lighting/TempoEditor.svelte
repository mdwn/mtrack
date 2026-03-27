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
  import type { TempoSection, TempoChange } from "../../lib/lighting/types";
  import { fetchTempoGuess, type GuessedTempo } from "../../lib/api/songs";
  import TimestampInput from "./TimestampInput.svelte";

  interface Props {
    tempo: TempoSection | undefined;
    onchange: (tempo: TempoSection | undefined) => void;
    onclose?: () => void;
    songName?: string;
    hasBeatGrid?: boolean;
    hasMidi?: boolean;
  }

  let {
    tempo,
    onchange,
    onclose,
    songName,
    hasBeatGrid = false,
    hasMidi = false,
  }: Props = $props();

  let guessSource = $state<"midi" | "beat_grid" | null>(null);
  let guessing = $state(false);

  function convertGuessToTempo(guessed: GuessedTempo): TempoSection {
    return {
      start: { value: guessed.start_seconds, unit: "s" },
      bpm: guessed.bpm,
      time_signature: guessed.time_signature,
      changes: guessed.changes.map((c) => {
        const change: TempoChange = {
          timestamp: { type: "measure_beat", measure: c.measure, beat: c.beat },
          bpm: c.bpm,
          time_signature: c.time_signature,
        };
        if (c.transition_beats) {
          change.transition = `${c.transition_beats}`;
        }
        return change;
      }),
    };
  }

  async function guessTempoMap() {
    if (!songName) return;
    guessing = true;
    try {
      const result = await fetchTempoGuess(songName);
      if (result) {
        guessSource = result.source;
        onchange(convertGuessToTempo(result.tempo));
      }
    } finally {
      guessing = false;
    }
  }

  let expanded = $state(true);

  function enableTempo() {
    onchange({
      start: { value: 0, unit: "s" },
      bpm: 120,
      time_signature: [4, 4],
      changes: [],
    });
  }

  function disableTempo() {
    onchange(undefined);
  }

  function updateBpm(val: string) {
    if (!tempo) return;
    const bpm = parseFloat(val);
    if (isNaN(bpm)) return;
    onchange({ ...tempo, bpm });
  }

  function updateStartValue(val: string) {
    if (!tempo) return;
    const v = parseFloat(val);
    if (isNaN(v)) return;
    onchange({ ...tempo, start: { ...tempo.start, value: v } });
  }

  function updateStartUnit(unit: string) {
    if (!tempo) return;
    onchange({
      ...tempo,
      start: { ...tempo.start, unit: unit as "ms" | "s" },
    });
  }

  function updateTimeSignature(num: string, den: string) {
    if (!tempo) return;
    const n = parseInt(num) || tempo.time_signature[0];
    const d = parseInt(den) || tempo.time_signature[1];
    onchange({ ...tempo, time_signature: [n, d] });
  }

  function updateChange(index: number, change: TempoChange) {
    if (!tempo) return;
    const changes = [...tempo.changes];
    changes[index] = change;
    onchange({ ...tempo, changes });
  }

  function deleteChange(index: number) {
    if (!tempo) return;
    onchange({
      ...tempo,
      changes: tempo.changes.filter((_, i) => i !== index),
    });
  }

  function addChange() {
    if (!tempo) return;
    const lastMeasure =
      tempo.changes.length > 0
        ? (tempo.changes[tempo.changes.length - 1].timestamp.measure ?? 1) + 8
        : 8;
    onchange({
      ...tempo,
      changes: [
        ...tempo.changes,
        {
          timestamp: { type: "measure_beat", measure: lastMeasure, beat: 1 },
          bpm: tempo.bpm,
        },
      ],
    });
  }
</script>

<div class="tempo-editor">
  <div class="tempo-header">
    <button class="expand-btn" onclick={() => (expanded = !expanded)}>
      {expanded ? "\u25BC" : "\u25B6"}
    </button>
    <span class="section-title">{$t("tempo.title")}</span>
    {#if tempo}
      <span class="tempo-info"
        >{tempo.bpm}
        {$t("tempo.bpm")}, {tempo.time_signature[0]}/{tempo
          .time_signature[1]}</span
      >
      {#if guessSource}
        <span class="estimated-badge">
          {guessSource === "midi" ? "from MIDI" : "estimated from beat grid"}
        </span>
      {/if}
      {#if (hasBeatGrid || hasMidi) && songName}
        <button
          class="btn btn-sm btn-accent"
          onclick={guessTempoMap}
          disabled={guessing}
          >{guessing
            ? "Loading..."
            : guessSource
              ? "Re-detect"
              : hasMidi
                ? "Detect from MIDI"
                : "Guess from beat grid"}</button
        >
      {/if}
      <button class="btn btn-sm btn-danger" onclick={disableTempo}
        >{$t("common.remove")}</button
      >
      {#if onclose}
        <button class="btn btn-sm" onclick={onclose}
          >{$t("common.close")}</button
        >
      {/if}
    {:else}
      <button class="btn btn-sm btn-primary" onclick={enableTempo}
        >{$t("tempo.addTempo")}</button
      >
      {#if (hasBeatGrid || hasMidi) && songName}
        <button class="btn btn-sm btn-accent" onclick={guessTempoMap}
          >{hasMidi ? "Detect from MIDI" : "Guess from beat grid"}</button
        >
      {/if}
      {#if onclose}
        <button class="btn btn-sm" onclick={onclose}
          >{$t("common.close")}</button
        >
      {/if}
    {/if}
  </div>

  {#if tempo && expanded}
    <div class="tempo-body">
      <div class="tempo-fields">
        <label class="field">
          <span class="field-label">{$t("tempo.bpm")}</span>
          <input
            type="number"
            class="input"
            min="1"
            max="999"
            step="1"
            value={tempo.bpm}
            onchange={(e) => updateBpm((e.target as HTMLInputElement).value)}
          />
        </label>
        <label class="field">
          <span class="field-label">{$t("tempo.timeSignature")}</span>
          <div class="ts-row">
            <input
              type="number"
              class="input ts-input"
              min="1"
              max="32"
              value={tempo.time_signature[0]}
              onchange={(e) =>
                updateTimeSignature(
                  (e.target as HTMLInputElement).value,
                  tempo!.time_signature[1].toString(),
                )}
            />
            <span class="ts-sep">/</span>
            <input
              type="number"
              class="input ts-input"
              min="1"
              max="32"
              value={tempo.time_signature[1]}
              onchange={(e) =>
                updateTimeSignature(
                  tempo!.time_signature[0].toString(),
                  (e.target as HTMLInputElement).value,
                )}
            />
          </div>
        </label>
        <label class="field">
          <span class="field-label">{$t("tempo.startOffset")}</span>
          <div class="start-row">
            <input
              type="number"
              class="input"
              min="0"
              step="0.1"
              value={tempo.start.value}
              onchange={(e) =>
                updateStartValue((e.target as HTMLInputElement).value)}
            />
            <select
              class="input"
              value={tempo.start.unit}
              onchange={(e) =>
                updateStartUnit((e.target as HTMLSelectElement).value)}
            >
              <option value="s">s</option>
              <option value="ms">ms</option>
            </select>
          </div>
        </label>
      </div>

      <!-- Tempo changes -->
      <div class="changes-section">
        <div class="changes-header">
          <span class="section-label">{$t("tempo.tempoChanges")}</span>
          <button class="btn btn-sm" onclick={addChange}
            >{$t("tempo.addChange")}</button
          >
        </div>
        {#each tempo.changes as change, i (i)}
          <div class="change-row">
            <label class="field">
              <span class="field-label">{$t("tempo.at")}</span>
              <TimestampInput
                value={change.timestamp}
                onchange={(ts) => updateChange(i, { ...change, timestamp: ts })}
              />
            </label>
            <label class="field">
              <span class="field-label">{$t("tempo.bpm")}</span>
              <input
                type="number"
                class="input sm-input"
                min="1"
                max="999"
                value={change.bpm ?? ""}
                onchange={(e) => {
                  const v = (e.target as HTMLInputElement).value;
                  updateChange(i, {
                    ...change,
                    bpm: v ? parseFloat(v) : undefined,
                  });
                }}
              />
            </label>
            <label class="field">
              <span class="field-label">{$t("tempo.timeSig")}</span>
              <div class="ts-row">
                <input
                  type="number"
                  class="input ts-input"
                  min="1"
                  max="32"
                  value={change.time_signature?.[0] ?? ""}
                  onchange={(e) => {
                    const v = (e.target as HTMLInputElement).value;
                    if (v) {
                      updateChange(i, {
                        ...change,
                        time_signature: [
                          parseInt(v),
                          change.time_signature?.[1] ?? 4,
                        ],
                      });
                    } else {
                      updateChange(i, { ...change, time_signature: undefined });
                    }
                  }}
                />
                <span class="ts-sep">/</span>
                <input
                  type="number"
                  class="input ts-input"
                  min="1"
                  max="32"
                  value={change.time_signature?.[1] ?? ""}
                  onchange={(e) => {
                    const v = (e.target as HTMLInputElement).value;
                    if (v) {
                      updateChange(i, {
                        ...change,
                        time_signature: [
                          change.time_signature?.[0] ?? 4,
                          parseInt(v),
                        ],
                      });
                    } else {
                      updateChange(i, { ...change, time_signature: undefined });
                    }
                  }}
                />
              </div>
            </label>
            <label class="field">
              <span class="field-label">{$t("tempo.transition")}</span>
              <input
                type="text"
                class="input sm-input"
                placeholder="snap"
                value={change.transition ?? ""}
                onchange={(e) => {
                  const v = (e.target as HTMLInputElement).value;
                  updateChange(i, {
                    ...change,
                    transition: v || undefined,
                  });
                }}
              />
            </label>
            <button
              class="btn-icon delete-btn"
              title={$t("common.remove")}
              onclick={() => deleteChange(i)}
            >
              &#10005;
            </button>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .tempo-editor {
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }
  .tempo-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
  }
  .expand-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 11px;
    padding: 2px 4px;
    width: 20px;
  }
  .section-title {
    font-size: 14px;
    font-weight: 600;
  }
  .tempo-info {
    color: var(--text-muted);
    font-size: 13px;
    flex: 1;
  }
  .estimated-badge {
    font-size: 11px;
    color: var(--yellow, #eab308);
    background: rgba(234, 179, 8, 0.12);
    padding: 2px 8px;
    border-radius: 3px;
    font-style: italic;
  }
  .btn-accent {
    background: rgba(94, 202, 234, 0.15);
    color: var(--accent, #5ecaea);
    border: 1px solid rgba(94, 202, 234, 0.3);
  }
  .btn-accent:hover {
    background: rgba(94, 202, 234, 0.25);
  }
  .tempo-body {
    padding: 8px 12px 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .tempo-fields {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .field-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .field .input {
    font-size: 13px;
    padding: 4px 6px;
    width: 80px;
  }
  .sm-input {
    width: 70px !important;
  }
  .ts-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .ts-input {
    width: 40px !important;
    text-align: center;
  }
  .ts-sep {
    color: var(--text-muted);
    font-weight: 600;
  }
  .start-row {
    display: flex;
    gap: 4px;
  }
  .start-row .input:first-child {
    width: 60px;
  }
  .start-row .input:last-child {
    width: 50px;
  }
  .changes-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .changes-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .section-label {
    font-size: 12px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .change-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    flex-wrap: wrap;
    padding: 6px 8px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .delete-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    padding: 4px 6px;
    border-radius: 4px;
  }
  .delete-btn:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
</style>
