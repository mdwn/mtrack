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
    type SongFailure,
    type WaveformData,
  } from "../../lib/api/songs";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { untrack } from "svelte";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { showConfirm, showAlert } from "../../lib/dialog.svelte";
  import CreateSongDialog from "./CreateSongDialog.svelte";
  import ImportSongs from "./ImportSongs.svelte";
  import Waveform from "./Waveform.svelte";

  interface Props {
    initialSearch?: string;
    onSearchChange?: (query: string) => void;
  }

  let { initialSearch = "", onSearchChange }: Props = $props();

  let songs = $state<SongSummary[]>([]);
  let failures = $state<SongFailure[]>([]);
  let waveforms = $state<Record<string, number[]>>({});
  let loading = $state(true);
  let error = $state("");
  let showCreate = $state(false);
  let showImport = $state(false);
  let searchQuery = $state(untrack(() => initialSearch) || "");

  $effect(() => {
    onSearchChange?.(searchQuery);
  });

  let collapsedGroups = new SvelteSet<string>();

  type SongOrFailure = SongSummary | SongFailure;

  function isSongFailure(item: SongOrFailure): item is SongFailure {
    return "failed" in item && item.failed === true;
  }

  function getFilteredItems(): SongOrFailure[] {
    const all: SongOrFailure[] = [...songs, ...failures];
    if (!searchQuery.trim()) return all;
    const q = searchQuery.toLowerCase();
    return all.filter((s) => s.name.toLowerCase().includes(q));
  }

  interface SongGroup {
    directory: string;
    label: string;
    items: SongOrFailure[];
  }

  function groupItems(itemList: SongOrFailure[]): SongGroup[] {
    const groups = new SvelteMap<string, SongOrFailure[]>();
    for (const item of itemList) {
      const parts = item.base_dir.split("/");
      const dir = parts.length > 1 ? parts.slice(0, -1).join("/") : "";
      if (!groups.has(dir)) {
        groups.set(dir, []);
      }
      groups.get(dir)!.push(item);
    }

    const result: SongGroup[] = [];
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
        items: groups.get(key)!,
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
      const result = await fetchSongs();
      songs = result.songs;
      failures = result.failures;
      for (const song of songs) {
        fetchWaveform(song.name)
          .then((w) => {
            waveforms = { ...waveforms, [song.name]: compositePeaks(w) };
          })
          .catch(() => {});
      }
    } catch (e) {
      error =
        e instanceof Error ? e.message : get(t)("songs.failedToLoadSongs");
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
      !(await showConfirm(get(t)("songs.confirmDelete", { values: { name } }), {
        danger: true,
      }))
    )
      return;
    try {
      const res = await deleteSong(name);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        await showAlert(data?.error ?? `Delete failed (${res.status})`);
        return;
      }
      load();
    } catch (err) {
      await showAlert(err instanceof Error ? err.message : "Delete failed");
    }
  }
</script>

{#if showImport}
  <ImportSongs {onimported} oncancel={() => (showImport = false)} />
{:else}
  <div class="header">
    <h2>{$t("songs.title")}</h2>
    <div class="header-actions">
      <button
        class="btn btn-primary"
        onclick={() => {
          showImport = true;
          showCreate = false;
        }}
      >
        {$t("songs.importFromFilesystem")}
      </button>
      <button
        class="btn"
        onclick={() => {
          showCreate = !showCreate;
          showImport = false;
        }}
      >
        {showCreate ? $t("common.cancel") : $t("songs.newSong")}
      </button>
    </div>
  </div>

  {#if showCreate}
    <CreateSongDialog {oncreated} oncancel={() => (showCreate = false)} />
  {/if}

  {#if loading}
    <div class="status">{$t("songs.loadingSongs")}</div>
  {:else if error}
    <div class="status error">{error}</div>
  {:else if songs.length === 0 && failures.length === 0}
    <div class="status">
      {$t("songs.noSongs")}
    </div>
  {:else}
    {@const filteredItems = getFilteredItems()}
    {@const groups = groupItems(filteredItems)}
    {@const totalCount = songs.length + failures.length}
    <div class="search-bar">
      <input
        type="text"
        class="search-input"
        placeholder={$t("songs.searchPlaceholder")}
        bind:value={searchQuery}
      />
      {#if searchQuery}
        <button class="search-clear" onclick={() => (searchQuery = "")}
          >&#10005;</button
        >
      {/if}
      <span class="search-count"
        >{$t("songs.searchCount", {
          values: { filtered: filteredItems.length, total: totalCount },
        })}</span
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
            <span class="group-count">{group.items.length}</span>
          </button>
        {/if}
        {#if !collapsedGroups.has(group.directory)}
          {#each group.items as item (isSongFailure(item) ? `__failed__${item.name}` : item.name)}
            {#if isSongFailure(item)}
              <button
                class="song-row song-row-failed"
                onclick={() => navigate(item.name)}
              >
                <div class="song-main">
                  <span class="song-name">{item.name}</span>
                  <div class="song-badges">
                    <span class="badge failed">ERROR</span>
                  </div>
                </div>
                <div class="song-error" title={item.error}>
                  {item.error}
                </div>
              </button>
            {:else}
              <button class="song-row" onclick={() => navigate(item.name)}>
                <div class="song-main">
                  <span class="song-name">{item.name}</span>
                  <div class="song-badges">
                    {#if item.has_midi}
                      <span class="badge midi">MIDI</span>
                    {/if}
                    {#if item.lighting_files.length > 0}
                      <span class="badge lighting">LIGHT</span>
                    {/if}
                    {#if item.midi_dmx_files.length > 0}
                      <span class="badge midi-dmx">MIDI DMX</span>
                    {/if}
                  </div>
                </div>
                <div class="song-waveform">
                  <Waveform peaks={waveforms[item.name] ?? []} height={24} />
                </div>
                <div class="song-meta">
                  <span>{item.duration_display}</span>
                  <span
                    >{$t("songs.trackCount", {
                      values: { count: item.track_count },
                    })}</span
                  >
                </div>
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <span
                  class="song-delete"
                  role="button"
                  tabindex="-1"
                  title={$t("songs.removeFromRegistry")}
                  aria-label={$t("songs.removeFromRegistry")}
                  onclick={(e) => handleDelete(e, item.name)}>&#10005;</span
                >
              </button>
            {/if}
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
  .badge.failed {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .song-row-failed {
    border-color: rgba(239, 68, 68, 0.3);
  }
  .song-row-failed:hover {
    border-color: rgba(239, 68, 68, 0.5);
  }
  .song-row-failed .song-name {
    color: var(--text-muted);
  }
  .song-error {
    font-size: 12px;
    color: var(--red);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    grid-column: 2 / -1;
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
