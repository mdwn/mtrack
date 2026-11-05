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
    Cue,
    Layer,
    Timestamp,
    SubLaneType,
  } from "../../../lib/lighting/types";
  import { emptyCue, absoluteTimestamp } from "../../../lib/lighting/types";
  import type {
    CueLayout,
    GridLine,
    TempoSource,
  } from "../../../gen/rules/mtrack_rules";
  import {
    msToPixel,
    timestampToMs,
    msToTimestamp,
    getGridLines,
    snapToNearestGrid,
    type SnapResolution,
  } from "../../../lib/lighting/timeline-state";
  import CueBlock, { type Block, type BlockEffect } from "./CueBlock.svelte";
  import { colorsOf } from "../../../lib/lighting/params";

  interface Props {
    name: string;
    cues: Cue[];
    /** Where the player fires each of `cues`, from its layout. */
    laidCues?: CueLayout[];
    /** The bar and beat lines the lane is drawn under. */
    grid?: GridLine[];
    /** The tempo map the lane snaps and places against. */
    tempoSource: TempoSource | null;
    laneType: "show" | "sequence";
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    selectedCueIndex: number | null;
    snapEnabled: boolean;
    snapResolution: SnapResolution;
    onselect: (index: number) => void;
    oncuechange: (index: number, cue: Cue) => void;
    oncuedelete: (index: number) => void;
    oncueadd: (cue: Cue) => void;
    ondelete: () => void;
    oneffectresize?: (cueIndex: number, newDurationStr: string) => void;
    onloopchange?: (cueIndex: number, newLoopCount: number) => void;
    onsequenceedit?: (sequenceName: string) => void;
    subLaneType?: SubLaneType;
    laneHeight?: number;
    hideLabel?: boolean;
    playheadMs?: number | null;
  }

  let {
    name,
    cues,
    laidCues = [],
    grid = [],
    tempoSource,
    laneType,
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    selectedCueIndex,
    snapEnabled,
    snapResolution,
    onselect,
    oncuechange,
    oncuedelete,
    oncueadd,
    ondelete,
    subLaneType,
    laneHeight,
    hideLabel = false,
    playheadMs = null,
    oneffectresize,
    onloopchange,
    onsequenceedit,
  }: Props = $props();

  let canvasEl: HTMLCanvasElement | undefined = $state();
  let laneEl: HTMLDivElement | undefined = $state();

  /** A block shown 500 ms wide when nothing says how long it is. */
  const DEFAULT_BLOCK_MS = 500;

  /** The layer this lane shows, for an effects lane. */
  let laneLayer = $derived(
    subLaneType?.startsWith("effects:") ? subLaneType.split(":")[1] : undefined,
  );

  /** The blocks the lane draws, from the player's layout of each cue: on
   *  an effects lane, one for the cue's own effects on the lane's layer
   *  (as long as the longest runs) and one for each pass of a sequence
   *  it references or a loop it holds; on the commands and sequences
   *  lanes, one per cue, a sequence block as long as its passes. */
  function getBlocks(): Block[] {
    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + viewportWidth) / pixelsPerMs;
    const margin = 100 / pixelsPerMs;
    const blocks: Block[] = [];
    const push = (block: Omit<Block, "px" | "visible">) => {
      blocks.push({
        ...block,
        px: msToPixel(block.ms, pixelsPerMs) - scrollLeft,
        visible:
          block.endMs >= viewStartMs - margin && block.ms <= viewEndMs + margin,
      });
    };
    cues.forEach((cue, index) => {
      const laid = laidCues[index];
      const ms = laid?.ms ?? timestampToMs(cue.timestamp, tempoSource);
      const scoreMs = laid?.score_ms ?? ms;
      if (subLaneType === "commands") {
        if (cue.commands.length === 0) return;
        push({ index, ms, endMs: ms + DEFAULT_BLOCK_MS, scoreMs, effects: [] });
        return;
      }
      if (subLaneType === "sequences") {
        if (cue.sequences.length === 0) return;
        const endMs = Math.max(
          ms + DEFAULT_BLOCK_MS,
          ...(laid?.passes ?? []).map((p) => p.end_ms),
        );
        push({ index, ms, endMs, scoreMs, effects: [] });
        return;
      }
      // An effects lane (one layer, or every layer in the sequence editor):
      // the cue's own effects as the layout places them (their layer by the
      // player's default), shown with the document's colours.
      const ownLaid = (laid?.effects ?? []).filter((e) => e.own);
      const own: BlockEffect[] = [];
      let ownEnd = ms + DEFAULT_BLOCK_MS;
      ownLaid.forEach((e, i) => {
        if (laneLayer && e.layer !== laneLayer) return;
        const written = cue.effects[i];
        own.push({
          type: e.kind,
          groups: e.groups,
          colors: written ? colorsOf(written.effect) : [],
        });
        ownEnd = Math.max(ownEnd, e.end_ms);
      });
      if (own.length > 0 || (!laneLayer && cue.commands.length > 0)) {
        push({ index, ms, endMs: ownEnd, scoreMs, effects: own });
      }
      // What the cue's references and loops put on the timeline, a block
      // per pass.
      const generated: Record<string, Block> = {};
      for (const e of laid?.effects ?? []) {
        if (e.own) continue;
        if (laneLayer && e.layer !== laneLayer) continue;
        const key = `${e.sequence ?? ""}@${e.start_ms}`;
        const block = generated[key];
        if (block) {
          block.endMs = Math.max(block.endMs, e.end_ms);
          block.effects.push({ type: e.kind, groups: e.groups, colors: [] });
        } else {
          generated[key] = {
            index,
            ms: e.start_ms,
            endMs: e.end_ms,
            scoreMs,
            effects: [{ type: e.kind, groups: e.groups, colors: [] }],
            fromSequence: e.sequence ?? "",
            px: 0,
            visible: false,
          };
        }
      }
      for (const block of Object.values(generated)) push(block);
    });
    return blocks;
  }

  // The offset in force at a drawn position: that of the last cue drawn
  // at or before it, its own offset (or reset) included.
  function getCumulativeOffsetAtMs(adjustedMs: number): number {
    if (laneType !== "show") return 0;
    for (let i = laidCues.length - 1; i >= 0; i--) {
      if (laidCues[i].ms <= adjustedMs) return laidCues[i].next_offset_ms;
    }
    return 0;
  }

  function handleDblClick(e: MouseEvent) {
    if (!laneEl) return;
    const rect = laneEl.getBoundingClientRect();
    const x = e.clientX - rect.left + scrollLeft;
    let adjustedMs = x / pixelsPerMs;
    adjustedMs = Math.max(0, adjustedMs);

    // Subtract cumulative offset to get the raw timestamp ms: the score
    // time, on the tempo map the offset moves as a whole, so the new cue
    // lands where it was clicked.
    const offsetMs = getCumulativeOffsetAtMs(adjustedMs);
    let rawMs = adjustedMs - offsetMs;
    rawMs = Math.max(0, rawMs);

    if (snapEnabled && tempoSource) {
      rawMs = snapToNearestGrid(rawMs, tempoSource, snapResolution);
    }

    const ts: Timestamp = tempoSource
      ? msToTimestamp(rawMs, tempoSource)
      : absoluteTimestamp(rawMs);

    const newCue = emptyCue(ts);
    if (subLaneType === "effects" || subLaneType?.startsWith("effects:")) {
      const layer = subLaneType?.startsWith("effects:")
        ? (subLaneType.split(":")[1] as Layer)
        : undefined;
      const defaultDuration = tempoSource ? "1measure" : "5s";
      newCue.effects = [
        {
          groups: ["all"],
          effect: {
            type: "static",
            params: [
              { name: "duration", value: defaultDuration },
              ...(layer ? [{ name: "layer", value: layer }] : []),
            ],
          },
        },
      ];
    } else if (subLaneType === "commands") {
      newCue.commands = [{ command: "clear" }];
    } else if (subLaneType === "sequences") {
      newCue.sequences = [{ name: "" }];
    }
    oncueadd(newCue);
  }

  function handleCueMove(index: number, deltaMs: number) {
    const cue = cues[index];
    // Move operates on score times (offset is structural, not per-cue
    // position; the tempo map moves with it as a whole), so the cue moves
    // by what was dragged.
    const currentMs =
      laidCues[index]?.score_ms ?? timestampToMs(cue.timestamp, tempoSource);
    let newMs = Math.max(0, currentMs + deltaMs);

    if (snapEnabled && tempoSource) {
      newMs = snapToNearestGrid(newMs, tempoSource, snapResolution);
    }

    const newTs: Timestamp =
      cue.timestamp.type === "measure_beat" && tempoSource
        ? msToTimestamp(newMs, tempoSource)
        : absoluteTimestamp(newMs);

    oncuechange(index, { ...cue, timestamp: newTs });
  }

  // Draw grid lines on the canvas background
  function drawGrid() {
    if (!canvasEl) return;
    const dpr = window.devicePixelRatio || 1;
    const w = canvasEl.clientWidth;
    const h = canvasEl.clientHeight;
    if (w <= 0 || h <= 0) return;
    canvasEl.width = w * dpr;
    canvasEl.height = h * dpr;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const viewStartMs = scrollLeft / pixelsPerMs;
    const viewEndMs = (scrollLeft + viewportWidth) / pixelsPerMs;
    const gridLines = getGridLines(grid, viewStartMs, viewEndMs, pixelsPerMs);
    for (const line of gridLines) {
      const x = msToPixel(line.ms, pixelsPerMs) - scrollLeft;
      ctx.strokeStyle =
        line.type === "measure"
          ? "rgba(94, 202, 234, 0.15)"
          : "rgba(94, 202, 234, 0.06)";
      ctx.lineWidth = line.type === "measure" ? 1 : 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    }

    // Playhead line
    if (playheadMs !== null && playheadMs !== undefined) {
      const px = msToPixel(playheadMs, pixelsPerMs) - scrollLeft;
      if (px >= 0 && px <= w) {
        ctx.strokeStyle = "#22c55e";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(px, 0);
        ctx.lineTo(px, h);
        ctx.stroke();
      }
    }
  }

  $effect(() => {
    void pixelsPerMs;
    void scrollLeft;
    void viewportWidth;
    void grid;
    void playheadMs;
    drawGrid();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="show-lane"
  class:sequence={laneType === "sequence"}
  style:height={laneHeight ? `${laneHeight}px` : undefined}
>
  {#if !hideLabel}
    <div class="lane-label">
      <span class="lane-name" title={name}>{name}</span>
      <button
        class="btn-icon lane-delete"
        title={$t("timeline.showLane.deleteLane", {
          values: { type: laneType },
        })}
        onclick={ondelete}
      >
        &#10005;
      </button>
    </div>
  {/if}
  <div class="lane-content" bind:this={laneEl} ondblclick={handleDblClick}>
    <canvas bind:this={canvasEl} class="lane-grid-canvas"></canvas>
    {#each getBlocks() as block, bi (`${block.index}:${block.fromSequence ?? ""}:${block.ms}:${bi}`)}
      {#if block.visible}
        <CueBlock
          cue={cues[block.index]}
          {block}
          passes={laidCues[block.index]?.passes ?? []}
          selected={selectedCueIndex === block.index}
          {pixelsPerMs}
          {subLaneType}
          {tempoSource}
          onselect={() => onselect(block.index)}
          onmove={(deltaMs) => handleCueMove(block.index, deltaMs)}
          ondelete={() => oncuedelete(block.index)}
          onresize={oneffectresize && !block.fromSequence
            ? (dur) => oneffectresize(block.index, dur)
            : undefined}
          onloopchange={onloopchange
            ? (count) => onloopchange(block.index, count)
            : undefined}
          {snapEnabled}
          {snapResolution}
          onedit={onsequenceedit && subLaneType === "sequences"
            ? () => {
                const ref = cues[block.index].sequences.find((s) => !s.stop);
                if (ref) onsequenceedit(ref.name);
              }
            : undefined}
        />
      {/if}
    {/each}
  </div>
</div>

<style>
  .show-lane {
    display: flex;
    height: 52px;
    border-bottom: 1px solid var(--card-border);
  }
  .show-lane.sequence {
    background: rgba(239, 96, 163, 0.04);
  }
  .lane-label {
    width: 100px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    padding: 0 10px;
    gap: 2px;
    border-right: 1px solid var(--card-border);
    background: var(--card-bg);
  }
  .lane-name {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 13px;
    color: var(--nc-fg-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .lane-delete {
    background: none;
    border: none;
    color: var(--nc-fg-3);
    cursor: pointer;
    font-size: 13px;
    padding: 1px 4px;
    border-radius: 3px;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .show-lane:hover .lane-delete {
    opacity: 1;
  }
  .lane-delete:hover {
    background: rgba(232, 75, 75, 0.15);
    color: var(--nc-error);
  }
  .lane-content {
    flex: 1;
    position: relative;
    min-width: 0;
    overflow: hidden;
  }
  .lane-grid-canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
</style>
