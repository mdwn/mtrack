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
  /* eslint-disable @typescript-eslint/no-explicit-any */
  import {
    fetchPlaylists,
    fetchPlaylist,
    savePlaylist,
    deletePlaylist,
    activatePlaylist,
    type PlaylistInfo,
    type PlaylistData,
  } from "../lib/api/config";
  import { playbackStore } from "../lib/ws/stores";
  import { showConfirm } from "../lib/dialog.svelte";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();

  // Parse: #/playlists/PlaylistName
  let routePlaylist = $derived.by(() => {
    const prefix = "#/playlists/";
    if (currentHash.startsWith(prefix) && currentHash.length > prefix.length) {
      return decodeURIComponent(currentHash.slice(prefix.length));
    }
    return null;
  });

  let playlists = $state<PlaylistInfo[]>([]);
  let selected = $state<string | null>(null);
  let detail = $state<PlaylistData | null>(null);
  let editSongs = $state<string[]>([]);
  let loading = $state(true);
  let error = $state("");
  let saving = $state(false);
  let dirty = $state(false);
  let newName = $state("");
  let showNewInput = $state(false);
  let searchQuery = $state("");
  let confirmDelete = $state<string | null>(null);
  let dragIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  let availableSongs = $derived(
    detail
      ? detail.available_songs.filter(
          (s) =>
            !editSongs.includes(s) &&
            s.toLowerCase().includes(searchQuery.toLowerCase()),
        )
      : [],
  );

  async function loadPlaylists() {
    try {
      loading = true;
      error = "";
      playlists = await fetchPlaylists();
    } catch (e: any) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  async function selectPlaylist(name: string) {
    if (dirty && !(await showConfirm(get(t)("config.discardUnsaved")))) return;
    try {
      error = "";
      detail = await fetchPlaylist(name);
      selected = name;
      editSongs = [...detail.songs];
      dirty = false;
      searchQuery = "";
      window.location.hash = `#/playlists/${encodeURIComponent(name)}`;
    } catch (e: any) {
      error = e.message;
    }
  }

  async function handleSave() {
    if (!selected) return;
    if ($playbackStore.locked) {
      error = get(t)("common.locked");
      return;
    }
    try {
      saving = true;
      error = "";
      await savePlaylist(selected, editSongs);
      dirty = false;
      await loadPlaylists();
      // Re-fetch to get updated available_songs
      detail = await fetchPlaylist(selected);
    } catch (e: any) {
      error = e.message;
    } finally {
      saving = false;
    }
  }

  async function handleActivate(name: string) {
    try {
      error = "";
      await activatePlaylist(name);
      await loadPlaylists();
    } catch (e: any) {
      error = e.message;
    }
  }

  async function handleDelete(name: string) {
    if ($playbackStore.locked) {
      error = get(t)("common.locked");
      return;
    }
    try {
      error = "";
      await deletePlaylist(name);
      if (selected === name) {
        selected = null;
        detail = null;
        editSongs = [];
      }
      confirmDelete = null;
      await loadPlaylists();
    } catch (e: any) {
      error = e.message;
    }
  }

  async function handleCreate() {
    const name = newName.trim();
    if (!name) return;
    if ($playbackStore.locked) {
      error = get(t)("common.locked");
      return;
    }
    try {
      error = "";
      await savePlaylist(name, []);
      newName = "";
      showNewInput = false;
      await loadPlaylists();
      await selectPlaylist(name);
    } catch (e: any) {
      error = e.message;
    }
  }

  function addSong(song: string) {
    editSongs = [...editSongs, song];
    dirty = true;
  }

  function removeSong(index: number) {
    editSongs = editSongs.filter((_, i) => i !== index);
    dirty = true;
  }

  function moveSong(from: number, to: number) {
    if (to < 0 || to >= editSongs.length) return;
    const songs = [...editSongs];
    const [item] = songs.splice(from, 1);
    songs.splice(to, 0, item);
    editSongs = songs;
    dirty = true;
  }

  function handleDragStart(e: DragEvent, index: number) {
    dragIndex = index;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", String(index));
    }
  }

  function handleDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dragOverIndex = index;
  }

  function handleDrop(e: DragEvent, targetIndex: number) {
    e.preventDefault();
    if (dragIndex === null || dragIndex === targetIndex) return;

    const songs = [...editSongs];
    const [moved] = songs.splice(dragIndex, 1);
    songs.splice(targetIndex, 0, moved);
    editSongs = songs;
    dirty = true;
    dragIndex = null;
    dragOverIndex = null;
  }

  function handleDragEnd() {
    dragIndex = null;
    dragOverIndex = null;
  }

  loadPlaylists();

  // Auto-select playlist from URL after data loads
  $effect(() => {
    if (loading || !routePlaylist || selected === routePlaylist) return;
    const match = playlists.find((p) => p.name === routePlaylist);
    if (match && match.name !== "all_songs") {
      selectPlaylist(match.name);
    }
  });

  $effect(() => {
    if (dirty) {
      const handler = (e: BeforeUnloadEvent) => {
        e.preventDefault();
      };
      window.addEventListener("beforeunload", handler);
      return () => window.removeEventListener("beforeunload", handler);
    }
  });
