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
  import {
    fetchSongs,
    fetchWaveform,
    deleteSong,
    type SongSummary,
    type WaveformData,
  } from "../../lib/api/songs";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import CreateSongDialog from "./CreateSongDialog.svelte";
  import ImportSongs from "./ImportSongs.svelte";
  import Waveform from "./Waveform.svelte";

  let songs = $state<SongSummary[]>([]);
  let waveforms = $state<Record<string, number[]>>({});
  let loading = $state(true);
  let error = $state("");
  let showCreate = $state(false);
  let showImport = $state(false);
  let searchQuery = $state("");
  let collapsedGroups = new SvelteSet<string>();

  function getFilteredSongs(): SongSummary[] {
    if (!searchQuery.trim()) return songs;
    const q = searchQuery.toLowerCase();
    return songs.filter((s) => s.name.toLowerCase().includes(q));
  }

  interface SongGroup {
    directory: string;
    label: string;
    songs: SongSummary[];
  }

  function groupSongs(songList: SongSummary[]): SongGroup[] {
    const groups = new SvelteMap<string, SongSummary[]>();
    for (const song of songList) {
      // base_dir is the relative path to the song directory.
      // The parent of that is the grouping directory.
      const parts = song.base_dir.split("/");
      // Remove the last component (the song's own directory name).
      const dir = parts.length > 1 ? parts.slice(0, -1).join("/") : "";
      if (!groups.has(dir)) {
        groups.set(dir, []);
      }
      groups.get(dir)!.push(song);
    }

    const result: SongGroup[] = [];
    // Sort group keys: root ("") first, then alphabetically.
    const keys = [...groups.keys()].sort((a, b) => {
      if (a === "" && b === "") return 0;
      if (a === "") return -1;
      if (b === "") return 1;
      return a.localeCompare(b);
    });

    for (const key of keys) {
      result.push({
        directory: key,
        label: key || "",
        songs: groups.get(key)!,
      });
    }
    return result;
  }

  function toggleGroup(dir: string) {
    if (collapsedGroups.has(dir)) {
      collapsedGroups.delete(dir);
    } else {
      collapsedGroups.add(dir);
    }
  }

  function compositePeaks(waveform: WaveformData): number[] {
    if (waveform.tracks.length === 0) return [];
    if (waveform.tracks.length === 1) return waveform.tracks[0].peaks;
    const len = waveform.tracks[0].peaks.length;
    const result = new Array<number>(len);
    for (let i = 0; i < len; i++) {
      let max = 0;
      for (const t of waveform.tracks) {
        if (i < t.peaks.length && t.peaks[i] > max) max = t.peaks[i];
      }
      result[i] = max;
    }
    return result;
  }

  async function load() {
    loading = true;
    error = "";
    try {
      songs = await fetchSongs();
      for (const song of songs) {
        fetchWaveform(song.name)
          .then((w) => {
            waveforms = { ...waveforms, [song.name]: compositePeaks(w) };
          })
          .catch(() => {});
      }
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load songs";
    } finally {
      loading = false;
    }
  }

  load();

  function navigate(name: string) {
    window.location.hash = `#/songs/${encodeURIComponent(name)}`;
  }

  function oncreated(name: string) {
    showCreate = false;
    window.location.hash = `#/songs/${encodeURIComponent(name)}`;
  }

  function onimported() {
    showImport = false;
    load();
  }

  async function handleDelete(e: MouseEvent, name: string) {
    e.stopPropagation();
    if (
      !confirm(
        `Remove "${name}" from the song registry?\n\nThis only deletes song.yaml — audio and other files are kept.`,
      )
    )
      return;
    try {
      const res = await deleteSong(name);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        alert(data?.error ?? `Delete failed (${res.status})`);
        return;
      }
      load();
    } catch (err) {
      alert(err instanceof Error ? err.message : "Delete failed");
    }
  }
</script>

