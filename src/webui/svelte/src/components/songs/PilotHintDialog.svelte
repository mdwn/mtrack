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
    PilotHintConfig,
    SongFile,
    TempoConfig,
  } from "../../lib/api/songs";
  import { beatsInMeasure, sigAtMeasure } from "../../lib/util/tempo";
  import NumberStepper from "../NumberStepper.svelte";
  import MarkerDialog from "./MarkerDialog.svelte";
  import PositionPicker from "./PositionPicker.svelte";
  import SongFileField from "./SongFileField.svelte";

  interface Props {
    hint: PilotHintConfig;
    hasBeatGrid?: boolean;
    /** Beat times (seconds) + measure start indices, for converting the
     * position when switching between measure and time anchoring. */
    beatGrid?: { beats: number[]; measure_starts: number[] } | null;
    /** The song's tempo map, for the meter of each measure. */
    tempo?: TempoConfig | null;
    /** The song's measure count, bounding the position picker. */
    maxMeasure?: number;
    songName?: string;
    /** The song directory listing, for the clip picker. */
    songFiles?: SongFile[];
    onchange: (patch: Partial<PilotHintConfig>) => void;
    ondelete: () => void;
    onclose: () => void;
  }

  let {
    hint,
    hasBeatGrid = false,
    beatGrid = null,
    tempo = null,
    maxMeasure = 9999,
    songName = "",
    songFiles = [],
    onchange,
    ondelete,
    onclose,
  }: Props = $props();

  let kind = $derived("measure" in hint.at ? "measure" : "time");

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

  function switchKind(next: "measure" | "time") {
    if (kind === next) return;
    // Convert the position instead of resetting it, so switching
    // representation keeps the hint where it is.
    if (next === "time") {
      const at = hint.at as { measure: number; beat?: number };
      const time = measureBeatToTime(at.measure, at.beat ?? 1);
      onchange({
        at: { time: time !== null ? Math.round(time * 1000) / 1000 : 0 },
      });
    } else {
      const at = hint.at as { time: number };
      onchange({ at: timeToMeasureBeat(at.time) ?? { measure: 1 } });
    }
  }
</script>

<MarkerDialog title={$t("pilot.marker.title")} {onclose}>
  <div class="field-block">
    <span class="field-label">{$t("pilot.labelPlaceholder")}</span>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="text"
      class="input label-input"
      autofocus={!hint.label}
      placeholder={$t("pilot.labelPlaceholder")}
      value={hint.label}
      onchange={(e) =>
        onchange({ label: (e.target as HTMLInputElement).value })}
    />
  </div>

  <div class="field-block">
    <span class="field-label">{$t("tempo.marker.position")}</span>
    {#if hasBeatGrid}
      <div class="segmented">
        <button
          type="button"
          class="seg-btn"
          class:active={kind === "measure"}
          onclick={() => switchKind("measure")}>{$t("pilot.atMeasure")}</button
        >
        <button
          type="button"
          class="seg-btn"
          class:active={kind === "time"}
          onclick={() => switchKind("time")}>{$t("pilot.atTime")}</button
        >
      </div>
    {/if}
    {#if "measure" in hint.at}
      {@const at = hint.at}
      <PositionPicker
        label={$t("position.hint")}
        measure={at.measure}
        beat={at.beat ?? 1}
        {maxMeasure}
        beatsIn={(m) => beatsInMeasure(tempo, m)}
        sigOf={(m) => sigAtMeasure(tempo, m).join("/")}
        onchange={(pos) => {
          const next: { measure: number; beat?: number } = {
            measure: pos.measure,
          };
          if (pos.beat > 1) next.beat = pos.beat;
          onchange({ at: next });
        }}
      />
    {:else}
      <div class="stepper-row">
        <NumberStepper
          value={hint.at.time}
          min={0}
          max={36000}
          step={0.1}
          decimals={1}
          suffix="s"
          ariaLabel={$t("pilot.timeSeconds")}
          onchange={(v) => onchange({ at: { time: v } })}
        />
      </div>
    {/if}
  </div>

  <div class="field-block">
    <span class="field-label">{$t("pilot.marker.audio")}</span>
    <SongFileField
      value={hint.file ?? ""}
      placeholder={$t("pilot.filePlaceholder")}
      {songName}
      files={songFiles}
      onchange={(path) => onchange({ file: path })}
    />
    {#if hint.file}
      <div class="segmented">
        <button
          type="button"
          class="seg-btn"
          class:active={(hint.align ?? "end") === "end"}
          onclick={() => onchange({ align: "end" })}
          >{$t("pilot.alignEnd")}</button
        >
        <button
          type="button"
          class="seg-btn"
          class:active={hint.align === "start"}
          onclick={() => onchange({ align: "start" })}
          >{$t("pilot.alignStart")}</button
        >
      </div>
    {:else}
      <span class="hint-note">{$t("pilot.marker.labelOnly")}</span>
    {/if}
  </div>

  {#snippet actions()}
    <button type="button" class="btn btn-danger" onclick={ondelete}
      >{$t("pilot.marker.deleteHint")}</button
    >
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
  .stepper-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .label-input {
    font-size: 16px;
    min-height: 44px;
  }
  .hint-note {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
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
