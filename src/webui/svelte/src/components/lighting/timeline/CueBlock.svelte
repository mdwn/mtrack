<script lang="ts" module>
  /** An effect as a block shows it: its type, groups and colours, from the
   *  document for the cue's own effects, from the layout for a pass's. */
  export interface BlockEffect {
    type: string;
    groups: string[];
    colors: string[];
  }

  /** One block of a lane: a cue's own effects, a pass of a sequence it
   *  references or a loop it holds, its commands, or its references. */
  export interface Block {
    /** The written cue it belongs to. */
    index: number;
    /** Where it is drawn, and where it ends. */
    ms: number;
    endMs: number;
    /** The cue's place in the tempo map before any offset: what its
     *  durations are timed from. */
    scoreMs: number;
    effects: BlockEffect[];
    /** The sequence a pass is of (`""` for an inline loop's). */
    fromSequence?: string;
    px: number;
    visible: boolean;
  }
</script>

<script lang="ts">
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import type { Cue, SubLaneType } from "../../../lib/lighting/types";
  import type {
    PassLayout,
    TempoSource,
  } from "../../../gen/rules/mtrack_rules";
  import {
    effectTypeColor,
    SNAP_BEATS,
    type SnapResolution,
  } from "../../../lib/lighting/timeline-state";
  import { rules } from "../../../lib/rules";

  interface Props {
    cue: Cue;
    block: Block;
    /** The passes of the sequences the cue references, from the layout. */
    passes?: PassLayout[];
    selected: boolean;
    onselect: () => void;
    onmove: (deltaMs: number) => void;
    ondelete: () => void;
    onresize?: (newDurationStr: string) => void;
    onloopchange?: (newLoopCount: number) => void;
    onedit?: () => void;
    pixelsPerMs: number;
    subLaneType?: SubLaneType;
    tempoSource?: TempoSource | null;
    snapEnabled?: boolean;
    snapResolution?: SnapResolution;
  }

  let {
    cue,
    block,
    passes = [],
    selected,
    onselect,
    onmove,
    ondelete,
    onresize,
    onloopchange,
    onedit,
    pixelsPerMs,
    subLaneType,
    tempoSource = null,
    snapEnabled = false,
    snapResolution = "beat",
  }: Props = $props();

  let positionPx = $derived(block.px);
  let cueMs = $derived(block.scoreMs);

  let dragging = $state(false);
  let dragOffsetPx = $state(0);
  let dragStartX = 0;
  let resizing = $state(false);
  let resizeOffsetPx = $state(0);
  let resizeStartX = 0;

  let visibleEffects = $derived(block.effects);

  let isEffectLane = $derived(
    !subLaneType ||
      subLaneType === "effects" ||
      subLaneType.startsWith("effects:"),
  );

  // A pass of a sequence or a loop, as the player placed it.
  let isFromSequence = $derived(block.fromSequence !== undefined);

  // Derive visual properties from cue content, branching on subLaneType
  let primaryColor = $derived.by(() => {
    if (subLaneType === "commands") return "#eab308";
    if (subLaneType === "sequences") return "#ef60a3";
    if (visibleEffects.length === 0) return "#555";
    const firstEffect = visibleEffects[0];
    if (firstEffect.colors.length > 0) {
      const c = firstEffect.colors[0];
      if (c.startsWith("#") || c.startsWith("rgb")) return c;
    }
    return effectTypeColor(firstEffect.type);
  });

  let label = $derived.by(() => {
    if (subLaneType === "commands") {
      return cue.commands.map((c) => c.command).join(", ");
    }
    if (subLaneType === "sequences") {
      return cue.sequences
        .map((s) => (s.stop ? "stop " : "") + s.name)
        .join(", ");
    }
    if (isEffectLane) {
      if (visibleEffects.length === 0) return get(t)("timeline.cueBlock.empty");
      const parts: string[] = [];
      for (const eff of visibleEffects) {
        const groups = eff.groups.filter((g) => g).join(", ");
        parts.push(groups ? `${groups}: ${eff.type}` : eff.type);
      }
      const from = block.fromSequence ? `${block.fromSequence}: ` : "";
      return from + parts.join(" | ");
    }
    // Combined mode (no subLaneType — e.g. SequenceEditorModal)
    if (visibleEffects.length === 0 && cue.commands.length > 0) {
      return cue.commands.map((c) => c.command).join(", ");
    }
    if (visibleEffects.length === 0) return get(t)("timeline.cueBlock.empty");
    const parts: string[] = [];
    for (const eff of visibleEffects) {
      const groups = eff.groups.filter((g) => g).join(", ");
      parts.push(groups ? `${groups}: ${eff.type}` : eff.type);
    }
    return parts.join(" | ");
  });

  // For sequence lanes: the first reference's passes, as the player laid
  // them: how long they run in all, how long one pass is, whether they
  // have an end (`loop:` runs to the timeline's horizon), and where each
  // starts.
  let seqTotalMs = $derived(
    subLaneType === "sequences" && passes.length > 0
      ? passes[0].end_ms - passes[0].start_ms
      : 0,
  );
  let seqIterMs = $derived(
    subLaneType === "sequences" && passes.length > 0 ? passes[0].pass_ms : 0,
  );
  let seqInfinite = $derived(
    subLaneType === "sequences" && passes.length > 0 && passes[0].infinite,
  );
  /** Where the passes after the first begin, as a fraction of the block. */
  let passDividers = $derived(
    subLaneType === "sequences" && passes.length > 0 && seqTotalMs > 0
      ? passes[0].pass_starts
          .slice(1)
          .map((start) => (start - passes[0].start_ms) / seqTotalMs)
      : [],
  );

  /** How many whole passes the dragged reference's end, moved by
   *  `offsetPx`, reaches: those of the passes the layout laid on to the
   *  end of the timeline that end by then (the player's `passes_by`). The
   *  drag starts at that reference's own end, not the block's (which is
   *  as long as its longest reference, and never narrower than it can be
   *  grabbed), so the smallest drag keeps the count. The ghost shows it
   *  and a drop writes it. */
  function passesReached(offsetPx: number): number {
    const endMs = passes[0].end_ms + offsetPx / pixelsPerMs;
    return rules.passes_by(passes[0].pass_ends, endMs) ?? 1;
  }

  // During resize: the passes the drag reaches, and where the last ends.
  let resizePreviewLoopCount = $derived(
    resizing && subLaneType === "sequences" && passes.length > 0 && !seqInfinite
      ? passesReached(resizeOffsetPx)
      : 0,
  );

  let resizePreviewWidth = $derived(
    resizePreviewLoopCount > 0
      ? ((passes[0].pass_ends[resizePreviewLoopCount - 1] ??
          passes[0].start_ms + seqIterMs) -
          passes[0].start_ms) *
          pixelsPerMs
      : 0,
  );

  // The block is as long as the layout says: the longest of its effects,
  // or the passes it references.
  let blockWidth = $derived(
    Math.max(24, pixelsPerMs * (block.endMs - block.ms)),
  );

  function handlePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    onselect();
    dragging = true;
    dragOffsetPx = 0;
    dragStartX = e.clientX;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function handlePointerMove(e: PointerEvent) {
    if (dragging) {
      dragOffsetPx = e.clientX - dragStartX;
    }
    if (resizing) {
      resizeOffsetPx = e.clientX - resizeStartX;
    }
  }

  function handlePointerUp(e: PointerEvent) {
    if (dragging) {
      const deltaPx = dragOffsetPx;
      dragging = false;
      dragOffsetPx = 0;
      if (Math.abs(deltaPx) > 3) {
        const deltaMs = deltaPx / pixelsPerMs;
        onmove(deltaMs);
      }
    }
    if (resizing) {
      const deltaPx = resizeOffsetPx;
      resizing = false;
      resizeOffsetPx = 0;
      if (Math.abs(deltaPx) > 3) {
        const currentWidthMs = blockWidth / pixelsPerMs;
        const newTotalMs = Math.max(
          100,
          currentWidthMs + deltaPx / pixelsPerMs,
        );
        // Hold Ctrl/Cmd to bypass snap
        const bypassSnap = e.ctrlKey || e.metaKey;
        if (subLaneType === "sequences") {
          // The passes the dragged end reaches, as the ghost showed them;
          // a `loop:` has no count to write.
          if (passes.length > 0 && !seqInfinite && onloopchange) {
            onloopchange(passesReached(deltaPx));
          }
        } else if (onresize) {
          const snappedMs =
            snapEnabled && tempoSource && !bypassSnap
              ? rules.duration_snap(
                  tempoSource,
                  cueMs,
                  newTotalMs,
                  SNAP_BEATS[snapResolution],
                )
              : newTotalMs;
          onresize(
            rules.duration_text(tempoSource ?? undefined, cueMs, snappedMs),
          );
        }
      }
    }
  }

  function handleDblClick(e: MouseEvent) {
    if (onedit) {
      e.stopPropagation(); // prevent lane from creating a new cue
      onedit();
    }
  }

  function handleResizePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.stopPropagation();
    resizing = true;
    resizeOffsetPx = 0;
    resizeStartX = e.clientX;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      ondelete();
    }
  }
