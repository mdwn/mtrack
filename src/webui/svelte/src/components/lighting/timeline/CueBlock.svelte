<script lang="ts">
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import type {
    Cue,
    Sequence,
    SubLaneType,
    TempoSection,
  } from "../../../lib/lighting/types";
  import {
    effectTypeColor,
    durationStringToMs,
    msToDurationString,
    getSequenceIterationMs,
    snapDurationToGrid,
    type SnapResolution,
  } from "../../../lib/lighting/timeline-state";

  interface Props {
    cue: Cue;
    positionPx: number;
    selected: boolean;
    onselect: () => void;
    onmove: (deltaMs: number) => void;
    ondelete: () => void;
    onresize?: (newDurationStr: string) => void;
    onloopchange?: (newLoopCount: number) => void;
    onedit?: () => void;
    pixelsPerMs: number;
    subLaneType?: SubLaneType;
    tempo?: TempoSection;
    cueMs?: number;
    sequenceDefs?: Sequence[];
    snapEnabled?: boolean;
    snapResolution?: SnapResolution;
  }

  let {
    cue,
    positionPx,
    selected,
    onselect,
    onmove,
    ondelete,
    onresize,
    onloopchange,
    onedit,
    pixelsPerMs,
    subLaneType,
    tempo,
    cueMs = 0,
    sequenceDefs = [],
    snapEnabled = false,
    snapResolution = "beat",
  }: Props = $props();

  let dragging = $state(false);
  let dragOffsetPx = $state(0);
  let dragStartX = 0;

  let resizing = $state(false);
  let resizeOffsetPx = $state(0);
  let resizeStartX = 0;

  // Filter effects to the current layer lane (if applicable)
  let visibleEffects = $derived.by(() => {
    if (subLaneType?.startsWith("effects:")) {
      const layer = subLaneType.split(":")[1];
      return cue.effects.filter(
        (e) => (e.effect.layer ?? "background") === layer,
      );
    }
    return cue.effects;
  });

  let isEffectLane = $derived(
    !subLaneType ||
      subLaneType === "effects" ||
      subLaneType.startsWith("effects:"),
  );

  // Check if any visible effect comes from a sequence
  let isFromSequence = $derived(
    visibleEffects.some((e) => e.sequenceName != null),
  );

  // Derive visual properties from cue content, branching on subLaneType
  let primaryColor = $derived.by(() => {
    if (subLaneType === "commands") return "#eab308";
    if (subLaneType === "sequences") return "#ef60a3";
    if (visibleEffects.length === 0) return "#555";
    const firstEffect = visibleEffects[0].effect;
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
        parts.push(groups ? `${groups}: ${eff.effect.type}` : eff.effect.type);
      }
      return parts.join(" | ");
    }
    // Combined mode (no subLaneType — e.g. SequenceEditorModal)
    if (visibleEffects.length === 0 && cue.commands.length > 0) {
      return cue.commands.map((c) => c.command).join(", ");
    }
    if (visibleEffects.length === 0) return get(t)("timeline.cueBlock.empty");
    const parts: string[] = [];
    for (const eff of visibleEffects) {
      const groups = eff.groups.filter((g) => g).join(", ");
      parts.push(groups ? `${groups}: ${eff.effect.type}` : eff.effect.type);
    }
    return parts.join(" | ");
  });

  // For sequence lanes: find the first non-stop sequence ref and compute its total duration
  let seqTotalMs = $derived.by(() => {
    if (subLaneType !== "sequences") return 0;
    for (const ref of cue.sequences) {
      if (ref.stop) continue;
      const def = sequenceDefs.find((s) => s.name === ref.name);
      if (!def) continue;
      const iterMs = getSequenceIterationMs(def, cueMs, tempo);
      const loopCount = ref.loop ? parseInt(ref.loop, 10) || 1 : 1;
      return iterMs * loopCount;
    }
    return 0;
  });

  // For sequence lanes: the iteration duration (for snapping resize to whole iterations)
  let seqIterMs = $derived.by(() => {
    if (subLaneType !== "sequences") return 0;
    for (const ref of cue.sequences) {
      if (ref.stop) continue;
      const def = sequenceDefs.find((s) => s.name === ref.name);
      if (!def) continue;
      return getSequenceIterationMs(def, cueMs, tempo);
    }
    return 0;
  });

  // For sequence lanes: the current loop count
  let seqLoopCount = $derived.by(() => {
    if (subLaneType !== "sequences" || seqIterMs <= 0) return 0;
    return Math.round(seqTotalMs / seqIterMs);
  });

  // During resize: the snapped preview loop count and width
  let resizePreviewLoopCount = $derived.by(() => {
    if (!resizing || subLaneType !== "sequences" || seqIterMs <= 0) return 0;
    const currentWidthMs = blockWidth / pixelsPerMs;
    const newTotalMs = Math.max(
      seqIterMs,
      currentWidthMs + resizeOffsetPx / pixelsPerMs,
    );
    return Math.max(1, Math.round(newTotalMs / seqIterMs));
  });

  let resizePreviewWidth = $derived(
    resizePreviewLoopCount > 0
      ? resizePreviewLoopCount * seqIterMs * pixelsPerMs
      : 0,
  );

  // Compute block width from effect/sequence duration
  let blockWidth = $derived.by(() => {
    if (subLaneType === "commands") {
      return Math.max(24, pixelsPerMs * 500);
    }
    if (subLaneType === "sequences") {
      if (seqTotalMs > 0) return Math.max(24, pixelsPerMs * seqTotalMs);
      return Math.max(24, pixelsPerMs * 500);
    }
    if (!isEffectLane) {
      return Math.max(24, pixelsPerMs * 500);
    }
    // For effect blocks, derive width from the maximum duration across all effects
    if (visibleEffects.length > 0) {
      let maxDurMs = 0;
      for (const eff of visibleEffects) {
        const durStr = eff.effect.duration ?? eff.effect.extra?.hold_time;
        const durMs = durationStringToMs(durStr, tempo, cueMs);
        if (durMs > maxDurMs) maxDurMs = durMs;
      }
      if (maxDurMs > 0) {
        return Math.max(24, pixelsPerMs * maxDurMs);
      }
    }
    // Fallback for effects without duration
    return Math.max(24, pixelsPerMs * 500);
  });

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

        if (subLaneType === "sequences" && seqIterMs > 0 && onloopchange) {
          // Snap to nearest whole iteration
          const newLoopCount = Math.max(1, Math.round(newTotalMs / seqIterMs));
          onloopchange(newLoopCount);
        } else if (onresize) {
          const snappedMs =
            snapEnabled && tempo && !bypassSnap
              ? snapDurationToGrid(newTotalMs, cueMs, tempo, snapResolution)
              : newTotalMs;
          onresize(msToDurationString(snappedMs, tempo, cueMs));
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
            style:background={effectTypeColor(eff.effect.type)}
            title={eff.effect.type}
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
  {#if subLaneType === "sequences" && seqLoopCount > 1}
    <div class="seq-iter-markers">
      {#each Array.from({ length: seqLoopCount - 1 }, (_, k) => k) as i (i)}
        <div
          class="seq-iter-divider"
          style:left="{((i + 1) / seqLoopCount) * 100}%"
        ></div>
      {/each}
    </div>
  {/if}
  {#if isEffectLane || subLaneType === "sequences"}
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
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.1);
    cursor: grab;
    overflow: hidden;
    display: flex;
    transition: box-shadow 0.1s;
    z-index: 1;
  }
  .cue-block:hover {
    background: rgba(255, 255, 255, 0.1);
    border-color: rgba(255, 255, 255, 0.2);
  }
  .cue-block.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
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
    width: 3px;
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