</script>

<div class="playlist-editor">
  <div class="panel list-panel">
    <div class="panel-header">
      <h3>{$t("playlists.title")}</h3>
      <button class="btn btn-sm" onclick={() => (showNewInput = !showNewInput)}>
        {showNewInput ? $t("common.cancel") : $t("playlists.new")}
      </button>
    </div>

    {#if showNewInput}
      <div class="new-playlist-form">
        <input
          type="text"
          class="input"
          placeholder={$t("playlists.namePlaceholder")}
          bind:value={newName}
          onkeydown={(e) => e.key === "Enter" && handleCreate()}
        />
        <button class="btn btn-primary btn-sm" onclick={handleCreate}
          >{$t("common.create")}</button
        >
      </div>
    {/if}

    {#if loading}
      <p class="muted">
        <span class="spinner sm"></span>
        {$t("common.loading")}
      </p>
    {:else if playlists.length === 0}
      <p class="muted">{$t("playlists.noPlaylists")}</p>
    {:else}
      <ul class="playlist-list">
        {#each playlists.filter((p) => p.name !== "all_songs") as pl (pl.name)}
          <li class:selected={selected === pl.name}>
            <button
              class="playlist-item"
              onclick={() => selectPlaylist(pl.name)}
            >
              <span class="pl-name">{pl.name}</span>
              <span class="pl-count"
                >{$t("playlists.songs", {
                  values: {
                    count:
                      selected === pl.name ? editSongs.length : pl.song_count,
                  },
                })}</span
              >
            </button>
            <div class="pl-actions">
              {#if pl.is_active}
                <span class="badge">{$t("playlists.active")}</span>
              {:else}
                <button
                  class="btn-icon"
                  title={$t("playlists.activate")}
                  aria-label={$t("playlists.activate")}
                  onclick={() => handleActivate(pl.name)}
                >
                  &#9654;
                </button>
              {/if}
              {#if confirmDelete === pl.name}
                <button
                  class="btn-icon danger"
                  onclick={() => handleDelete(pl.name)}
                >
                  {$t("common.confirm")}
                </button>
                <button class="btn-icon" onclick={() => (confirmDelete = null)}>
                  {$t("common.cancel")}
                </button>
              {:else}
                <button
                  class="btn-icon"
                  title={$t("common.delete")}
                  aria-label={$t("common.delete")}
                  onclick={() => (confirmDelete = pl.name)}
                >
                  &#10005;
                </button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="panel detail-panel">
    {#if error}
      <div class="error-banner">
        {error}
        <button class="error-dismiss" onclick={() => (error = "")}
          >&#10005;</button
        >
      </div>
    {/if}

    {#if !selected}
      <p class="muted center">
        {$t("playlists.selectOrCreate")}
      </p>
    {:else if detail}
      <div class="panel-header">
        <h3>{selected}</h3>
        <button
          class="btn btn-primary btn-sm"
          onclick={handleSave}
          disabled={!dirty || saving}
        >
          {saving ? $t("common.saving") : $t("common.save")}
        </button>
      </div>

      <div class="song-columns">
        <div class="song-col">
          <h4>
            {$t("playlists.playlistSongs", {
              values: { count: editSongs.length },
            })}
          </h4>
          {#if editSongs.length === 0}
            <p class="muted">
              {$t("playlists.noSongsInPlaylist")}
            </p>
          {:else}
            <ul class="song-list">
              {#each editSongs as song, i (song + i)}
                <li
                  draggable="true"
                  ondragstart={(e) => handleDragStart(e, i)}
                  ondragover={(e) => handleDragOver(e, i)}
                  ondrop={(e) => handleDrop(e, i)}
                  ondragend={handleDragEnd}
                  class:drag-over={dragOverIndex === i}
                  style={dragIndex === i ? "opacity: 0.4" : ""}
                >
                  <span class="song-position">{i + 1}.</span>
                  <div class="reorder-btns">
                    <button
                      class="btn-icon small"
                      aria-label={$t("cue.moveUp")}
                      disabled={i === 0}
                      onclick={() => moveSong(i, i - 1)}>&#9650;</button
                    >
                    <button
                      class="btn-icon small"
                      aria-label={$t("cue.moveDown")}
                      disabled={i === editSongs.length - 1}
                      onclick={() => moveSong(i, i + 1)}>&#9660;</button
                    >
                  </div>
                  <span class="song-name">{song}</span>
                  <button
                    class="btn-icon"
                    title={$t("common.remove")}
                    aria-label={$t("common.remove")}
                    onclick={() => removeSong(i)}
                  >
                    &#10005;
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>

        <div class="song-col">
          <h4>{$t("playlists.availableSongs")}</h4>
          <input
            type="text"
            class="input"
            placeholder={$t("playlists.searchSongs")}
            bind:value={searchQuery}
          />
          <ul class="song-list available">
            {#each availableSongs as song (song)}
              <li>
                <span class="song-name">{song}</span>
                <button
                  class="btn-icon"
                  title={$t("common.add")}
                  aria-label={$t("common.add")}
                  onclick={() => addSong(song)}>+</button
                >
              </li>
            {/each}
          </ul>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .playlist-editor {
    display: flex;
    gap: 16px;
    height: calc(100vh - 120px);
  }
  .panel {
    border-radius: var(--radius-lg);
  }
  .list-panel {
    width: 300px;
    flex-shrink: 0;
  }
  .detail-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .new-playlist-form {
    display: flex;
    gap: 8px;
    margin-bottom: 12px;
  }
  .new-playlist-form .input {
    flex: 1;
  }
  .muted {
    color: var(--text-muted);
    font-size: 14px;
  }
  .center {
    text-align: center;
    margin-top: 40px;
  }
  /* Playlist list */
  .playlist-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .playlist-list li {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 6px;
    padding: 2px;
  }
  .playlist-list li.selected {
    background: rgba(94, 202, 234, 0.12);
  }
  .playlist-item {
    flex: 1;
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: none;
    border: none;
    color: var(--text);
    padding: 8px;
    font-size: 14px;
    cursor: pointer;
    border-radius: 4px;
    text-align: left;
  }
  .playlist-item:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.04);
  }
  .playlist-item:disabled {
    cursor: default;
  }
  .pl-count {
    color: var(--text-muted);
    font-size: 12px;
  }
  .pl-actions {
    display: flex;
    gap: 2px;
    align-items: center;
  }
  .badge {
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--accent);
    color: white;
  }
  /* Song columns */
  .song-columns {
    display: flex;
    gap: 16px;
    flex: 1;
    min-height: 0;
  }
  .song-col {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .song-col h4 {
    margin: 0 0 8px 0;
    font-size: 14px;
    color: var(--text-muted);
  }
  .song-col .input {
    margin-bottom: 8px;
  }
  .song-list {
    list-style: none;
    padding: 0;
    margin: 0;
    overflow-y: auto;
    flex: 1;
  }
  .song-list li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: 13px;
    border-radius: 4px;
  }
  .song-list li:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .song-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .song-position {
    color: var(--text-dim);
    min-width: 24px;
    text-align: right;
    font-size: 13px;
  }
  .reorder-btns {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .song-list li[draggable="true"] {
    cursor: grab;
  }
  .song-list li[draggable="true"]:active {
    cursor: grabbing;
  }
  .drag-over {
    border-top: 2px solid var(--accent);
  }

  @media (max-width: 768px) {
    .playlist-editor {
      flex-direction: column;
      height: auto;
    }
    .list-panel {
      width: 100%;
      max-height: 300px;
    }
    .song-columns {
      flex-direction: column;
    }
    .btn-icon.small {
      min-width: 44px;
      min-height: 44px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }
  }
</style>
