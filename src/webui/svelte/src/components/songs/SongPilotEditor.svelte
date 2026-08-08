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
  import type { PilotConfig, PilotHintConfig } from "../../lib/api/songs";

  interface Props {
    /** The song.yaml `pilot:` block, or null when not configured. */
    pilot: PilotConfig | null;
    onchange: (pilot: PilotConfig | null) => void;
    /** Whether the song has a beat grid (for measure-based positions). */
    hasBeatGrid?: boolean;
    /** The song's beat grid (beat times in seconds + measure start indices),
     * used to convert positions when switching between measure and time. */
    beatGrid?: { beats: number[]; measure_starts: number[] } | null;
  }

  let {
    pilot,
    onchange,
    hasBeatGrid = false,
    beatGrid = null,
  }: Props = $props();

  let expanded = $state(true);

  function enable() {
    onchange({ hints: [] });
  }

  function disable() {
    onchange(null);
  }

  function emit(hints: PilotHintConfig[], track?: string) {
    const updated: PilotConfig = { ...(pilot ?? {}), hints };
    if (track !== undefined) {
      updated.track = track;
    }
    if (updated.track === "pilot" || updated.track === "") {
      delete updated.track;
    }
    onchange(updated);
  }

  function addHint() {
    const hints = [...(pilot?.hints ?? [])];
    hints.push({
      at: hasBeatGrid ? { measure: 1 } : { time: 0 },
      label: "",
    });
    emit(hints);
  }

  function updateHint(index: number, patch: Partial<PilotHintConfig>) {
    const hints = [...(pilot?.hints ?? [])];
    const merged = { ...hints[index], ...patch };
    if (merged.align === "end") {
      delete merged.align;
    }
    if (merged.offset === 0 || merged.offset === undefined) {
      delete merged.offset;
    }
    if (!merged.file) {
      delete merged.file;
    }
    hints[index] = merged;
    emit(hints);
  }

  function deleteHint(index: number) {
    const hints = (pilot?.hints ?? []).filter((_, i) => i !== index);
    emit(hints);
  }

  function positionKind(hint: PilotHintConfig): "measure" | "time" {
    return "measure" in hint.at ? "measure" : "time";
  }

  /** Time (seconds) of measure/beat on the grid, or null when off-grid. */
  function measureBeatToTime(measure: number, beat: number): number | null {
    if (!beatGrid) return null;
    const startIdx = beatGrid.measure_starts[measure - 1];
    if (startIdx === undefined) return null;
    const time = beatGrid.beats[startIdx + (beat - 1)];
    return time === undefined ? null : time;
  }

  /** Nearest measure/beat for a time (seconds), or null without a grid. */
  function timeToMeasureBeat(
    time: number,
  ): { measure: number; beat?: number } | null {
    if (!beatGrid || beatGrid.beats.length === 0) return null;
    let nearest = 0;
    for (let i = 1; i < beatGrid.beats.length; i++) {
      if (
        Math.abs(beatGrid.beats[i] - time) <
        Math.abs(beatGrid.beats[nearest] - time)
      ) {
        nearest = i;
      }
    }
    let measure = 0;
    while (
      measure + 1 < beatGrid.measure_starts.length &&
      beatGrid.measure_starts[measure + 1] <= nearest
    ) {
      measure++;
    }
    const beat = nearest - beatGrid.measure_starts[measure] + 1;
    const at: { measure: number; beat?: number } = { measure: measure + 1 };
    if (beat > 1) at.beat = beat;
    return at;
  }

  function switchPosition(index: number, kind: "measure" | "time") {
    const hint = (pilot?.hints ?? [])[index];
    if (!hint || positionKind(hint) === kind) return;
    // Convert the current position instead of resetting it, so switching
    // representation keeps the hint where it is.
    if (kind === "time") {
      const at = hint.at as { measure: number; beat?: number };
      const time = measureBeatToTime(at.measure, at.beat ?? 1);
      updateHint(index, {
        at: { time: time !== null ? Math.round(time * 1000) / 1000 : 0 },
      });
    } else {
      const at = hint.at as { time: number };
      updateHint(index, {
        at: timeToMeasureBeat(at.time) ?? { measure: 1 },
      });
    }
  }
</script>

