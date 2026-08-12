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
  import { beatsInMeasure, sigAtMeasure } from "../../lib/util/tempo";
  import type { BeatGrid } from "../../lib/util/beatGrid";
  import type { TempoConfig } from "../../lib/api/songs";
  import MarkerDialog from "./MarkerDialog.svelte";
  import PositionPicker, { type Position } from "./PositionPicker.svelte";
  import UnitToggle from "./UnitToggle.svelte";

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
    /** The song's measure count, bounding the pickers. */
    maxMeasure?: number;
    /** The song's tempo map, for the meter of each measure. */
    tempo?: TempoConfig | null;
    /** Beat times, so a boundary can also be read and typed as a time. */
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
    tempo = null,
    beatGrid = null,
    posToMs = null,
    playheadPos = null,
    onchange,
    ondelete,
    onclose,
  }: Props = $props();

  let autoColor = $derived(sectionColor(undefined, index));
  /** Both boundaries read out in the same unit — one toggle, not two. */
  let unit = $state<"beat" | "time">("beat");

  /** "13" for a measure-line boundary, "13.4" with a beat offset. */
  function posLabel(measure: number, beat?: number): string {
    return beat && beat !== 1 ? `${measure}.${beat}` : `${measure}`;
  }

  /** Position ordering by resolved time, the way the edge drags do it.
   * Comparing (measure, beat) tuples instead would call m1 beat 5 earlier
   * than m2 beat 1 when in 4/4 they are the same instant — the picker no
   * longer offers that, but a hand-written config still can. Falls back to
   * the tuples when there is nothing to resolve against. */
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

  /** Writes a boundary; beat 1 is the measure line, stored as "unset".
   * A move that would invert the section is refused — the picker is
   * controlled, so it snaps back to the position still in the config. */
  function setBoundary(field: "start" | "end", pos: Position) {
    const other = field === "start" ? endPos : startPos;
    const ordered =
      field === "start" ? isBefore(pos, other) : isBefore(other, pos);
    if (!ordered) return;
    onchange({
      [`${field}_measure`]: pos.measure,
      [`${field}_beat`]: pos.beat === 1 ? undefined : pos.beat,
    });
  }

  function captureBoundary(field: "start" | "end") {
    if (!playheadPos) return;
    setBoundary(field, playheadPos);
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
    <div class="position-head">
      <span class="section-label">{$t("tempo.marker.position")}</span>
      {#if beatGrid}
        <UnitToggle {unit} onchange={(u) => (unit = u)} />
      {/if}
    </div>
    <div class="labeled-picker">
      <span class="mini-label">{$t("sections.dialog.startPos")}</span>
      <PositionPicker
        label={$t("sections.dialog.startPos")}
        value={{ kind: "beat", ...startPos }}
        step={0.5}
        stores="beat"
        bind:unit
        showToggle={false}
        {maxMeasure}
        {beatGrid}
        beatsIn={(m) => beatsInMeasure(tempo, m)}
        sigOf={(m) => sigAtMeasure(tempo, m).join("/")}
        ghost={playheadPos}
        onchange={(v) => v.kind === "beat" && setBoundary("start", v)}
      />
    </div>
    <div class="labeled-picker">
      <span class="mini-label">{$t("sections.dialog.endPos")}</span>
      <PositionPicker
        label={$t("sections.dialog.endPos")}
        value={{ kind: "beat", ...endPos }}
        step={0.5}
        stores="beat"
        bind:unit
        showToggle={false}
        {maxMeasure}
        {beatGrid}
        beatsIn={(m) => beatsInMeasure(tempo, m)}
        sigOf={(m) => sigAtMeasure(tempo, m).join("/")}
        ghost={playheadPos}
        onchange={(v) => v.kind === "beat" && setBoundary("end", v)}
      />
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
  .position-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .labeled-picker {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .labeled-picker + .labeled-picker {
    margin-top: 10px;
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
