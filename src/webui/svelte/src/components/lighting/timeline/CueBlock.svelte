<script lang="ts">
  import type { Cue } from "../../../lib/lighting/types";
  import { effectTypeColor } from "../../../lib/lighting/timeline-state";

  interface Props {
    cue: Cue;
    positionPx: number;
    selected: boolean;
    onselect: () => void;
    onmove: (deltaMs: number) => void;
    ondelete: () => void;
    pixelsPerMs: number;
  }

  let {
    cue,
    positionPx,
    selected,
    onselect,
    onmove,
    ondelete,
    pixelsPerMs,
  }: Props = $props();

  let dragging = $state(false);
  let dragOffsetPx = $state(0);
  let dragStartX = 0;

  // Derive visual properties from cue content
  let primaryColor = $derived.by(() => {
    if (cue.effects.length === 0) return "#555";
    const firstEffect = cue.effects[0].effect;
    // Use the first explicit color if available
    if (firstEffect.colors.length > 0) {
      const c = firstEffect.colors[0];
      if (c.startsWith("#") || c.startsWith("rgb")) return c;
      // Named colors — use effect type color as fallback
    }
    return effectTypeColor(firstEffect.type);
  });

  let label = $derived.by(() => {
    if (cue.effects.length === 0 && cue.commands.length > 0) {
      return cue.commands.map((c) => c.command).join(", ");
    }
    if (cue.effects.length === 0) return "empty";
    const parts: string[] = [];
    for (const eff of cue.effects) {
      const groups = eff.groups.filter((g) => g).join(", ");
      parts.push(groups ? `${groups}: ${eff.effect.type}` : eff.effect.type);
    }
    return parts.join(" | ");
  });

  let blockWidth = $derived(Math.max(24, pixelsPerMs * 500));

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
    if (!dragging) return;
    dragOffsetPx = e.clientX - dragStartX;
  }

  function handlePointerUp() {
    if (!dragging) return;
    const deltaPx = dragOffsetPx;
    dragging = false;
    dragOffsetPx = 0;
    if (Math.abs(deltaPx) > 3) {
      const deltaMs = deltaPx / pixelsPerMs;
      onmove(deltaMs);
    }
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
  style:left="{positionPx + dragOffsetPx}px"
  style:width="{blockWidth}px"
  style:--cue-color={primaryColor}
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onkeydown={handleKeyDown}
  tabindex="0"
  role="button"
  title={label}
>
  <div class="cue-color-strip"></div>
  <div class="cue-content">
    <div class="cue-effects">
      {#each cue.effects.slice(0, 3) as eff, ei (ei)}
        <span
          class="effect-dot"
          style:background={effectTypeColor(eff.effect.type)}
          title={eff.effect.type}
        ></span>
      {/each}
      {#if cue.effects.length > 3}
        <span class="effect-overflow">+{cue.effects.length - 3}</span>
      {/if}
    </div>
    <span class="cue-label">{label}</span>
    {#if cue.commands.length > 0}
      <span class="badge cmd-badge">{cue.commands.length} cmd</span>
    {/if}
    {#if cue.sequences.length > 0}
      <span class="badge seq-badge">{cue.sequences.length} seq</span>
    {/if}
  </div>
</div>

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
  .cue-block.dragging {
    cursor: grabbing;
    opacity: 0.8;
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
    font-size: 8px;
    color: var(--text-dim);
  }
  .cue-label {
    font-size: 9px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .badge {
    font-size: 8px;
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
</style>
