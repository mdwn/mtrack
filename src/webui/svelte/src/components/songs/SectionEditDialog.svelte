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
  import { maxBeatInMeasure, type BeatGrid } from "../../lib/util/beatGrid";

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
    /** Beat times, so the beat steppers can stop at the end of their
     * measure and the two boundaries can be ordered by resolved time. */
    beatGrid?: BeatGrid | null;
    /** Resolves a boundary to milliseconds — the same function the edge
     * drags order positions with. */
    posToMs?: ((measure: number, beat: number) => number) | null;
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
    beatGrid = null,
    posToMs = null,
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

  /** Beat 1 is the measure line — store that as "unset".
   *
   * A value that would invert the section is clamped to the last one that does
   * not, rather than refused: the steppers are controlled, so a silent refusal
   * reads as a broken button. An inverted range is what `Section::validate`
   * rejects on save, which is the outcome being avoided. */
  function setBeat(field: "start_beat" | "end_beat", beat: number) {
    const measure =
      section[field === "start_beat" ? "start_measure" : "end_measure"];
    const ordered = (b: number) =>
      field === "start_beat"
        ? isBefore({ measure, beat: b }, endPos)
        : isBefore(startPos, { measure, beat: b });

    // Walk back by whole steps rather than computing the nearest legal value:
    // the ordering test resolves through the grid, where beat 5 of a 4/4
    // measure and beat 1 of the next are the same instant, so there is no
    // arithmetic shortcut.
    let candidate = beat;
    while (candidate > 1 && !ordered(candidate)) {
      candidate = Math.max(1, candidate - BEAT_STEP);
    }
    // Beat 1 is the measure line, which orders correctly against any
    // well-formed section, so the walk terminates there at worst.
    if (!ordered(candidate)) return;
    onchange({ [field]: candidate === 1 ? undefined : candidate });
  }

  /** How far the beat steppers go: the end of their own measure. Without a
   * grid there is nothing to measure against, so they stay open. */
  const BEAT_STEP = 0.5;
  let maxStartBeat = $derived(
    maxBeatInMeasure(beatGrid, section.start_measure, BEAT_STEP) ?? 32,
  );
  let maxEndBeat = $derived(
    maxBeatInMeasure(beatGrid, section.end_measure, BEAT_STEP) ?? 32,
  );

  /** Position ordering by resolved time, the way the edge drags do it.
   * Comparing (measure, beat) tuples instead would call m1 beat 5 earlier
   * than m2 beat 1 when in 4/4 they are the same instant — the steppers no
   * longer offer that, but a hand-written config still can. Falls back to
   * the tuples when there is no grid to resolve against. */
  function isBefore(
    a: { measure: number; beat: number },
    b: { measure: number; beat: number },
  ): boolean {
    if (posToMs) {
      return posToMs(a.measure, a.beat) < posToMs(b.measure, b.beat);
    }
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

  /** Clamps a beat to what the destination measure actually holds. Carrying
   * beat 4.5 into a 3/4 measure names a beat that does not exist. */
  function clampBeat(
    measure: number,
    beat: number | undefined,
  ): number | undefined {
    if (beat === undefined || beat === 1) return undefined;
    const max = maxBeatInMeasure(beatGrid, measure, BEAT_STEP);
    if (max === null) return beat;
    const clamped = Math.min(beat, max);
    return clamped === 1 ? undefined : clamped;
  }

  /** Moving the start slides the whole section, like a body drag. */
  function moveStart(start: number) {
    const clamped = Math.max(1, Math.min(start, maxMeasure - length + 1));
    onchange({
      start_measure: clamped,
      end_measure: clamped + length,
      start_beat: clampBeat(clamped, section.start_beat),
      end_beat: clampBeat(clamped + length, section.end_beat),
    });
  }

  function setLength(measures: number) {
    const clamped = Math.max(
      1,
      Math.min(measures, maxMeasure + 1 - section.start_measure),
    );
    const end = section.start_measure + clamped;
    onchange({
      end_measure: end,
      end_beat: clampBeat(end, section.end_beat),
    });
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
          max={maxStartBeat}
          step={BEAT_STEP}
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
          max={maxEndBeat}
          step={BEAT_STEP}
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
