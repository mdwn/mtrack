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
  import { playbackStore } from "../../lib/ws/stores";
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
  <div class="page__head">
    <div>
      <h1 class="page__title">{$t("songs.title")}</h1>
    </div>
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
    <div class="card search-card">
      <div class="search-bar">
        <span class="search-icon" aria-hidden="true">
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><circle cx="11" cy="11" r="7" /><path d="M21 21l-4.3-4.3" /></svg
          >
        </span>
        <input
          type="search"
          class="search-input"
          placeholder={$t("songs.searchPlaceholder")}
          bind:value={searchQuery}
        />
        {#if searchQuery}
          <button
            class="search-clear"
            onclick={() => (searchQuery = "")}
            aria-label={$t("common.cancel")}>×</button
          >
        {/if}
        <span class="search-count mono"
          >{$t("songs.searchCount", {
            values: { filtered: filteredItems.length, total: totalCount },
          })}</span
        >
      </div>
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
              aria-hidden="true">▾</span
            >
            <span class="overline group-label">{group.label}</span>
            <span class="mono group-count">{group.items.length}</span>
          </button>
        {/if}
        {#if !collapsedGroups.has(group.directory)}
          <div class="card group-card">
            {#each group.items as item, idx (isSongFailure(item) ? `__failed__${item.name}` : item.name)}
              {#if isSongFailure(item)}
                <button
                  class="song-row song-row--failed"
                  class:song-row--first={idx === 0}
                  onclick={() => navigate(item.name)}
                >
                  <div class="song-row__main">
                    <span class="song-row__name">{item.name}</span>
                    <div class="song-row__badges">
                      <span class="badge badge--trigger">ERROR</span>
                    </div>
                  </div>
                  <div class="song-row__error" title={item.error}>
                    {item.error}
                  </div>
                </button>
              {:else}
                <div
                  class="song-row"
                  class:song-row--first={idx === 0}
                  onclick={() => navigate(item.name)}
                  onkeydown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      navigate(item.name);
                    }
                  }}
                  tabindex="0"
                  role="link"
                >
                  <div class="song-row__main">
                    <span class="song-row__name">{item.name}</span>
                    <div class="song-row__badges">
                      {#if item.has_midi}
                        <span class="badge badge--midi">MIDI</span>
                      {/if}
                      {#if item.lighting_files.length > 0}
                        <span class="badge badge--light">LIGHT</span>
                      {/if}
                      {#if item.midi_dmx_files.length > 0}
                        <span class="badge badge--dmx">MIDI DMX</span>
                      {/if}
                    </div>
                  </div>
                  <div class="song-row__waveform">
                    <Waveform peaks={waveforms[item.name] ?? []} height={24} />
                  </div>
                  <div class="mono song-row__meta">
                    <span>{item.duration_display}</span>
                    <span class="song-row__tracks"
                      >{$t("songs.trackCount", {
                        values: { count: item.track_count },
                      })}</span
                    >
                  </div>
                  <button
                    class="song-row__delete"
                    title={$playbackStore.locked
                      ? $t("common.locked")
                      : $t("songs.removeFromRegistry")}
                    aria-label={$t("songs.removeFromRegistry")}
                    disabled={$playbackStore.locked}
                    onclick={(e) => handleDelete(e, item.name)}>×</button
                  >
                  <span class="song-row__chevron" aria-hidden="true">
                    <svg
                      width="14"
                      height="14"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.7"
                      stroke-linecap="round"
                      stroke-linejoin="round"><path d="M9 6l6 6-6 6" /></svg
                    >
                  </span>
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      {/each}
    </div>
  {/if}
{/if}

<style>
  .header-actions {
    display: flex;
    gap: 8px;
  }
  .status {
    text-align: center;
    padding: 48px 16px;
    color: var(--nc-fg-2);
    font-size: 15px;
  }
  .status.error {
    color: var(--nc-error);
  }

  .search-card {
    margin-bottom: 16px;
    padding: 14px 16px;
  }
  .search-bar {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .search-icon {
    display: inline-flex;
    color: var(--nc-fg-3);
  }
  .search-input {
    flex: 1;
    padding: 8px 10px;
    font-size: 14px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: var(--nc-fg-1);
  }
  .search-input::-webkit-search-cancel-button {
    display: none;
  }
  .search-input:focus {
    outline: none;
    border-color: transparent;
    box-shadow: none;
  }
  .search-clear {
    background: none;
    border: none;
    color: var(--nc-fg-3);
    cursor: pointer;
    font-size: 18px;
    padding: 0 4px;
    line-height: 1;
  }
  .search-clear:hover {
    color: var(--nc-fg-1);
  }
  .search-count {
    color: var(--nc-fg-3);
    white-space: nowrap;
  }

  .song-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .group-card {
    padding: 0;
    overflow: hidden;
  }
  .group-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 4px 0;
    margin-top: 6px;
    background: none;
    border: none;
    cursor: pointer;
    font: inherit;
    color: var(--nc-fg-3);
  }
  .group-header:hover {
    color: var(--nc-fg-1);
  }
  .group-chevron {
    font-size: 11px;
    transition: transform 0.15s var(--nc-ease);
    flex-shrink: 0;
  }
  .group-chevron.collapsed {
    transform: rotate(-90deg);
  }
  .group-label {
    color: var(--nc-fg-3);
  }
  .group-count {
    color: var(--nc-fg-4);
    font-size: 11px;
  }

  .song-row {
    display: grid;
    grid-template-columns: 1fr minmax(120px, 220px) auto auto auto;
    align-items: center;
    gap: 16px;
    padding: 0 16px;
    height: 56px;
    border-top: 1px solid var(--card-border);
    cursor: pointer;
    background: transparent;
    color: var(--nc-fg-1);
    transition: background var(--nc-dur-fast) var(--nc-ease);
  }
  .song-row.song-row--first {
    border-top: none;
  }
  .song-row:hover {
    background: var(--nc-bg-2);
  }
  .song-row__main {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .song-row__name {
    font-family: var(--nc-font-sans);
    font-size: 14px;
    font-weight: 500;
    color: var(--nc-fg-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .song-row__badges {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }
  .song-row__waveform {
    min-width: 0;
    color: var(--nc-cyan-400);
  }
  .song-row__meta {
    display: flex;
    gap: 14px;
    color: var(--nc-fg-3);
    white-space: nowrap;
    flex-shrink: 0;
    font-size: 12px;
  }
  .song-row__tracks {
    color: var(--nc-fg-4);
  }
  .song-row__delete {
    opacity: 0;
    color: var(--nc-fg-3);
    cursor: pointer;
    font-size: 18px;
    padding: 2px 6px;
    border-radius: 4px;
    border: none;
    background: none;
    font: inherit;
    line-height: 1;
    transition:
      opacity 0.15s,
      background 0.15s,
      color 0.15s;
  }
  .song-row__delete:focus-visible {
    opacity: 1;
  }
  .song-row:hover .song-row__delete {
    opacity: 1;
  }
  .song-row__delete:hover {
    background: rgba(232, 75, 75, 0.15);
    color: var(--nc-error);
  }
  .song-row__chevron {
    display: none;
    color: var(--nc-fg-3);
  }
  .song-row--failed {
    background: rgba(232, 75, 75, 0.04);
  }
  .song-row--failed .song-row__name {
    color: var(--nc-fg-2);
  }
  .song-row__error {
    grid-column: 2 / -1;
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-error);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 720px) {
    .header-actions {
      width: 100%;
    }
    .header-actions .btn {
      flex: 1;
    }
    .song-row {
      grid-template-columns: 1fr auto auto;
      padding: 0 12px;
    }
    .song-row__waveform,
    .song-row__meta,
    .song-row__delete {
      display: none;
    }
    .song-row__chevron {
      display: inline-flex;
    }
  }
</style>
