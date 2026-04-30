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
  import { formatMs } from "../../../lib/lighting/timeline-state";

  interface Props {
    pixelsPerMs: number;
    cursorMs: number | null;
    snapEnabled: boolean;
    snapResolution: import("../../../lib/lighting/timeline-state").SnapResolution;
    hasTempo: boolean;
    isPlaying: boolean;
    canPlay: boolean;
    playheadMs?: number | null;
    playCursorMs?: number | null;
    onzoom: (pixelsPerMs: number) => void;
    onfitview: () => void;
    onsnapchange: (
      enabled: boolean,
      resolution: import("../../../lib/lighting/timeline-state").SnapResolution,
    ) => void;
    onaddshow: () => void;
    onaddsequence: () => void;
    onplay?: () => void;
    onpause?: () => void;
    onstop?: () => void;
    onskipstart?: () => void;
    onskipend?: () => void;
  }

  let {
    pixelsPerMs,
    cursorMs,
    snapEnabled,
    snapResolution,
    hasTempo,
    isPlaying,
    canPlay,
    playheadMs = null,
    playCursorMs = null,
    onzoom,
    onfitview,
    onsnapchange,
    onaddshow,
    onaddsequence,
    onplay,
    onpause = () => {},
    onstop = () => {},
    onskipstart = () => {},
    onskipend = () => {},
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
  <div class="toolbar-group transport">
    <button
      class="btn btn-sm btn-transport"
      onclick={onskipstart}
      title={$t("timeline.toolbar.goToStart")}
      disabled={isPlaying}>&#9198;</button
    >
    <button
      class="btn btn-sm btn-transport btn-stop-transport"
      onclick={onstop}
      title={$t("timeline.toolbar.stopReset")}
      disabled={!isPlaying && (playCursorMs === null || playCursorMs === 0)}
      >&#9632;</button
    >
    {#if isPlaying}
      <button
        class="btn btn-sm btn-transport btn-pause"
        onclick={onpause}
        title={$t("timeline.toolbar.pause")}>&#9646;&#9646;</button
      >
    {:else}
      <button
        class="btn btn-sm btn-transport btn-play"
        onclick={onplay}
        disabled={!canPlay}
        title={$t("timeline.toolbar.playFromCursor")}>&#9654;</button
      >
    {/if}
    <button
      class="btn btn-sm btn-transport"
      onclick={onskipend}
      title={$t("timeline.toolbar.goToEnd")}
      disabled={isPlaying}>&#9197;</button
    >
  </div>

  <div class="toolbar-group">
    <button
      class="btn btn-sm"
      onclick={zoomOut}
      title={$t("timeline.toolbar.zoomOut")}>-</button
    >
    <button
      class="btn btn-sm"
      onclick={onfitview}
      title={$t("timeline.toolbar.fitToView")}
      >{$t("timeline.toolbar.fit")}</button
    >
    <button
      class="btn btn-sm"
      onclick={zoomIn}
      title={$t("timeline.toolbar.zoomIn")}>+</button
    >
  </div>

  {#if hasTempo}
    <div class="toolbar-group">
      <label class="snap-toggle">
        <input
          type="checkbox"
          checked={snapEnabled}
          onchange={() => onsnapchange(!snapEnabled, snapResolution)}
        />
        <span>{$t("timeline.toolbar.snap")}</span>
      </label>
      {#if snapEnabled}
        <select
          class="snap-select"
          value={snapResolution}
          onchange={(e) =>
            onsnapchange(
              true,
              (e.target as HTMLSelectElement)
                .value as import("../../../lib/lighting/timeline-state").SnapResolution,
            )}
        >
          <option value="measure">{$t("timeline.toolbar.measure")}</option>
          <option value="beat">{$t("timeline.toolbar.beat")}</option>
          <option value="1/2">1/2</option>
          <option value="1/4">1/4</option>
          <option value="1/8">1/8</option>
          <option value="1/16">1/16</option>
        </select>
      {/if}
    </div>
  {/if}

  <div class="toolbar-group">
    <button class="btn btn-sm" onclick={onaddshow}
      >{$t("timeline.toolbar.addShow")}</button
    >
    <button class="btn btn-sm" onclick={onaddsequence}
      >{$t("timeline.toolbar.addSequence")}</button
    >
  </div>

  <div class="toolbar-spacer"></div>

  {#if isPlaying && playheadMs !== null}
    <span class="cursor-time playhead-time">{formatMs(playheadMs)}</span>
  {:else if cursorMs !== null}
    <span class="cursor-time">{formatMs(cursorMs)}</span>
  {:else if playCursorMs !== null && playCursorMs > 0}
    <span class="cursor-time play-cursor-time">{formatMs(playCursorMs)}</span>
  {/if}
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
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
  .transport {
    gap: 2px;
  }
  .btn-transport {
    min-width: 28px;
    padding: 3px 5px;
    font-size: 12px;
    line-height: 1;
    letter-spacing: -1px;
  }
  .btn-transport:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }
  .btn-play {
    color: var(--green, #22c55e);
  }
  .btn-play:hover:not(:disabled) {
    background: rgba(34, 197, 94, 0.2);
  }
  .btn-pause {
    color: var(--accent, #5eacea);
  }
  .btn-pause:hover {
    background: rgba(94, 172, 234, 0.2);
  }
  .btn-stop-transport {
    color: var(--text-muted);
  }
  .btn-stop-transport:hover:not(:disabled) {
    color: var(--red, #ef4444);
    background: rgba(239, 68, 68, 0.15);
  }
  .cursor-time {
    font-family: var(--mono);
    font-size: 13px;
    color: var(--text-muted);
    min-width: 80px;
    text-align: right;
  }
  .playhead-time {
    color: var(--green, #22c55e);
  }
  .play-cursor-time {
    color: rgba(34, 197, 94, 0.6);
  }
</style>
