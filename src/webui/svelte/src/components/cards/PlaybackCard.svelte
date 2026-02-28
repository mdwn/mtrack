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
  import { playbackStore } from "../../lib/ws/stores";
  import { playerClient } from "../../lib/grpc/client";
  import { formatMs } from "../../lib/util/format";

  async function play() {
    try {
      await playerClient.play({});
    } catch (e) {
      console.error("play failed:", e);
    }
  }

  async function stop() {
    try {
      await playerClient.stop({});
    } catch (e) {
      console.error("stop failed:", e);
    }
  }

  async function next() {
    try {
      await playerClient.next({});
    } catch (e) {
      console.error("next failed:", e);
    }
  }

  async function previous() {
    try {
      await playerClient.previous({});
    } catch (e) {
      console.error("previous failed:", e);
    }
  }

  let progressPct = $derived(
    $playbackStore.song_duration_ms > 0
      ? ($playbackStore.elapsed_ms / $playbackStore.song_duration_ms) * 100
      : 0,
  );
</script>

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">Playback</span>
  </div>
  <div class="playback-song">{$playbackStore.song_name || "No song"}</div>
  <div class="playback-status">
    {#if $playbackStore.is_playing}
      <span class="playing">Playing</span>
    {:else}
      <span class="stopped">Stopped</span>
    {/if}
  </div>
  <div class="progress-bar">
    <div class="progress-fill" style:width="{progressPct}%"></div>
  </div>
  <div class="progress-time">
    <span>{formatMs($playbackStore.elapsed_ms)}</span>
    <span>{formatMs($playbackStore.song_duration_ms)}</span>
  </div>
  <div class="controls">
    <button class="btn" onclick={previous} disabled={$playbackStore.is_playing}
      >Prev</button
    >
    {#if $playbackStore.is_playing}
      <button class="btn btn-primary" onclick={stop}>Stop</button>
    {:else}
      <button class="btn btn-primary" onclick={play}>Play</button>
    {/if}
    <button class="btn" onclick={next} disabled={$playbackStore.is_playing}
      >Next</button
    >
  </div>
</div>

<style>
  .playback-song {
    font-size: 20px;
    font-weight: 600;
    margin-bottom: 4px;
    color: var(--text);
  }
  .playback-status {
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 12px;
  }
  .playback-status .playing {
    color: var(--green);
  }
  .playback-status .stopped {
    color: var(--text-dim);
  }
  .progress-bar {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
    margin-bottom: 6px;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.2s linear;
  }
  .progress-time {
    display: flex;
    justify-content: space-between;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-dim);
  }
  .controls {
    display: flex;
    justify-content: center;
    gap: 8px;
    margin-top: 12px;
  }
</style>
