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
  let slotIdCounter = 0;
  let editSlotIds = $state<number[]>([]);

  function assignSlotIds(songs: string[]) {
    editSlotIds = songs.map(() => slotIdCounter++);
  }

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
      assignSlotIds(editSongs);
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
        editSlotIds = [];
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
    editSlotIds = [...editSlotIds, slotIdCounter++];
    dirty = true;
  }

  function removeSong(index: number) {
    editSongs = editSongs.filter((_, i) => i !== index);
    editSlotIds = editSlotIds.filter((_, i) => i !== index);
    dirty = true;
  }

  function moveSong(from: number, to: number) {
    if (to < 0 || to >= editSongs.length) return;
    const songs = [...editSongs];
    const ids = [...editSlotIds];
    const [item] = songs.splice(from, 1);
    const [id] = ids.splice(from, 1);
    songs.splice(to, 0, item);
    ids.splice(to, 0, id);
    editSongs = songs;
    editSlotIds = ids;
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
    const ids = [...editSlotIds];
    const [moved] = songs.splice(dragIndex, 1);
    const [movedId] = ids.splice(dragIndex, 1);
    songs.splice(targetIndex, 0, moved);
    ids.splice(targetIndex, 0, movedId);
    editSongs = songs;
    editSlotIds = ids;
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
      <button
        class="btn btn-sm"
        disabled={$playbackStore.locked}
        title={$playbackStore.locked ? $t("common.locked") : null}
        onclick={() => (showNewInput = !showNewInput)}
      >
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
        <button
          class="btn btn-primary btn-sm"
          disabled={$playbackStore.locked}
          title={$playbackStore.locked ? $t("common.locked") : null}
          onclick={handleCreate}>{$t("common.create")}</button
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
                  title={$playbackStore.locked
                    ? $t("common.locked")
                    : $t("playlists.activate")}
                  aria-label={$t("playlists.activate")}
                  disabled={$playbackStore.locked}
                  onclick={() => handleActivate(pl.name)}
                >
                  &#9654;
                </button>
              {/if}
              {#if confirmDelete === pl.name}
                <button
                  class="btn-icon danger"
                  disabled={$playbackStore.locked}
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
                  title={$playbackStore.locked
                    ? $t("common.locked")
                    : $t("common.delete")}
                  aria-label={$t("common.delete")}
                  disabled={$playbackStore.locked}
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
        <div class="panel-header__actions">
          {#if dirty && !saving}
            <span class="dirty-flag">{$t("common.unsaved")}</span>
          {/if}
          <button
            class="btn btn-sm"
            class:btn-primary={dirty && !$playbackStore.locked}
            onclick={handleSave}
            disabled={!dirty || saving || $playbackStore.locked}
            title={$playbackStore.locked ? $t("common.locked") : null}
          >
            {saving ? $t("common.saving") : $t("common.save")}
          </button>
        </div>
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
              {#each editSongs as song, i (editSlotIds[i])}
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
                    title={$playbackStore.locked
                      ? $t("common.locked")
                      : $t("common.remove")}
                    aria-label={$t("common.remove")}
                    disabled={$playbackStore.locked}
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
                  title={$playbackStore.locked
                    ? $t("common.locked")
                    : $t("common.add")}
                  aria-label={$t("common.add")}
                  disabled={$playbackStore.locked}
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
    display: grid;
    grid-template-columns: 320px 1fr;
    gap: 24px;
    align-items: start;
  }
  .panel {
    border-radius: var(--nc-radius-md);
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    padding: 0;
    overflow: hidden;
  }
  .panel-header {
    padding: 16px 20px;
    border-bottom: 1px solid var(--card-border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .panel-header h3 {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 18px;
    margin: 0;
    color: var(--nc-fg-1);
  }
  .panel-header .btn-sm {
    flex-shrink: 0;
  }
  .panel-header__actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .dirty-flag {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-cyan-600);
  }
  :global(.nc--dark) .dirty-flag {
    color: var(--nc-cyan-300);
  }
  .panel > :global(*:not(.panel-header)) {
    padding-left: 20px;
    padding-right: 20px;
  }
  .panel > :global(:last-child) {
    padding-bottom: 20px;
  }
  .list-panel {
    flex-shrink: 0;
  }
  .detail-panel {
    display: flex;
    flex-direction: column;
    min-height: 480px;
  }
  .new-playlist-form {
    display: flex;
    gap: 8px;
    margin: 16px 0 4px;
  }
  .new-playlist-form .input {
    flex: 1;
  }
  .muted {
    color: var(--nc-fg-2);
    font-size: 14px;
    margin: 16px 0;
  }
  .center {
    text-align: center;
    margin-top: 60px;
  }
  /* Playlist list */
  .playlist-list {
    list-style: none;
    padding: 0;
    margin: 16px 0 0;
  }
  .playlist-list li {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 8px;
    padding: 4px;
    margin-bottom: 4px;
    transition: background var(--nc-dur-fast) var(--nc-ease);
  }
  .playlist-list li:hover {
    background: var(--nc-bg-2);
  }
  .playlist-list li.selected {
    background: rgba(94, 202, 234, 0.12);
    box-shadow: inset 3px 0 0 var(--nc-cyan-400);
  }
  .playlist-item {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    background: none;
    border: none;
    color: var(--nc-fg-1);
    padding: 8px;
    font-family: var(--nc-font-display);
    font-size: 15px;
    font-weight: 600;
    cursor: pointer;
    border-radius: 4px;
    text-align: left;
    min-width: 0;
    width: 100%;
  }
  .pl-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 100%;
  }
  .pl-count {
    color: var(--nc-fg-3);
    font-size: 12px;
    font-family: var(--nc-font-mono);
    font-weight: 400;
  }
  .pl-actions {
    display: flex;
    gap: 2px;
    align-items: center;
    flex-shrink: 0;
    padding-right: 4px;
  }
  .badge {
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    padding: 4px 10px;
    border-radius: 999px;
    background: var(--nc-cyan-400);
    color: var(--nc-ink);
    border: 1px solid var(--nc-cyan-500);
  }
  /* Song columns */
  .song-columns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 24px;
    margin-top: 16px;
    flex: 1;
    min-height: 0;
  }
  .song-col {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    background: var(--inset-bg);
    padding: 16px;
  }
  .song-col h4 {
    margin: 0 0 12px 0;
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--nc-fg-3);
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
    max-height: 480px;
  }
  .song-list li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    font-size: 13px;
    border-radius: 6px;
    transition: background var(--nc-dur-fast) var(--nc-ease);
  }
  .song-list li:hover {
    background: var(--nc-bg-2);
  }
  .song-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .song-position {
    color: var(--nc-fg-3);
    min-width: 26px;
    text-align: right;
    font-family: var(--nc-font-mono);
    font-size: 12px;
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
    border-top: 2px solid var(--nc-cyan-400);
  }

  @media (max-width: 900px) {
    .playlist-editor {
      grid-template-columns: 1fr;
    }
    .song-columns {
      grid-template-columns: 1fr;
    }
  }
  @media (max-width: 720px) {
    .btn-icon.small {
      min-width: 44px;
      min-height: 44px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }
  }
</style>