<div class="pilot-editor">
  <div class="pilot-header">
    <button class="expand-btn" onclick={() => (expanded = !expanded)}>
      {expanded ? "▼" : "▶"}
    </button>
    <span class="section-title">{$t("pilot.title")}</span>
    {#if pilot}
      <span class="pilot-info">
        {pilot.hints?.length ?? 0}
        {$t("pilot.hintCount")}
      </span>
      <button class="btn btn-sm" onclick={addHint}>{$t("pilot.addHint")}</button
      >
      <button class="btn btn-sm btn-danger" onclick={disable}
        >{$t("common.remove")}</button
      >
    {:else}
      <button class="btn btn-sm btn-primary" onclick={enable}
        >{$t("pilot.add")}</button
      >
    {/if}
  </div>

  {#if pilot && expanded}
    <div class="pilot-body">
      {#if (pilot.hints ?? []).length === 0}
        <p class="muted hint-text">{$t("pilot.empty")}</p>
      {/if}
      {#each pilot.hints ?? [] as hint, i (i)}
        <div class="hint-row">
          <select
            class="input kind-select"
            value={positionKind(hint)}
            onchange={(e) =>
              switchPosition(
                i,
                (e.target as HTMLSelectElement).value as "measure" | "time",
              )}
          >
            <option value="measure">{$t("pilot.atMeasure")}</option>
            <option value="time">{$t("pilot.atTime")}</option>
          </select>
          {#if "measure" in hint.at}
            {@const at = hint.at}
            <input
              type="number"
              class="input pos-input"
              min="1"
              title={$t("pilot.measure")}
              value={at.measure}
              onchange={(e) =>
                updateHint(i, {
                  at: {
                    ...at,
                    measure:
                      parseInt((e.target as HTMLInputElement).value) || 1,
                  },
                })}
            />
            <input
              type="number"
              class="input pos-input"
              min="1"
              placeholder="1"
              title={$t("pilot.beat")}
              value={at.beat ?? ""}
              onchange={(e) => {
                const v = (e.target as HTMLInputElement).value;
                const beat = v ? parseInt(v) : undefined;
                const nextAt: { measure: number; beat?: number } = {
                  measure: at.measure,
                };
                if (beat && beat > 1) nextAt.beat = beat;
                updateHint(i, { at: nextAt });
              }}
            />
          {:else}
            <input
              type="number"
              class="input pos-input pos-input--time"
              min="0"
              step="0.1"
              title={$t("pilot.timeSeconds")}
              value={hint.at.time}
              onchange={(e) =>
                updateHint(i, {
                  at: {
                    time: parseFloat((e.target as HTMLInputElement).value) || 0,
                  },
                })}
            />
          {/if}
          <input
            type="text"
            class="input label-input"
            placeholder={$t("pilot.labelPlaceholder")}
            value={hint.label}
            onchange={(e) =>
              updateHint(i, {
                label: (e.target as HTMLInputElement).value,
              })}
          />
          <input
            type="text"
            class="input file-input"
            placeholder={$t("pilot.filePlaceholder")}
            value={hint.file ?? ""}
            onchange={(e) => {
              const v = (e.target as HTMLInputElement).value.trim();
              updateHint(i, { file: v || undefined });
            }}
          />
          <select
            class="input align-select"
            title={$t("pilot.align")}
            value={hint.align ?? "end"}
            onchange={(e) =>
              updateHint(i, {
                align: (e.target as HTMLSelectElement).value as "end" | "start",
              })}
          >
            <option value="end">{$t("pilot.alignEnd")}</option>
            <option value="start">{$t("pilot.alignStart")}</option>
          </select>
          <button
            class="btn-icon delete-btn"
            title={$t("common.remove")}
            onclick={() => deleteHint(i)}
          >
            ✕
          </button>
        </div>
      {/each}
      <p class="muted hint-text">{$t("pilot.hint")}</p>
    </div>
  {/if}
</div>

<style>
  .pilot-editor {
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
  }
  .pilot-header {
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
    padding: 2px;
  }
  .section-title {
    font-weight: 600;
    font-size: 13px;
  }
  .pilot-info {
    color: var(--text-muted);
    font-size: 12px;
    flex: 1;
  }
  .pilot-body {
    padding: 0 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .hint-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .kind-select {
    width: 100px;
  }
  .pos-input {
    width: 64px;
  }
  .pos-input--time {
    width: 90px;
  }
  .label-input {
    flex: 2;
    min-width: 140px;
  }
  .file-input {
    flex: 1;
    min-width: 130px;
  }
  .align-select {
    width: 130px;
  }
  .delete-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .delete-btn:hover {
    color: var(--error, #e84b4b);
  }
  .hint-text {
    font-size: 12px;
    margin: 0;
  }
</style>