{#if showImport}
  <ImportSongs {onimported} oncancel={() => (showImport = false)} />
{:else}
  <div class="header">
    <h2>Songs</h2>
    <div class="header-actions">
      <button
        class="btn"
        onclick={() => {
          showImport = true;
          showCreate = false;
        }}
      >
        Import from Filesystem
      </button>
      <button
        class="btn btn-primary"
        onclick={() => {
          showCreate = !showCreate;
          showImport = false;
        }}
      >
        {showCreate ? "Cancel" : "New Song"}
      </button>
    </div>
  </div>

  {#if showCreate}
    <CreateSongDialog {oncreated} oncancel={() => (showCreate = false)} />
  {/if}

  {#if loading}
    <div class="status">Loading songs...</div>
  {:else if error}
    <div class="status error">{error}</div>
  {:else if songs.length === 0}
    <div class="status">
      No songs yet. Create one or import audio files from the filesystem.
    </div>
  {:else}
    {@const filteredSongs = getFilteredSongs()}
    {@const groups = groupSongs(filteredSongs)}
    <div class="search-bar">
      <input
        type="text"
        class="search-input"
        placeholder="Search songs..."
        bind:value={searchQuery}
      />
      {#if searchQuery}
        <button class="search-clear" onclick={() => (searchQuery = "")}
          >&#10005;</button
        >
      {/if}
      <span class="search-count"
        >{filteredSongs.length} of {songs.length} songs</span
      >
    </div>
    <div class="song-list">
      {#each groups as group (group.directory)}
        {#if group.label}
          <button
            class="group-header"
            onclick={() => toggleGroup(group.directory)}
          >
            <span
              class="group-chevron"
              class:collapsed={collapsedGroups.has(group.directory)}
              >&#9662;</span
            >
            <span class="group-label">{group.label}</span>
            <span class="group-count">{group.songs.length}</span>
          </button>
        {/if}
        {#if !collapsedGroups.has(group.directory)}
          {#each group.songs as song (song.name)}
            <button class="song-row" onclick={() => navigate(song.name)}>
              <div class="song-main">
                <span class="song-name">{song.name}</span>
                <div class="song-badges">
                  {#if song.has_midi}
                    <span class="badge midi">MIDI</span>
                  {/if}
                  {#if song.lighting_files.length > 0}
                    <span class="badge lighting">LIGHT</span>
                  {/if}
                  {#if song.legacy_lighting_files.length > 0}
                    <span class="badge midi-dmx">MIDI DMX</span>
                  {/if}
                </div>
              </div>
              <div class="song-waveform">
                <Waveform peaks={waveforms[song.name] ?? []} height={24} />
              </div>
              <div class="song-meta">
                <span>{song.duration_display}</span>
                <span
                  >{song.track_count} track{song.track_count !== 1
                    ? "s"
                    : ""}</span
                >
              </div>
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <span
                class="song-delete"
                role="button"
                tabindex="-1"
                title="Remove from registry"
                onclick={(e) => handleDelete(e, song.name)}>&#10005;</span
              >
            </button>
          {/each}
        {/if}
      {/each}
    </div>
  {/if}
{/if}

<style>
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }
  .header h2 {
    font-size: 18px;
    font-weight: 600;
  }
  .header-actions {
    display: flex;
    gap: 8px;
  }
  .status {
    text-align: center;
    padding: 48px 16px;
    color: var(--text-muted);
    font-size: 15px;
  }
  .status.error {
    color: var(--red);
  }
  .search-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 12px;
  }
  .search-input {
    flex: 1;
    padding: 6px 10px;
    font-size: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text);
    outline: none;
  }
  .search-input:focus {
    border-color: var(--accent);
  }
  .search-clear {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 15px;
    padding: 4px;
    line-height: 1;
  }
  .search-clear:hover {
    color: var(--text);
  }
  .search-count {
    font-size: 13px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .song-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .group-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    margin-top: 8px;
    background: none;
    border: none;
    cursor: pointer;
    font: inherit;
    color: var(--text-muted);
  }
  .group-header:first-child {
    margin-top: 0;
  }
  .group-header:hover {
    color: var(--text);
  }
  .group-chevron {
    font-size: 11px;
    transition: transform 0.15s;
    flex-shrink: 0;
  }
  .group-chevron.collapsed {
    transform: rotate(-90deg);
  }
  .group-label {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.3px;
    text-transform: uppercase;
  }
  .group-count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .song-delete {
    opacity: 0;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 13px;
    padding: 2px 4px;
    border-radius: 3px;
    transition: opacity 0.15s;
  }
  .song-row:hover .song-delete {
    opacity: 1;
  }
  .song-delete:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .song-row {
    display: grid;
    grid-template-columns: 1fr minmax(80px, 160px) auto auto;
    align-items: center;
    gap: 12px;
    padding: 8px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    text-align: left;
    width: 100%;
    font-family: var(--sans);
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .song-row:hover {
    background: var(--bg-card-hover);
    border-color: var(--text-dim);
  }
  .song-main {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .song-name {
    font-size: 15px;
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .song-badges {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }
  .badge {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.5px;
    padding: 2px 5px;
    border-radius: 3px;
    line-height: 1;
  }
  .badge.midi {
    background: var(--blue);
    color: #fff;
  }
  .badge.lighting {
    background: var(--yellow);
    color: #000;
  }
  .badge.midi-dmx {
    background: var(--green-dim);
    color: var(--green);
  }
  .song-waveform {
    min-width: 0;
  }
  .song-meta {
    display: flex;
    gap: 10px;
    font-size: 13px;
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
  }
  @media (max-width: 768px) {
    .header-actions {
      flex-direction: column;
      gap: 4px;
    }
    .song-row {
      grid-template-columns: 1fr auto auto;
    }
    .song-waveform {
      display: none;
    }
  }
</style>
