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
  import { formatMs } from "../../../lib/lighting/timeline-state";

  interface Props {
    pixelsPerMs: number;
    cursorMs: number | null;
    snapEnabled: boolean;
    snapResolution: "beat" | "measure";
    hasTempo: boolean;
    onzoom: (pixelsPerMs: number) => void;
    onfitview: () => void;
    onsnapchange: (enabled: boolean, resolution: "beat" | "measure") => void;
    onaddshow: () => void;
    onaddsequence: () => void;
  }

  let {
    pixelsPerMs,
    cursorMs,
    snapEnabled,
    snapResolution,
    hasTempo,
    onzoom,
    onfitview,
    onsnapchange,
    onaddshow,
    onaddsequence,
  }: Props = $props();

  const MIN_ZOOM = 0.01; // 10px per second
  const MAX_ZOOM = 2; // 2000px per second

  function zoomIn() {
    onzoom(Math.min(pixelsPerMs * 1.5, MAX_ZOOM));
  }

  function zoomOut() {
    onzoom(Math.max(pixelsPerMs / 1.5, MIN_ZOOM));
  }
</script>

<div class="toolbar">
  <div class="toolbar-group">
    <button class="btn btn-sm" onclick={zoomOut} title="Zoom out">-</button>
    <button class="btn btn-sm" onclick={onfitview} title="Fit to view"
      >Fit</button
    >
    <button class="btn btn-sm" onclick={zoomIn} title="Zoom in">+</button>
  </div>

  {#if hasTempo}
    <div class="toolbar-group">
      <label class="snap-toggle">
        <input
          type="checkbox"
          checked={snapEnabled}
          onchange={() => onsnapchange(!snapEnabled, snapResolution)}
        />
        <span>Snap</span>
      </label>
      {#if snapEnabled}
        <select
          class="snap-select"
          value={snapResolution}
          onchange={(e) =>
            onsnapchange(
              true,
              (e.target as HTMLSelectElement).value as "beat" | "measure",
            )}
        >
          <option value="beat">Beat</option>
          <option value="measure">Measure</option>
        </select>
      {/if}
    </div>
  {/if}

  <div class="toolbar-group">
    <button class="btn btn-sm" onclick={onaddshow}>+ Show</button>
    <button class="btn btn-sm" onclick={onaddsequence}>+ Sequence</button>
  </div>

  <div class="toolbar-spacer"></div>

  {#if cursorMs !== null}
    <span class="cursor-time">{formatMs(cursorMs)}</span>
  {/if}
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    flex-shrink: 0;
  }
  .toolbar-group {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .toolbar-spacer {
    flex: 1;
  }
  .snap-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .snap-toggle input {
    margin: 0;
  }
  .snap-select {
    font-size: 12px;
    padding: 2px 4px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text);
  }
  .cursor-time {
    font-family: var(--mono);
    font-size: 13px;
    color: var(--text-muted);
    min-width: 80px;
    text-align: right;
  }
</style>
