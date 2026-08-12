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
  import PositionPicker, { type PositionValue } from "./PositionPicker.svelte";
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

  /** The picker's value, mirroring however the hint is anchored. Conversion
   * between the two anchorings is the picker's own beat/time toggle. */
  let pickerValue = $derived.by((): PositionValue => {
    if ("measure" in hint.at) {
      return {
        kind: "beat",
        measure: hint.at.measure,
        beat: hint.at.beat ?? 1,
      };
    }
    return { kind: "time", time: hint.at.time };
  });

  /** Writes the anchoring the picker reports; beat 1 stays implicit and a
   * time keeps millisecond precision. */
  function writePosition(value: PositionValue) {
    if (value.kind === "time") {
      onchange({ at: { time: Math.round(value.time * 1000) / 1000 } });
      return;
    }
    const at: { measure: number; beat?: number } = { measure: value.measure };
    if (value.beat > 1) at.beat = value.beat;
    onchange({ at });
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
      <PositionPicker
        label={$t("position.hint")}
        value={pickerValue}
        stores="either"
        {maxMeasure}
        {beatGrid}
        beatsIn={(m) => beatsInMeasure(tempo, m)}
        sigOf={(m) => sigAtMeasure(tempo, m).join("/")}
        onchange={writePosition}
      />
    {:else}
      <!-- No grid, no beats to pick: seconds are all there is. -->
      <div class="stepper-row">
        <NumberStepper
          value={"time" in hint.at ? hint.at.time : 0}
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
