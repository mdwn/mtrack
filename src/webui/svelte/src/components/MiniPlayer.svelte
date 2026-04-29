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
  import { playbackStore } from "../lib/ws/stores";
  import { playerClient } from "../lib/grpc/client";
  import { formatMs } from "../lib/util/format";
  import { t } from "svelte-i18n";

  let busy = $state(false);

  async function togglePlay() {
    busy = true;
    try {
      if ($playbackStore.is_playing) {
        await playerClient.stop({});
      } else {
        await playerClient.play({});
      }
    } catch (e) {
      console.error("toggle play failed:", e);
    } finally {
      busy = false;
    }
  }

  async function next() {
    busy = true;
    try {
      await playerClient.next({});
    } catch (e) {
      if (!(e instanceof ConnectError && e.code === Code.OutOfRange)) {
        console.error("next failed:", e);
      }
    } finally {
      busy = false;
    }
  }

  async function previous() {
    busy = true;
    try {
      await playerClient.previous({});
    } catch (e) {
      if (!(e instanceof ConnectError && e.code === Code.OutOfRange)) {
        console.error("previous failed:", e);
      }
    } finally {
      busy = false;
    }
  }

  function jumpToDashboard() {
    if (window.location.hash !== "#/" && window.location.hash !== "") {
      window.location.hash = "#/";
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

  let displayName = $derived(
    $playbackStore.song_name || $t("playback.noSong"),
  );

  let elapsed = $derived(formatMs($playbackStore.elapsed_ms));
  let total = $derived(formatMs($playbackStore.song_duration_ms));
</script>

<div class="miniplayer" role="region" aria-label={$t("playback.title")}>
  <span
    class="miniplayer__state"
    class:miniplayer__state--stopped={!$playbackStore.is_playing}
    aria-hidden="true"
  >
    {#if $playbackStore.is_playing}
      <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"
        ><rect x="6" y="5" width="4" height="14" rx="1" /><rect
          x="14"
          y="5"
          width="4"
          height="14"
          rx="1"
        /></svg
      >
    {:else}
      <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"
        ><path d="M8 5v14l11-7z" /></svg
      >
    {/if}
  </span>
  <button
    class="miniplayer__title-btn"
    onclick={jumpToDashboard}
    aria-label={$t("nav.dashboard")}
  >
    <div class="miniplayer__title">{displayName}</div>
    <div class="miniplayer__time">{elapsed} / {total}</div>
  </button>
  <button
    class="miniplayer__btn"
    onclick={previous}
    disabled={busy || !canPrev}
    aria-label={$t("playback.prev")}
  >
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"
      ><path d="M6 5h2v14H6zM20 5L9 12l11 7V5z" /></svg
    >
  </button>
  <button
    class="miniplayer__btn miniplayer__btn--play"
    class:miniplayer__btn--play--paused={!$playbackStore.is_playing}
    onclick={togglePlay}
    disabled={busy}
    aria-label={$playbackStore.is_playing
      ? $t("playback.stop")
      : $t("playback.play")}
  >
    {#if $playbackStore.is_playing}
      <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"
        ><rect x="6" y="5" width="4" height="14" rx="1" /><rect
          x="14"
          y="5"
          width="4"
          height="14"
          rx="1"
        /></svg
      >
    {:else}
      <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"
        ><path d="M8 5v14l11-7z" /></svg
      >
    {/if}
  </button>
  <button
    class="miniplayer__btn"
    onclick={next}
    disabled={busy || !canNext}
    aria-label={$t("playback.next")}
  >
    <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"
      ><path d="M16 5h2v14h-2zM4 5l11 7L4 19V5z" /></svg
    >
  </button>
</div>

<style>
  .miniplayer {
    display: none;
    position: fixed;
    left: 12px;
    right: 12px;
    bottom: max(12px, env(safe-area-inset-bottom, 12px));
    z-index: 60;
    border-radius: 14px;
    background: var(--card-bg);
    border: 1.5px solid var(--nc-ink);
    box-shadow:
      3px 3px 0 var(--nc-ink),
      0 18px 40px rgba(0, 0, 0, 0.35);
    padding: 10px 12px;
    align-items: center;
    gap: 12px;
    min-height: 56px;
  }
  :global(.nc--dark) .miniplayer {
    border-color: var(--nc-cyan-400);
    box-shadow:
      3px 3px 0 var(--nc-cyan-700),
      0 18px 40px rgba(0, 0, 0, 0.5);
  }
  .miniplayer__state {
    width: 28px;
    height: 28px;
    border-radius: 999px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--nc-pink-400);
    color: var(--nc-ink);
    flex: 0 0 auto;
  }
  .miniplayer__state--stopped {
    background: var(--nc-gray-300);
  }
  .miniplayer__title-btn {
    flex: 1;
    min-width: 0;
    text-align: left;
    background: transparent;
    border: none;
    color: inherit;
    padding: 0;
    cursor: pointer;
    line-height: 1.2;
  }
  .miniplayer__title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 14px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--nc-fg-1);
  }
  .miniplayer__time {
    font-family: var(--nc-font-mono);
    font-weight: 500;
    font-size: 11px;
    color: var(--nc-fg-3);
    margin-top: 3px;
  }
  .miniplayer__btn {
    width: 44px;
    height: 44px;
    border-radius: 10px;
    border: 1px solid var(--card-border);
    background: var(--card-bg);
    color: var(--nc-fg-1);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex: 0 0 auto;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .miniplayer__btn:hover:not(:disabled) {
    background: var(--nc-bg-2);
  }
  .miniplayer__btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .miniplayer__btn--play {
    background: var(--nc-pink-400);
    color: var(--nc-ink);
    border-color: var(--nc-pink-500);
  }
  .miniplayer__btn--play:hover:not(:disabled) {
    background: var(--nc-pink-500);
  }
  .miniplayer__btn--play--paused {
    background: var(--nc-cyan-400);
    color: var(--nc-ink);
    border-color: var(--nc-cyan-500);
  }
  .miniplayer__btn--play--paused:hover:not(:disabled) {
    background: var(--nc-cyan-500);
  }

  @media (max-width: 720px) {
    .miniplayer {
      display: flex;
    }
  }
</style>
