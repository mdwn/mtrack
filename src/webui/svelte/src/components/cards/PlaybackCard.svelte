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
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

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
      showError(get(t)("playback.error.play"));
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
      showError(get(t)("playback.error.stop"));
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
        showError(get(t)("playback.error.next"));
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
        showError(get(t)("playback.error.prev"));
      }
    } finally {
      loading = false;
    }
  }

  let loading = $state(false);

  function handleKeydown(e: KeyboardEvent) {
    // Only handle shortcuts on the dashboard page to avoid accidental
    // playback triggers when interacting with forms on other pages.
    if (
      window.location.hash !== "#/" &&
      window.location.hash !== "" &&
      window.location.hash !== "#"
    )
      return;

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

  let currentBeatInfo = $derived.by(() => {
    const grid = $playbackStore.beat_grid;
    if (!grid || grid.beats.length === 0) return null;
    const elapsed = $playbackStore.elapsed_ms / 1000;

    // Find current beat index.
    let beatIdx = 0;
    for (let i = grid.beats.length - 1; i >= 0; i--) {
      if (grid.beats[i] <= elapsed) {
        beatIdx = i;
        break;
      }
    }

    // Find current measure.
    let measureIdx = 0;
    for (let i = grid.measure_starts.length - 1; i >= 0; i--) {
      if (grid.measure_starts[i] <= beatIdx) {
        measureIdx = i;
        break;
      }
    }

    const beatInMeasure = beatIdx - grid.measure_starts[measureIdx] + 1;
    return { measure: measureIdx + 1, beat: beatInMeasure };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">{$t("playback.title")}</span>
  </div>
  <div class="transport">
    <div class="transport-info">
      <div class="playback-song">
        {$playbackStore.song_name || $t("playback.noSong")}
      </div>
      <div class="playback-status">
        {#if $playbackStore.is_playing}
          <span class="playing">{$t("playback.playing")}</span>
        {:else}
          <span class="stopped">{$t("playback.stopped")}</span>
        {/if}
        {#if $playbackStore.looping}
          <span class="loop-badge">LOOP</span>
        {/if}
        {#if currentBeatInfo}
          <span class="beat-info"
            >m{currentBeatInfo.measure} b{currentBeatInfo.beat}</span
          >
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
        aria-label={$t("playback.songProgress")}
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
        disabled={($playbackStore.is_playing && !$playbackStore.looping) ||
          loading ||
          !canPrev}
        title={$t("playback.prevTooltip")}>{$t("playback.prev")}</button
      >
      {#if $playbackStore.is_playing}
        <button
          class="btn btn-primary"
          onclick={stop}
          disabled={loading}
          title={$t("playback.stopTooltip")}>{$t("playback.stop")}</button
        >
      {:else}
        <button
          class="btn btn-primary"
          onclick={play}
          disabled={loading}
          title={$t("playback.playTooltip")}>{$t("playback.play")}</button
        >
      {/if}
      <button
        class="btn"
        onclick={next}
        disabled={($playbackStore.is_playing && !$playbackStore.looping) ||
          loading ||
          !canNext}
        title={$t("playback.nextTooltip")}>{$t("playback.next")}</button
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
  .beat-info {
    margin-left: 8px;
    font-family: var(--mono);
    color: var(--text-dim);
  }
  .loop-badge {
    margin-left: 8px;
    font-size: 10px;
    font-weight: 600;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--accent);
    color: var(--bg);
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