</script>

<div
  class="cue-block"
  class:selected
  class:dragging
  class:resizing
  class:from-sequence={isFromSequence}
  style:left="{positionPx + dragOffsetPx}px"
  style:width="{subLaneType === 'sequences' && resizing
    ? blockWidth
    : blockWidth + resizeOffsetPx}px"
  style:--cue-color={primaryColor}
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  ondblclick={handleDblClick}
  onkeydown={handleKeyDown}
  tabindex="0"
  role="button"
  title={label}
>
  <div class="cue-color-strip"></div>
  <div class="cue-content">
    {#if isEffectLane}
      <div class="cue-effects">
        {#each visibleEffects.slice(0, 3) as eff, ei (ei)}
          <span
            class="effect-dot"
            style:background={effectTypeColor(eff.type)}
            title={eff.type}
          ></span>
        {/each}
        {#if visibleEffects.length > 3}
          <span class="effect-overflow">+{visibleEffects.length - 3}</span>
        {/if}
      </div>
      <span class="cue-label">{label}</span>
      {#if !subLaneType && cue.commands.length > 0}
        <span class="badge cmd-badge"
          >{$t("timeline.cueBlock.cmdCount", {
            values: { count: cue.commands.length },
          })}</span
        >
      {/if}
      {#if !subLaneType && cue.sequences.length > 0}
        <span class="badge seq-badge"
          >{$t("timeline.cueBlock.seqCount", {
            values: { count: cue.sequences.length },
          })}</span
        >
      {/if}
    {:else if subLaneType === "commands"}
      <span class="cue-label cmd-label">{label}</span>
    {:else if subLaneType === "sequences"}
      <span class="cue-label seq-label">{label}</span>
    {/if}
  </div>
  {#if passDividers.length > 0}
    <div class="seq-iter-markers">
      {#each passDividers as at, i (i)}
        <div class="seq-iter-divider" style:left="{at * 100}%"></div>
      {/each}
    </div>
  {/if}
  {#if (isEffectLane && !isFromSequence) || (subLaneType === "sequences" && !seqInfinite)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-handle" onpointerdown={handleResizePointerDown}></div>
  {/if}
</div>
{#if subLaneType === "sequences" && resizing && resizePreviewWidth > 0}
  <div
    class="seq-resize-ghost"
    style:left="{positionPx}px"
    style:width="{resizePreviewWidth}px"
  >
    <span class="ghost-label">{resizePreviewLoopCount}x</span>
  </div>
{/if}

<style>
  .cue-block {
    position: absolute;
    top: 3px;
    bottom: 3px;
    border-radius: 6px;
    background: rgba(94, 202, 234, 0.18);
    border: 1px solid rgba(94, 202, 234, 0.4);
    cursor: grab;
    overflow: hidden;
    display: flex;
    transition: box-shadow 0.1s;
    z-index: 1;
  }
  .cue-block:hover {
    background: rgba(94, 202, 234, 0.28);
    border-color: rgba(94, 202, 234, 0.55);
  }
  .cue-block.selected {
    border-color: var(--nc-cyan-500);
    box-shadow: 0 0 0 1px var(--nc-cyan-500);
    z-index: 2;
  }
  .cue-block.from-sequence {
    border-style: dashed;
    background: rgba(239, 96, 163, 0.06);
    border-color: rgba(239, 96, 163, 0.25);
  }
  .cue-block.from-sequence:hover {
    background: rgba(239, 96, 163, 0.1);
    border-color: rgba(239, 96, 163, 0.35);
  }
  .cue-block.dragging {
    cursor: grabbing;
    opacity: 0.8;
  }
  .cue-block.resizing {
    cursor: ew-resize;
    opacity: 0.9;
  }
  .cue-block:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .cue-color-strip {
    width: 4px;
    flex-shrink: 0;
    background: var(--cue-color);
  }
  .cue-content {
    flex: 1;
    min-width: 0;
    padding: 2px 4px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    justify-content: center;
  }
  .cue-effects {
    display: flex;
    gap: 2px;
    align-items: center;
  }
  .effect-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .effect-overflow {
    font-size: 11px;
    color: var(--text-dim);
  }
  .cue-label {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .badge {
    font-size: 11px;
    padding: 0 3px;
    border-radius: 2px;
    align-self: flex-start;
  }
  .cmd-badge {
    background: rgba(234, 179, 8, 0.2);
    color: var(--yellow);
  }
  .seq-badge {
    background: rgba(239, 96, 163, 0.2);
    color: var(--pink);
  }
  .cmd-label {
    color: var(--yellow, #eab308);
  }
  .seq-label {
    color: var(--pink, #ef60a3);
  }
  .seq-iter-markers {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .seq-iter-divider {
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: 1px;
    background: rgba(239, 96, 163, 0.3);
  }
  .seq-resize-ghost {
    position: absolute;
    top: 3px;
    bottom: 3px;
    border-radius: 4px;
    background: rgba(239, 96, 163, 0.08);
    border: 1px dashed rgba(239, 96, 163, 0.35);
    pointer-events: none;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: 8px;
    z-index: 0;
  }
  .ghost-label {
    font-size: 11px;
    font-weight: 600;
    color: rgba(239, 96, 163, 0.6);
  }
  .resize-handle {
    width: 6px;
    flex-shrink: 0;
    cursor: ew-resize;
    background: transparent;
    border-left: 1px solid rgba(255, 255, 255, 0.08);
    transition: background 0.1s;
  }
  .resize-handle:hover,
  .resizing .resize-handle {
    background: rgba(255, 255, 255, 0.15);
  }
</style>
