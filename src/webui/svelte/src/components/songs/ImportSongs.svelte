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
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import {
    browseDirectory,
    createSongInDirectory,
    bulkImportSongs,
    type BrowseEntry,
    type BulkImportResult,
  } from "../../lib/api/songs";

  interface Props {
    onimported: () => void;
    oncancel: () => void;
  }

  let { onimported, oncancel }: Props = $props();

  type Step = "browse" | "confirm" | "bulk-result";
  let step = $state<Step>("browse");

  // Browse state
  let currentPath = $state("/");
  let entries = $state<BrowseEntry[]>([]);
  let loading = $state(true);
  let browseError = $state("");
  let pathInput = $state("/");

  // Confirm state
  let songDir = $state("/");
  let songName = $state("");
  let createError = $state("");
  let creating = $state(false);

  // Bulk import state
  let bulkImporting = $state(false);
  let bulkResult = $state<BulkImportResult | null>(null);
  let bulkError = $state("");

  async function navigate(path?: string) {
    loading = true;
    browseError = "";
    try {
      const result = await browseDirectory(path);
      currentPath = result.path;
      pathInput = result.path;
      entries = result.entries;
    } catch (e) {
      browseError =
        e instanceof Error ? e.message : get(t)("fileBrowser.emptyDir");
    } finally {
      loading = false;
    }
  }

  navigate();

  function navigateToInput() {
    const trimmed = pathInput.trim();
    if (trimmed) navigate(trimmed);
  }

  let atRoot = $derived(currentPath === "/");

  function parentPath(): string {
    if (atRoot) return "/";
    const parts = currentPath.replace(/\/+$/, "").split("/");
    if (parts.length <= 2) return "/";
    parts.pop();
    return parts.join("/");
  }

  let dirEntries = $derived(entries.filter((e) => e.is_dir));
  let fileEntries = $derived(entries.filter((e) => !e.is_dir));
  let audioFiles = $derived(fileEntries.filter((e) => e.type === "audio"));
  let midiFiles = $derived(fileEntries.filter((e) => e.type === "midi"));
  let lightingFilesList = $derived(
    fileEntries.filter((e) => e.type === "lighting"),
  );
  let hasAudioFiles = $derived(audioFiles.length > 0);

  let breadcrumbs = $derived.by(() => {
    const crumbs: { name: string; path: string }[] = [{ name: "/", path: "/" }];
    if (currentPath !== "/") {
      const parts = currentPath.split("/").filter(Boolean);
      let acc = "";
      for (const part of parts) {
        acc += "/" + part;
        crumbs.push({ name: part, path: acc });
      }
    }
    return crumbs;
  });

  function selectDirectory() {
    songDir = currentPath;
    songName =
      currentPath === "/"
        ? "new-song"
        : (currentPath.split("/").filter(Boolean).pop() ?? "new-song");
    createError = "";
    step = "confirm";
  }

  let canCreate = $derived(songName.trim().length > 0);

  async function createSong() {
    creating = true;
    createError = "";
    try {
      const res = await createSongInDirectory(songDir, songName.trim());
      if (res.status === 409) {
        createError = get(t)("songs.import.songExists");
        return;
      }
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        createError =
          data?.error ?? data?.errors?.[0] ?? `Failed (${res.status})`;
        return;
      }
      onimported();
    } catch (e) {
      createError =
        e instanceof Error ? e.message : get(t)("songs.create.failed");
    } finally {
      creating = false;
    }
  }

  let canBulkImport = $derived(dirEntries.length > 0);

  async function handleBulkImport() {
    bulkImporting = true;
    bulkError = "";
    bulkResult = null;
    try {
      bulkResult = await bulkImportSongs(currentPath);
      step = "bulk-result";
    } catch (e) {
      bulkError = e instanceof Error ? e.message : "Bulk import failed";
    } finally {
      bulkImporting = false;
    }
  }

  function typeIcon(type: string): string {
    switch (type) {
      case "directory":
        return "\uD83D\uDCC1";
      case "audio":
        return "\uD83C\uDFB5";
      case "midi":
        return "\uD83C\uDFB9";
      case "lighting":
        return "\uD83D\uDCA1";
      default:
        return "\uD83D\uDCC4";
    }
  }
</script>

