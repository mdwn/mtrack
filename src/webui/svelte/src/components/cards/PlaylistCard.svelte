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
  import { t } from "svelte-i18n";

  async function switchPlaylist(name: string) {
    try {
      await playerClient.switchToPlaylist({ playlistName: name });
    } catch (e) {
      console.error("switchPlaylist failed:", e);
    }
  }

  async function jumpToSong(songName: string) {
    try {
      await playerClient.playSongFrom({ songName, startTime: {} });
    } catch (e) {
      console.error("jumpToSong failed:", e);
    }
  }

  let playlists = $derived($playbackStore.available_playlists);
  let currentPlaylist = $derived($playbackStore.playlist_name);
  let songCount = $derived($playbackStore.playlist_songs.length);
</script>

<section class="card playlist-card">
  <header class="playlist-card__head">
    <div>
      <div class="overline">{$t("playlist.title")}</div>
      <div class="playlist-card__title">
        {currentPlaylist || $t("common.default")}
        <span class="mono playlist-card__count"
          >· {$t("playlists.songs", { values: { count: songCount } })}</span
        >
      </div>
    </div>
    {#if playlists.length > 0}
      <select
        class="playlist-card__select"
        value={currentPlaylist}
        onchange={(e) => switchPlaylist(e.currentTarget.value)}
        aria-label={$t("playlist.title")}
      >
        {#each playlists as name (name)}
          <option value={name}>{name}</option>
        {/each}
      </select>
    {/if}
  </header>
  <ol class="playlist-card__list">
    {#each $playbackStore.playlist_songs as song, i (`${i}:${song}`)}
      <li>
        <button
          class="playlist-card__row"
          class:playlist-card__row--active={i ===
            $playbackStore.playlist_position}
          onclick={() => jumpToSong(song)}
        >
          <span class="mono playlist-card__num"
            >{String(i + 1).padStart(2, "0")}</span
          >
          <span class="playlist-card__name">{song}</span>
          {#if i === $playbackStore.playlist_position && $playbackStore.is_playing}
            <span
              class="playlist-card__playing-dot"
              aria-label={$t("playback.playing")}
            ></span>
          {/if}
        </button>
      </li>
    {/each}
  </ol>
</section>

<style>
  .playlist-card {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .playlist-card__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    gap: 12px;
    border-bottom: 1px solid var(--card-border);
  }
  .playlist-card__title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 16px;
    margin-top: 4px;
    color: var(--nc-fg-1);
  }
  .playlist-card__count {
    color: var(--nc-fg-3);
    font-weight: 400;
    font-size: 13px;
  }
  .playlist-card__select {
    width: 160px;
    padding: 6px 10px;
    font-size: 13px;
    border-radius: 8px;
    border: 1px solid var(--card-border);
    background: var(--card-bg);
    color: var(--nc-fg-1);
    cursor: pointer;
  }
  .playlist-card__list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    max-height: 360px;
    flex: 1;
  }
  .playlist-card__list li {
    border-bottom: 1px solid var(--card-border);
  }
  .playlist-card__list li:last-child {
    border-bottom: none;
  }
  .playlist-card__row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 0 20px;
    height: 48px;
    border: none;
    background: transparent;
    color: var(--nc-fg-1);
    text-align: left;
    cursor: pointer;
    font-family: var(--nc-font-sans);
    transition: background var(--nc-dur-fast) var(--nc-ease);
  }
  .playlist-card__row:hover {
    background: var(--nc-bg-2);
  }
  .playlist-card__row--active {
    background: rgba(94, 202, 234, 0.12);
    box-shadow: inset 3px 0 0 var(--nc-cyan-400);
  }
  .playlist-card__row--active:hover {
    background: rgba(94, 202, 234, 0.18);
  }
  .playlist-card__num {
    color: var(--nc-fg-3);
    min-width: 26px;
    font-size: 12px;
  }
  .playlist-card__row--active .playlist-card__num {
    color: var(--nc-cyan-600);
  }
  :global(.nc--dark) .playlist-card__row--active .playlist-card__num {
    color: var(--nc-cyan-300);
  }
  .playlist-card__name {
    flex: 1;
    font-weight: 500;
    font-size: 14px;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .playlist-card__playing-dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--nc-pink-400);
    box-shadow: 0 0 8px rgba(239, 96, 163, 0.7);
    flex: 0 0 auto;
    animation: ncPulsePink 1.6s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  @keyframes ncPulsePink {
    0%,
    100% {
      box-shadow: 0 0 0 0 rgba(239, 96, 163, 0.6);
    }
    50% {
      box-shadow: 0 0 0 6px rgba(239, 96, 163, 0);
    }
  }
</style>
