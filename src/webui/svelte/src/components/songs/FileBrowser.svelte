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
  import { untrack } from "svelte";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { SvelteSet } from "svelte/reactivity";
  import { browseDirectory, type BrowseEntry } from "../../lib/api/songs";

  interface Props {
    /** File types to show (in addition to directories). Empty = show all. */
    filter?: string[];
    /** Allow selecting multiple files. */
    multiple?: boolean;
    /** Initial directory path to navigate to. */
    initialPath?: string;
    /** Called when user confirms selection. */
    onselect: (paths: string[]) => void;
    /** Called when user cancels. */
    oncancel: () => void;
  }

  let { filter = [], multiple = false, initialPath, onselect, oncancel }: Props = $props();

  // Absolute filesystem prefix from the API (used to reconstruct absolute paths for onselect).
  let absoluteRoot = $state("");
  let currentPath = $state("/");
  let entries = $state<BrowseEntry[]>([]);
  let selected = new SvelteSet<string>();
  let loading = $state(true);
  let error = $state("");
  let pathInput = $state("/");

  async function navigate(path?: string) {
    loading = true;
    error = "";
    selected.clear();
    try {
      const result = await browseDirectory(path);
      absoluteRoot = result.root;
      currentPath = result.path;
      pathInput = result.path;
      entries = result.entries;
    } catch (e) {
      error = e instanceof Error ? e.message : get(t)("fileBrowser.emptyDir");
    } finally {
      loading = false;
    }
  }

  navigate(untrack(() => initialPath) || undefined);

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

  let visibleEntries = $derived(
    filter.length === 0
      ? entries
      : entries.filter((e) => e.is_dir || filter.includes(e.type)),
  );

  function toggleSelect(entry: BrowseEntry) {
    if (entry.is_dir) {
      navigate(entry.path);
      return;
    }
    if (selected.has(entry.path)) {
      selected.delete(entry.path);
    } else {
      if (!multiple) selected.clear();
      selected.add(entry.path);
    }
  }

  function selectAll() {
    const files = visibleEntries.filter((e) => !e.is_dir);
    if (selected.size === files.length) {
      selected.clear();
    } else {
      selected.clear();
      for (const e of files) selected.add(e.path);
    }
  }

  /** Convert a project-relative path to an absolute filesystem path. */
  function toAbsolute(relativePath: string): string {
    const suffix = relativePath === "/" ? "" : relativePath;
    const root = absoluteRoot.replace(/\/+$/, "");
    return root + suffix;
  }

  function confirm() {
    if (selected.size > 0) {
      // Return absolute paths so consumers can write them to song configs.
      onselect(Array.from(selected).map(toAbsolute));
    }
  }

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

  let fileCount = $derived(visibleEntries.filter((e) => !e.is_dir).length);

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
      <button class="btn" onclick={navigateToInput}>{$t("common.go")}</button>
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
    {:else if error}
      <div class="browser-status error">{error}</div>
    {:else if visibleEntries.length === 0}
      <div class="browser-status">{$t("fileBrowser.emptyDir")}</div>
    {:else}
      <div class="entry-list">
        {#if !atRoot}
          <button class="entry" onclick={() => navigate(parentPath())}>
            <span class="entry-icon">⬆️</span>
            <span class="entry-name">..</span>
          </button>
        {/if}
        {#each visibleEntries as entry (entry.path)}
          <button
            class="entry"
            class:selected={selected.has(entry.path)}
            class:dir={entry.is_dir}
            onclick={() => toggleSelect(entry)}
          >
            <span class="entry-icon">{typeIcon(entry.type)}</span>
            <span class="entry-name">{entry.name}</span>
            {#if !entry.is_dir && selected.has(entry.path)}
              <span class="check">✓</span>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="browser-footer">
    <div class="footer-info">
      {#if selected.size > 0}
        <span
          >{$t("fileBrowser.selected", {
            values: { count: selected.size },
          })}</span
        >
      {:else if fileCount > 0}
        <span
          >{$t("fileBrowser.fileCount", { values: { count: fileCount } })}</span
        >
      {/if}
      {#if multiple && fileCount > 0}
        <button class="btn btn-sm" onclick={selectAll}>
          {selected.size === fileCount
            ? $t("fileBrowser.deselectAll")
            : $t("fileBrowser.selectAll")}
        </button>
      {/if}
    </div>
    <div class="footer-actions">
      <button class="btn" onclick={oncancel}>{$t("common.cancel")}</button>
      <button
        class="btn btn-primary"
        onclick={confirm}
        disabled={selected.size === 0}
      >
        {$t("fileBrowser.select", { values: { count: selected.size } })}
      </button>
    </div>
  </div>
</div>

<style>
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
  .entry.selected {
    background: rgba(94, 202, 234, 0.12);
  }
  .entry.dir .entry-name {
    color: var(--accent);
    font-weight: 500;
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
  .check {
    color: var(--accent);
    font-weight: 700;
    font-size: 15px;
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
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .footer-actions {
    display: flex;
    gap: 6px;
  }
  .btn-sm {
    padding: 2px 8px;
    font-size: 12px;
  }
</style>
