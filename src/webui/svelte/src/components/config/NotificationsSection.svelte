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
<script lang="ts" module>
  export interface NotifBrowseTarget {
    /** Which field is being browsed — an event key or "section:<name>" */
    field: string;
  }
</script>

<script lang="ts">
  import { t } from "svelte-i18n";
  import Tooltip from "./Tooltip.svelte";
  import FileUpload from "../songs/FileUpload.svelte";

  interface Props {
    notifications: Record<string, unknown>;
    onchange: () => void;
    onbrowse?: (target: NotifBrowseTarget) => void;
    onupload?: (files: File[]) => void;
    uploadMsg?: string;
    uploading?: boolean;
    /** Section names for autocomplete in per-section overrides. */
    sectionNames?: string[];
  }

  let {
    notifications = $bindable(),
    onchange,
    onbrowse,
    onupload,
    uploadMsg = "",
    uploading = false,
    sectionNames = [],
  }: Props = $props();

  const eventFields: [string, string, string][] = [
    [
      "loop_armed",
      "notifications.loopArmed",
      "tooltips.notifications.loopArmed",
    ],
    [
      "break_requested",
      "notifications.breakRequested",
      "tooltips.notifications.breakRequested",
    ],
    [
      "loop_exited",
      "notifications.loopExited",
      "tooltips.notifications.loopExited",
    ],
    [
      "section_entering",
      "notifications.sectionEntering",
      "tooltips.notifications.sectionEntering",
    ],
  ];

  function updateField(key: string, value: string) {
    if (value.trim()) {
      notifications[key] = value.trim();
    } else {
      delete notifications[key];
    }
    onchange();
  }

  /** Called by the parent after a file browse completes. */
  export function applyBrowseResult(target: NotifBrowseTarget, path: string) {
    if (target.field.startsWith("section:")) {
      const name = target.field.slice("section:".length);
      updateSectionFile(name, path);
    } else {
      updateField(target.field, path);
    }
  }

  // Per-section overrides
  function sections(): [string, string][] {
    const s = notifications.sections;
    if (!s || typeof s !== "object") return [];
    return Object.entries(s as Record<string, string>).sort(([a], [b]) =>
      a.localeCompare(b),
    );
  }

  function addSection() {
    if (!notifications.sections || typeof notifications.sections !== "object") {
      notifications.sections = {};
    }
    const existing = notifications.sections as Record<string, string>;
    let name = "section";
    let i = 1;
    while (name in existing) {
      name = `section${i++}`;
    }
    existing[name] = "";
    notifications = { ...notifications };
    onchange();
  }

  function removeSection(name: string) {
    const s = notifications.sections as Record<string, string> | undefined;
    if (s) {
      delete s[name];
      if (Object.keys(s).length === 0) {
        delete notifications.sections;
      }
    }
    notifications = { ...notifications };
    onchange();
  }

  function updateSectionName(oldName: string, newName: string) {
    const s = notifications.sections as Record<string, string>;
    if (!s || newName === oldName) return;
    const trimmed = newName.trim();
    if (!trimmed || trimmed in s) return;
    const value = s[oldName];
    delete s[oldName];
    s[trimmed] = value;
    notifications = { ...notifications };
    onchange();
  }

  function updateSectionFile(name: string, value: string) {
    const s = notifications.sections as Record<string, string>;
    if (!s) return;
    s[name] = value;
    onchange();
  }
</script>

<div class="section-fields">
  <p class="muted hint-text">{$t("notifications.hint")}</p>

  {#each eventFields as [key, labelKey, tooltipKey] (key)}
    <div class="field">
      <label for="notif-{key}"
        >{$t(labelKey)} <Tooltip text={$t(tooltipKey)} /></label
      >
      <div class="file-row">
        <input
          id="notif-{key}"
          class="input"
          type="text"
          placeholder={$t("notifications.filePlaceholder")}
          value={(notifications[key] as string) ?? ""}
          onchange={(e) =>
            updateField(key, (e.target as HTMLInputElement).value)}
        />
        {#if onbrowse}
          <button
            class="btn browse-btn"
            onclick={() => onbrowse({ field: key })}
            title={$t("notifications.browse")}>...</button
          >
        {/if}
      </div>
    </div>
  {/each}

  <div class="sections-area">
    <div class="field-header">
      <span class="field-label"
        >{$t("notifications.sections")}
        <Tooltip text={$t("tooltips.notifications.sections")} /></span
      >
      <button class="btn" onclick={addSection}>{$t("common.add")}</button>
    </div>
    <p class="muted hint-text">{$t("notifications.sectionsHint")}</p>
    {#each sections() as [name, file] (name)}
      <div class="section-row">
        <input
          class="input section-name"
          type="text"
          list="notif-section-names"
          value={name}
          placeholder={$t("notifications.sectionNamePlaceholder")}
          onchange={(e) =>
            updateSectionName(name, (e.target as HTMLInputElement).value)}
        />
        <div class="file-row section-file-row">
          <input
            class="input"
            type="text"
            value={file}
            placeholder={$t("notifications.filePlaceholder")}
            onchange={(e) =>
              updateSectionFile(name, (e.target as HTMLInputElement).value)}
          />
          {#if onbrowse}
            <button
              class="btn browse-btn"
              onclick={() => onbrowse({ field: `section:${name}` })}
              title={$t("notifications.browse")}>...</button
            >
          {/if}
        </div>
        <button class="btn btn-danger" onclick={() => removeSection(name)}
          >X</button
        >
      </div>
    {/each}
  </div>

  {#if onupload}
    <div class="upload-area">
      <FileUpload
        accept=".wav,.flac,.mp3,.ogg,.aac,.m4a,.aiff,.aif"
        label={uploading
          ? $t("common.uploading")
          : $t("notifications.dropAudio")}
        {onupload}
      />
    </div>
    {#if uploadMsg}
      <div class="msg">{uploadMsg}</div>
    {/if}
  {/if}
</div>

{#if sectionNames.length > 0}
  <datalist id="notif-section-names">
    {#each sectionNames as name (name)}
      <option value={name}></option>
    {/each}
  </datalist>
{/if}

<style>
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .hint-text {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .file-row {
    display: flex;
    gap: 4px;
  }
  .file-row .input {
    flex: 1;
    min-width: 0;
  }
  .browse-btn {
    padding: 4px 8px;
    font-size: 13px;
    flex: 0 0 auto;
  }
  .sections-area {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .field-label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .section-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .section-name {
    width: 140px;
    flex-shrink: 0;
  }
  .section-file-row {
    flex: 1;
  }
  .upload-area {
    margin-top: 4px;
  }
  .msg {
    font-size: 13px;
    color: var(--text-muted);
    margin-top: 4px;
  }
  @media (max-width: 480px) {
    .section-row {
      flex-wrap: wrap;
    }
    .section-name {
      width: 100%;
    }
  }
</style>
