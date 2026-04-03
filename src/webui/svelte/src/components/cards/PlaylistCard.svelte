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
</script>

<div class="card">
  <div class="card-header">
    <span class="card-title">{$t("playlist.title")}</span>
    {#if playlists.length > 0}
      <select
        class="playlist-select"
        value={currentPlaylist}
        onchange={(e) => switchPlaylist(e.currentTarget.value)}
      >
        {#each playlists as name (name)}
          <option value={name}>{name}</option>
        {/each}
      </select>
    {/if}
  </div>
  <ul class="playlist-songs">
    {#each $playbackStore.playlist_songs as song, i (`${i}:${song}`)}
      <li class:current={i === $playbackStore.playlist_position}>
        <button class="playlist-song-btn" onclick={() => jumpToSong(song)}
          >{song}</button
        >
      </li>
    {/each}
  </ul>
</div>

<style>
  .playlist-select {
    font-size: 13px;
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text);
    cursor: pointer;
  }
  .playlist-songs {
    list-style: none;
    max-height: 200px;
    overflow-y: auto;
  }
  .playlist-songs li {
    border-radius: 4px;
    transition:
      background 0.15s,
      color 0.15s;
  }
  .playlist-song-btn {
    background: none;
    border: none;
    font: inherit;
    color: var(--text-muted);
    text-align: left;
    display: block;
    width: 100%;
    padding: 4px 8px;
    font-size: var(--text-sm);
    cursor: pointer;
    box-sizing: border-box;
    border-radius: 4px;
  }
  .playlist-songs li:hover {
    background: rgba(255, 255, 255, 0.05);
  }
  .playlist-songs li:hover .playlist-song-btn {
    color: var(--text);
  }
  .playlist-songs li.current {
    background: var(--accent-subtle);
    color: var(--text);
    font-weight: 500;
    border-left: 3px solid var(--accent);
    padding-left: 0;
  }
  .playlist-songs li.current .playlist-song-btn {
    padding-left: 5px;
  }
  .playlist-songs li.current:hover {
    background: var(--accent-subtle-hover);
  }
</style>
