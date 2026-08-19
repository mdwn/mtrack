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
  export interface LaneMarker {
    id: string;
    ms: number;
    label: string;
    /** Optional icon prefix rendered before the label (e.g. 🔊). */
    icon?: string;
  }

  interface Props {
    laneLabel: string;
    markers: LaneMarker[];
    pixelsPerMs: number;
    scrollLeft: number;
    /** Accent for the marker pins/chips (any CSS color). */
    accent: string;
    /** Shown centered when there are no markers. */
    emptyHint?: string;
    onmarkerclick: (id: string) => void;
    /** Click on empty lane space, in song ms. */
    onemptyclick: (ms: number) => void;
  }

  let {
    laneLabel,
    markers,
    pixelsPerMs,
    scrollLeft,
    accent,
    emptyHint = "",
    onmarkerclick,
    onemptyclick,
  }: Props = $props();

  const LANE_HEIGHT = 30;
  const LABEL_WIDTH = 80;

  let positioned = $derived(
    markers.map((m) => ({ ...m, left: m.ms * pixelsPerMs - scrollLeft })),
  );

  function handleLaneClick(e: MouseEvent) {
    // Marker chips stop propagation, so this is always empty space.
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = e.clientX - rect.left;
    const ms = Math.max(0, (scrollLeft + x) / pixelsPerMs);
    onemptyclick(ms);
  }
</script>

<div class="marker-lane" style:height="{LANE_HEIGHT}px">
  <div class="lane-label" style:width="{LABEL_WIDTH}px">{laneLabel}</div>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="lane-content" onclick={handleLaneClick}>
    {#if markers.length === 0 && emptyHint}
      <span class="lane-empty-hint">{emptyHint}</span>
    {/if}
    {#each positioned as marker (marker.id)}
      <button
        type="button"
        class="marker"
        style:left="{marker.left}px"
        style:--marker-accent={accent}
        onclick={(e) => {
          e.stopPropagation();
          onmarkerclick(marker.id);
        }}
      >
        <span class="marker-pin"></span>
        <span class="marker-chip">
          {#if marker.icon}<span class="marker-icon">{marker.icon}</span
            >{/if}{marker.label}
        </span>
      </button>
    {/each}
  </div>
</div>

<style>
  .marker-lane {
    display: flex;
    border-bottom: 1px solid var(--border);
    position: sticky;
    left: 0;
  }
  .lane-label {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-dim);
    border-right: 1px solid var(--border);
    font-weight: 600;
  }
  .lane-content {
    flex: 1;
    position: relative;
    overflow: hidden;
    background: color-mix(in srgb, var(--nc-fg-1) 2%, transparent);
    cursor: copy;
  }
  .lane-empty-hint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    color: var(--text-dim);
    font-style: italic;
    pointer-events: none;
  }
  .marker {
    position: absolute;
    top: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 0;
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
    /* Generous touch target that extends past the visible pin. */
    margin-left: -8px;
    padding-left: 8px;
  }
  .marker:hover,
  .marker:focus-visible {
    z-index: 2;
  }
  .marker-pin {
    width: 2px;
    align-self: stretch;
    background: var(--marker-accent);
    flex-shrink: 0;
  }
  .marker-chip {
    font-size: 10px;
    font-weight: 600;
    color: var(--text);
    background: color-mix(in srgb, var(--marker-accent) 22%, transparent);
    border: 1px solid color-mix(in srgb, var(--marker-accent) 55%, transparent);
    border-left: none;
    border-radius: 0 8px 8px 0;
    padding: 2px 7px 2px 5px;
    white-space: nowrap;
    line-height: 1.4;
  }
  .marker:hover .marker-chip {
    background: color-mix(in srgb, var(--marker-accent) 38%, transparent);
  }
  .marker-icon {
    margin-right: 3px;
    font-size: 9px;
  }
</style>
