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
      <li class:current={i === $playbackStore.playlist_position}>{song}</li>
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
    padding: 4px 8px;
    font-size: 13px;
    color: var(--text-muted);
    border-radius: 4px;
  }
  .playlist-songs li.current {
    background: rgba(94, 202, 234, 0.12);
    color: var(--text);
    font-weight: 500;
  }
</style>
