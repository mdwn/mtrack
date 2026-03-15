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
    type SongSummary,
    type WaveformData,
  } from "../../lib/api/songs";
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

  function getFilteredSongs(): SongSummary[] {
    if (!searchQuery.trim()) return songs;
    const q = searchQuery.toLowerCase();
    return songs.filter((s) => s.name.toLowerCase().includes(q));
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
    <div class="grid">
      {#each filteredSongs as song (song.name)}
        <button class="card song-card" onclick={() => navigate(song.name)}>
          <div class="song-header">
            <div class="song-name">{song.name}</div>
            <div class="badges">
              {#if song.has_midi}
                <span class="badge midi">MIDI</span>
              {/if}
              {#if song.has_lighting}
                <span class="badge lighting">LIGHT</span>
              {/if}
            </div>
          </div>
          <div class="song-waveform">
            <Waveform peaks={waveforms[song.name] ?? []} height={28} />
          </div>
          <div class="song-meta">
            <span>{song.duration_display}</span>
            <span
              >{song.track_count} track{song.track_count !== 1 ? "s" : ""}</span
            >
          </div>
        </button>
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
    font-size: 14px;
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
    font-size: 13px;
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
    font-size: 14px;
    padding: 4px;
    line-height: 1;
  }
  .search-clear:hover {
    color: var(--text);
  }
  .search-count {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 12px;
  }
  @media (max-width: 768px) {
    .grid {
      grid-template-columns: 1fr;
    }
    .header-actions {
      flex-direction: column;
      gap: 4px;
    }
  }
  .song-card {
    text-align: left;
    cursor: pointer;
    transition:
      background 0.15s,
      border-color 0.15s;
    font-family: var(--sans);
  }
  .song-card:hover {
    background: var(--bg-card-hover);
    border-color: var(--text-dim);
  }
  .song-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .song-name {
    font-size: 15px;
    font-weight: 500;
    color: var(--text);
  }
  .badges {
    display: flex;
    gap: 4px;
  }
  .badge {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.5px;
    padding: 2px 6px;
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
  .song-waveform {
    margin-bottom: 6px;
  }
  .song-meta {
    display: flex;
    gap: 12px;
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
