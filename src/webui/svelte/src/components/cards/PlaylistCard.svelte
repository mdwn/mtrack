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

  async function switchPlaylist(name: string) {
    try {
      await playerClient.switchToPlaylist({ playlistName: name });
    } catch (e) {
      console.error("switchPlaylist failed:", e);
    }
  }

  let isPlaylistMode = $derived($playbackStore.playlist_name === "playlist");
</script>

<div class="card">
  <div class="card-header">
    <span class="card-title">Playlist</span>
    <div class="btn-group">
      <button
        class="btn"
        class:active={isPlaylistMode}
        onclick={() => switchPlaylist("playlist")}>Playlist</button
      >
      <button
        class="btn"
        class:active={!isPlaylistMode}
        onclick={() => switchPlaylist("all_songs")}>All Songs</button
      >
    </div>
  </div>
  <div class="playlist-name">{$playbackStore.playlist_name}</div>
  <ul class="playlist-songs">
    {#each $playbackStore.playlist_songs as song, i (song)}
      <li class:current={i === $playbackStore.playlist_position}>{song}</li>
    {/each}
  </ul>
</div>

<style>
  .playlist-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    margin-bottom: 8px;
  }
  .playlist-songs {
    list-style: none;
    max-height: 200px;
    overflow-y: auto;
  }
  .playlist-songs li {
    padding: 4px 8px;
    font-size: 12px;
    color: var(--text-muted);
    border-radius: 4px;
  }
  .playlist-songs li.current {
    background: rgba(91, 91, 214, 0.15);
    color: var(--text);
    font-weight: 500;
  }
</style>
