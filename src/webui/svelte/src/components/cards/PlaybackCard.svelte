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
  import { Code, ConnectError } from "@connectrpc/connect";
  import { playbackStore } from "../../lib/ws/stores";
  import { playerClient } from "../../lib/grpc/client";
  import { formatMs } from "../../lib/util/format";

  let errorMsg = $state("");

  function showError(msg: string) {
    errorMsg = msg;
    setTimeout(() => (errorMsg = ""), 3000);
  }

  async function play() {
    loading = true;
    try {
      await playerClient.play({});
    } catch (e) {
      console.error("play failed:", e);
      showError("Play failed");
    } finally {
      loading = false;
    }
  }

  async function stop() {
    loading = true;
    try {
      await playerClient.stop({});
    } catch (e) {
      console.error("stop failed:", e);
      showError("Stop failed");
    } finally {
      loading = false;
    }
  }

  async function next() {
    loading = true;
    try {
      await playerClient.next({});
    } catch (e) {
      if (e instanceof ConnectError && e.code === Code.OutOfRange) {
        // Already at end of playlist — not an error.
      } else {
        console.error("next failed:", e);
        showError("Next failed");
      }
    } finally {
      loading = false;
    }
  }

  async function previous() {
    loading = true;
    try {
      await playerClient.previous({});
    } catch (e) {
      if (e instanceof ConnectError && e.code === Code.OutOfRange) {
        // Already at beginning of playlist — not an error.
      } else {
        console.error("previous failed:", e);
        showError("Previous failed");
      }
    } finally {
      loading = false;
    }
  }

  let loading = $state(false);

  function handleKeydown(e: KeyboardEvent) {
    // Don't intercept when typing in form fields
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

    switch (e.code) {
      case "Space":
        e.preventDefault();
        if ($playbackStore.is_playing) stop();
        else play();
        break;
      case "ArrowRight":
        e.preventDefault();
        next();
        break;
      case "ArrowLeft":
        e.preventDefault();
        previous();
        break;
    }
  }

  let canPrev = $derived(
    $playbackStore.playlist_songs.length > 0 &&
      $playbackStore.playlist_position > 0,
  );
  let canNext = $derived(
    $playbackStore.playlist_songs.length > 0 &&
      $playbackStore.playlist_position <
        $playbackStore.playlist_songs.length - 1,
  );

  let progressPct = $derived(
    $playbackStore.song_duration_ms > 0
      ? ($playbackStore.elapsed_ms / $playbackStore.song_duration_ms) * 100
      : 0,
  );
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">Playback</span>
  </div>
  <div class="transport">
    <div class="transport-info">
      <div class="playback-song">{$playbackStore.song_name || "No song"}</div>
      <div class="playback-status">
        {#if $playbackStore.is_playing}
          <span class="playing">Playing</span>
        {:else}
          <span class="stopped">Stopped</span>
        {/if}
      </div>
    </div>
    <div class="transport-progress">
      <div
        class="progress-bar"
        role="progressbar"
        aria-valuenow={progressPct}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label="Song progress"
      >
        <div class="progress-fill" style:width="{progressPct}%"></div>
      </div>
      <div class="progress-time">
        <span>{formatMs($playbackStore.elapsed_ms)}</span>
        <span>{formatMs($playbackStore.song_duration_ms)}</span>
      </div>
    </div>
    <div class="controls">
      <button
        class="btn"
        onclick={previous}
        disabled={$playbackStore.is_playing || loading || !canPrev}
        title="Previous (Left Arrow)">Prev</button
      >
      {#if $playbackStore.is_playing}
        <button
          class="btn btn-primary"
          onclick={stop}
          disabled={loading}
          title="Stop (Space)">Stop</button
        >
      {:else}
        <button
          class="btn btn-primary"
          onclick={play}
          disabled={loading}
          title="Play (Space)">Play</button
        >
      {/if}
      <button
        class="btn"
        onclick={next}
        disabled={$playbackStore.is_playing || loading || !canNext}
        title="Next (Right Arrow)">Next</button
      >
    </div>
  </div>
  {#if errorMsg}
    <div class="playback-error">{errorMsg}</div>
  {/if}
</div>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: 20px;
  }
  .transport-info {
    flex-shrink: 0;
    min-width: 120px;
  }
  .transport-progress {
    flex: 1;
    min-width: 0;
  }
  .playback-song {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
  }
  .playback-status {
    font-size: 14px;
    color: var(--text-muted);
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
    font-size: 12px;
    color: var(--text-dim);
  }
  .playback-error {
    font-size: 12px;
    color: var(--red);
    text-align: center;
    padding: 2px 0;
  }
  .controls {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }
</style>
