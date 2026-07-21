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
  import { SECTION_COLORS, sectionColor } from "../../lib/sectionColors";
  import MarkerDialog from "./MarkerDialog.svelte";
  import NumberStepper from "../NumberStepper.svelte";

  interface SectionEntry {
    name: string;
    start_measure: number;
    end_measure: number;
    /** 1-based beat within the measure, fractional allowed; absent = 1. */
    start_beat?: number;
    end_beat?: number;
    color?: string;
  }

  interface Props {
    section: SectionEntry;
    /** The section's index, for the automatic palette color. */
    index: number;
    /** The song's measure count, bounding the steppers. */
    maxMeasure?: number;
    /** The preview playhead as a boundary position (half-beat snapped),
     * when this song is loaded in the player. Enables "set here". */
    playheadPos?: { measure: number; beat: number } | null;
    onchange: (patch: Partial<SectionEntry>) => void;
    ondelete: () => void;
    onclose: () => void;
  }

  let {
    section,
    index,
    maxMeasure = 9999,
    playheadPos = null,
    onchange,
    ondelete,
    onclose,
  }: Props = $props();

  let autoColor = $derived(sectionColor(undefined, index));
  let length = $derived(section.end_measure - section.start_measure);

  /** "13" for a measure-line boundary, "13.4" with a beat offset. */
  function posLabel(measure: number, beat?: number): string {
    return beat && beat !== 1 ? `${measure}.${beat}` : `${measure}`;
  }

  /** Beat 1 is the measure line — store that as "unset". */
  function setBeat(field: "start_beat" | "end_beat", beat: number) {
    onchange({ [field]: beat === 1 ? undefined : beat });
  }

  /** Position ordering as (measure, beat) tuples. */
  function isBefore(
    a: { measure: number; beat: number },
    b: { measure: number; beat: number },
  ): boolean {
    return (
      a.measure < b.measure || (a.measure === b.measure && a.beat < b.beat)
    );
  }

  let startPos = $derived({
    measure: section.start_measure,
    beat: section.start_beat ?? 1,
  });
  let endPos = $derived({
    measure: section.end_measure,
    beat: section.end_beat ?? 1,
  });
  /** The capture buttons stay disabled when they would invert the section. */
  let canCaptureStart = $derived(
    playheadPos !== null && isBefore(playheadPos, endPos),
  );
  let canCaptureEnd = $derived(
    playheadPos !== null && isBefore(startPos, playheadPos),
  );

  function captureBoundary(field: "start" | "end") {
    if (!playheadPos) return;
    onchange({
      [`${field}_measure`]: playheadPos.measure,
      [`${field}_beat`]: playheadPos.beat === 1 ? undefined : playheadPos.beat,
    });
  }

  /** Moving the start slides the whole section, like a body drag. */
  function moveStart(start: number) {
    const clamped = Math.max(1, Math.min(start, maxMeasure - length + 1));
    onchange({
      start_measure: clamped,
      end_measure: clamped + length,
    });
  }

  function setLength(measures: number) {
    const clamped = Math.max(
      1,
      Math.min(measures, maxMeasure + 1 - section.start_measure),
    );
    onchange({ end_measure: section.start_measure + clamped });
  }
</script>