{#if step === "browse"}
  <div class="import-section">
    <div class="import-header">
      <h3>{$t("songs.import.title")}</h3>
      <p class="hint">
        {$t("songs.import.hint")}
      </p>
    </div>
    <div class="browser">
      <div class="browser-header">
        <div class="path-bar">
          <input
            class="input path-input"
            type="text"
            bind:value={pathInput}
            onkeydown={(e) => {
              if (e.key === "Enter") navigateToInput();
            }}
          />
          <button class="btn" onclick={navigateToInput}
            >{$t("common.go")}</button
          >
        </div>
        <div class="breadcrumbs">
          {#each breadcrumbs as crumb, i (crumb.path)}
            {#if i > 0}<span class="sep">/</span>{/if}
            <button class="crumb" onclick={() => navigate(crumb.path)}
              >{crumb.name}</button
            >
          {/each}
        </div>
      </div>

      <div class="browser-body">
        {#if loading}
          <div class="browser-status">{$t("common.loading")}</div>
        {:else if browseError}
          <div class="browser-status error">{browseError}</div>
        {:else}
          <div class="entry-list">
            {#if !atRoot}
              <button class="entry" onclick={() => navigate(parentPath())}>
                <span class="entry-icon">⬆️</span>
                <span class="entry-name">..</span>
              </button>
            {/if}
            {#each dirEntries as entry (entry.path)}
              <button class="entry dir" onclick={() => navigate(entry.path)}>
                <span class="entry-icon">{typeIcon(entry.type)}</span>
                <span class="entry-name">{entry.name}</span>
              </button>
            {/each}
            {#each fileEntries as entry (entry.path)}
              <div class="entry file-preview">
                <span class="entry-icon">{typeIcon(entry.type)}</span>
                <span class="entry-name">{entry.name}</span>
                <span class="entry-type">{entry.type}</span>
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <div class="browser-footer">
        <div class="footer-info">
          {#if hasAudioFiles}
            <span
              >{$t("songs.import.audioFileCount", {
                values: { count: audioFiles.length },
              })}</span
            >
            {#if midiFiles.length > 0}<span>
                &middot; {$t("songs.import.midiFileCount", {
                  values: { count: midiFiles.length },
                })}</span
              >{/if}
            {#if lightingFilesList.length > 0}<span>
                &middot; {$t("songs.import.lightingFileCount", {
                  values: { count: lightingFilesList.length },
                })}</span
              >{/if}
          {:else}
            <span class="hint">{$t("songs.import.noAudio")}</span>
          {/if}
        </div>
        <div class="footer-actions">
          <button class="btn" onclick={oncancel}>{$t("common.cancel")}</button>
          {#if canBulkImport}
            <button
              class="btn"
              onclick={handleBulkImport}
              disabled={bulkImporting}
            >
              {bulkImporting
                ? $t("songs.import.importing")
                : $t("songs.import.importAll", {
                    values: { count: dirEntries.length },
                  })}
            </button>
          {/if}
          <button
            class="btn btn-primary"
            onclick={selectDirectory}
            disabled={!hasAudioFiles}
          >
            {$t("songs.import.useThisDir")}
          </button>
        </div>
        {#if bulkError}
          <div class="error-msg" style="margin-top: 6px">{bulkError}</div>
        {/if}
      </div>
    </div>
  </div>
{:else if step === "confirm"}
  <div class="card import-section">
    <div class="card-header">
      <span class="card-title">{$t("songs.import.createSong")}</span>
    </div>
    <p class="hint">
      {$t("songs.import.songGenHint", {
        values: {
          dir: songDir,
          count: audioFiles.length,
          extra:
            (midiFiles.length > 0 ? ", MIDI" : "") +
            (lightingFilesList.length > 0 ? ", lighting" : ""),
        },
      })}
    </p>

    <label class="field">
      <span class="field-label">{$t("songs.import.songName")}</span>
      <input class="input field-input" type="text" bind:value={songName} />
    </label>

    {#if createError}
      <div class="error-msg">{createError}</div>
    {/if}

    <div class="configure-actions">
      <button class="btn" onclick={() => (step = "browse")}
        >{$t("common.back")}</button
      >
      <button class="btn" onclick={oncancel}>{$t("common.cancel")}</button>
      <button
        class="btn btn-primary"
        onclick={createSong}
        disabled={!canCreate || creating}
      >
        {creating ? $t("common.creating") : $t("songs.import.createSong")}
      </button>
    </div>
  </div>
{:else if step === "bulk-result" && bulkResult}
  <div class="import-section">
    <div class="import-header">
      <h3>{$t("songs.import.bulkComplete")}</h3>
    </div>
    <div class="bulk-result">
      {#if bulkResult.created.length > 0}
        <div class="result-group">
          <span class="result-label created"
            >{$t("songs.import.created", {
              values: { count: bulkResult.created.length },
            })}</span
          >
          <div class="result-list">
            {#each bulkResult.created as name (name)}
              <span class="result-item">{name}</span>
            {/each}
          </div>
        </div>
      {/if}
      {#if bulkResult.skipped.length > 0}
        <div class="result-group">
          <span class="result-label skipped"
            >{$t("songs.import.skipped", {
              values: { count: bulkResult.skipped.length },
            })}</span
          >
          <div class="result-list">
            {#each bulkResult.skipped as name (name)}
              <span class="result-item dimmed">{name}</span>
            {/each}
          </div>
        </div>
      {/if}
      {#if bulkResult.failed.length > 0}
        <div class="result-group">
          <span class="result-label failed"
            >{$t("songs.import.failed", {
              values: { count: bulkResult.failed.length },
            })}</span
          >
          <div class="result-list">
            {#each bulkResult.failed as item (item.name)}
              <span class="result-item error">{item.name}: {item.error}</span>
            {/each}
          </div>
        </div>
      {/if}
      {#if bulkResult.created.length === 0 && bulkResult.failed.length === 0}
        <p class="hint">
          {$t("songs.import.noNewSongs")}
        </p>
      {/if}
    </div>
    <div class="configure-actions">
      <button class="btn" onclick={() => (step = "browse")}
        >{$t("common.back")}</button
      >
      {#if bulkResult.created.length > 0}
        <button class="btn btn-primary" onclick={onimported}
          >{$t("common.done")}</button
        >
      {:else}
        <button class="btn" onclick={oncancel}>{$t("common.close")}</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .import-section {
    margin-bottom: 16px;
  }
  .import-header {
    margin-bottom: 12px;
  }
  .import-header h3 {
    font-size: 16px;
    font-weight: 600;
    margin-bottom: 4px;
  }
  .hint {
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 8px;
  }
  .browser {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    overflow: hidden;
  }
  .browser-header {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .path-bar {
    display: flex;
    gap: 6px;
    margin-bottom: 6px;
  }
  .path-input {
    flex: 1;
    font-family: var(--mono);
    font-size: 13px;
  }
  .breadcrumbs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    align-items: center;
  }
  .crumb {
    background: none;
    border: none;
    color: var(--accent);
    font-size: 13px;
    cursor: pointer;
    padding: 1px 3px;
    border-radius: 3px;
    font-family: var(--mono);
  }
  .crumb:hover {
    background: rgba(94, 202, 234, 0.1);
  }
  .sep {
    color: var(--text-dim);
    font-size: 13px;
  }
  .browser-body {
    flex: 1;
    min-height: 200px;
    max-height: 400px;
    overflow-y: auto;
  }
  .browser-status {
    padding: 32px 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 14px;
  }
  .browser-status.error {
    color: var(--red);
  }
  .entry-list {
    display: flex;
    flex-direction: column;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border: none;
    background: none;
    color: var(--text);
    font-size: 14px;
    font-family: var(--sans);
    cursor: pointer;
    text-align: left;
    transition: background 0.1s;
  }
  .entry:hover {
    background: var(--bg-card-hover);
  }
  .entry.dir .entry-name {
    color: var(--accent);
    font-weight: 500;
  }
  .file-preview {
    cursor: default;
    opacity: 0.7;
  }
  .file-preview:hover {
    background: none;
  }
  .entry-icon {
    flex: 0 0 20px;
    text-align: center;
    font-size: 15px;
  }
  .entry-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry-type {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim);
    flex: 0 0 auto;
  }
  .browser-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    gap: 8px;
  }
  .footer-info {
    font-size: 13px;
    color: var(--text-muted);
  }
  .footer-actions {
    display: flex;
    gap: 6px;
  }
  .field {
    display: flex;
    flex-direction: column;
    margin-bottom: 12px;
  }
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim);
    margin-bottom: 3px;
  }
  .field-input {
    font-size: 14px;
    padding: 6px 10px;
  }
  .error-msg {
    font-size: 13px;
    color: var(--red);
    margin-bottom: 8px;
  }
  .configure-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .bulk-result {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-card);
    padding: 12px 16px;
    max-height: 400px;
    overflow-y: auto;
  }
  .result-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .result-label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .result-label.created {
    color: var(--green);
  }
  .result-label.skipped {
    color: var(--text-muted);
  }
  .result-label.failed {
    color: var(--red);
  }
  .result-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .result-item {
    font-size: 13px;
    font-family: var(--mono);
    background: rgba(255, 255, 255, 0.04);
    padding: 2px 8px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .result-item.dimmed {
    opacity: 0.5;
  }
  .result-item.error {
    color: var(--red);
    border-color: rgba(239, 68, 68, 0.3);
  }
</style>