<MarkerDialog title={$t("sections.dialog.title")} {onclose}>
  <div class="dialog-section">
    <span class="section-label">
      {$t("sections.dialog.name")}
      <span class="range-note"
        >m{posLabel(section.start_measure, section.start_beat)}–{posLabel(
          section.end_measure,
          section.end_beat,
        )}</span
      >
    </span>
    <input
      type="text"
      class="input name-input"
      value={section.name}
      onchange={(e) =>
        onchange({ name: (e.target as HTMLInputElement).value.trim() })}
    />
  </div>

  <div class="dialog-section">
    <span class="section-label">{$t("tempo.marker.position")}</span>
    <div class="stepper-row">
      <div class="labeled-stepper">
        <span class="mini-label">{$t("sections.dialog.start")}</span>
        <NumberStepper
          value={section.start_measure}
          min={1}
          max={Math.max(1, maxMeasure - length + 1)}
          ariaLabel={$t("sections.dialog.start")}
          onchange={moveStart}
        />
      </div>
      <div class="labeled-stepper">
        <span class="mini-label">{$t("sections.dialog.length")}</span>
        <NumberStepper
          value={length}
          min={1}
          max={Math.max(1, maxMeasure + 1 - section.start_measure)}
          ariaLabel={$t("sections.dialog.length")}
          onchange={setLength}
        />
      </div>
    </div>
    <div class="stepper-row">
      <div class="labeled-stepper">
        <span class="mini-label">{$t("sections.dialog.startBeat")}</span>
        <NumberStepper
          value={section.start_beat ?? 1}
          min={1}
          max={32}
          step={0.5}
          decimals={1}
          ariaLabel={$t("sections.dialog.startBeat")}
          onchange={(v) => setBeat("start_beat", v)}
        />
      </div>
      <div class="labeled-stepper">
        <span class="mini-label">{$t("sections.dialog.endBeat")}</span>
        <NumberStepper
          value={section.end_beat ?? 1}
          min={1}
          max={32}
          step={0.5}
          decimals={1}
          ariaLabel={$t("sections.dialog.endBeat")}
          onchange={(v) => setBeat("end_beat", v)}
        />
      </div>
    </div>
    <span class="beat-note">{$t("sections.dialog.beatNote")}</span>
    {#if playheadPos}
      <div class="stepper-row playhead-capture">
        <span class="mini-label"
          >{$t("sections.dialog.playheadAt", {
            values: {
              pos: `m${posLabel(playheadPos.measure, playheadPos.beat)}`,
            },
          })}</span
        >
        <button
          type="button"
          class="btn btn-sm"
          disabled={!canCaptureStart}
          onclick={() => captureBoundary("start")}
          >{$t("sections.dialog.setStartHere")}</button
        >
        <button
          type="button"
          class="btn btn-sm"
          disabled={!canCaptureEnd}
          onclick={() => captureBoundary("end")}
          >{$t("sections.dialog.setEndHere")}</button
        >
      </div>
    {/if}
  </div>

  <div class="dialog-section">
    <span class="section-label">{$t("sections.dialog.color")}</span>
    <div class="swatches">
      <button
        type="button"
        class="swatch swatch--auto"
        class:active={!section.color}
        style:--swatch-color={autoColor}
        title={$t("sections.dialog.colorAuto")}
        onclick={() => onchange({ color: undefined })}
      >
        A
      </button>
      {#each SECTION_COLORS as color (color)}
        <button
          type="button"
          class="swatch"
          class:active={section.color === color}
          style:--swatch-color={color}
          aria-label={color}
          onclick={() => onchange({ color })}
        ></button>
      {/each}
    </div>
  </div>

  {#snippet actions()}
    <button type="button" class="btn btn-danger" onclick={ondelete}
      >{$t("sections.dialog.delete")}</button
    >
  {/snippet}
</MarkerDialog>

<style>
  .dialog-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .section-label {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.6px;
    font-weight: 600;
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .range-note {
    font-family: var(--mono);
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-dim);
  }

  .beat-note {
    display: block;
    margin-top: 6px;
    font-size: 11px;
    color: var(--text-dim);
  }

  .playhead-capture {
    margin-top: 10px;
    align-items: center;
  }

  .playhead-capture .mini-label {
    font-family: var(--mono);
  }
  .name-input {
    font-size: 16px;
    min-height: 44px;
  }
  .stepper-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .labeled-stepper {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .mini-label {
    font-size: 10px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .swatches {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  .swatch {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    border: 2px solid transparent;
    background: color-mix(in srgb, var(--swatch-color) 45%, transparent);
    box-shadow: inset 0 0 0 1.5px var(--swatch-color);
    cursor: pointer;
    touch-action: manipulation;
    transition: transform 0.06s;
  }
  .swatch:active {
    transform: scale(0.92);
  }
  .swatch.active {
    border-color: var(--text);
    background: var(--swatch-color);
  }
  .swatch--auto {
    color: var(--text);
    font-size: 13px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
