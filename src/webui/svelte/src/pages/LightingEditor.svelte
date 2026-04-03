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
    fetchLightingFile,
    saveLightingFile,
    validateLighting,
    fetchVenues,
  } from "../lib/api/config";
  import {
    fetchSongs,
    fetchWaveform,
    uploadTrack,
    importFileToSong,
    type SongSummary,
    type WaveformTrack,
  } from "../lib/api/songs";
  import type {
    LightFile,
    LightShow,
    Sequence,
    TempoSection,
  } from "../lib/lighting/types";
  import { parseLightFile } from "../lib/lighting/parser";
  import { serializeLightFile } from "../lib/lighting/serializer";
  import TimelineEditor from "../components/lighting/timeline/TimelineEditor.svelte";
  import FileUpload from "../components/songs/FileUpload.svelte";
  import FileBrowser from "../components/songs/FileBrowser.svelte";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { showConfirm, showPrompt } from "../lib/dialog.svelte";

  // --- Types ---

  /** A loaded lighting file with its path and parsed content. */
  interface LoadedLightFile {
    /** Path relative to song directory (e.g. "lighting/main_show.light") */
    path: string;
    /** Raw DSL content */
    raw: string;
    /** Parsed light file */
    parsed: LightFile;
    /** Parse error, if any */
    parseError?: string;
  }

  // --- State ---

  let songs = $state<SongSummary[]>([]);
  let selectedSongName = $state<string | null>(null);
  let loading = $state(true);
  let error = $state("");
  let saveMsg = $state("");
  let saving = $state(false);
  let dirty = $state(false);
  let tab = $state<"timeline" | "raw">("timeline");

  // Lighting files and song data
  let selectedSong = $state<SongSummary | null>(null);
  let lightFiles = $state<LoadedLightFile[]>([]);
  let songDurationMs = $state(0);
  let waveformTracks = $state<WaveformTrack[]>([]);

  // Merged view for the timeline editor
  let mergedLightFile = $state<LightFile>({ sequences: [], shows: [] });

  // Raw editor state
  let rawFileIndex = $state(0);
  let rawContent = $state("");
  let validationResult = $state<{ valid: boolean; errors?: string[] } | null>(
    null,
  );

  // Venue groups for autocomplete
  let venueGroups = $state<string[]>([]);
  let sequenceNames = $derived(mergedLightFile.sequences.map((s) => s.name));

  let uploading = $state(false);
  let showMidiDmxModal = $state(false);
  let showFileBrowser = $state(false);

  // --- Data loading ---

  async function loadSongs() {
    try {
      loading = true;
      error = "";
      songs = (await fetchSongs()).songs;
    } catch (e: any) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  async function loadVenueGroups() {
    try {
      const venues = await fetchVenues();
      const groupNames: string[] = [];
      for (const v of Object.values(venues)) {
        if (v.groups) {
          for (const g of Object.keys(v.groups)) {
            if (!groupNames.includes(g)) groupNames.push(g);
          }
        }
      }
      venueGroups = groupNames.sort();
    } catch {
      // Non-critical
    }
  }

  async function selectSong(name: string) {
    if (dirty && selectedSongName) {
      if (!(await showConfirm(get(t)("lightingEditor.discardSwitch")))) return;
    }
    try {
      error = "";
      dirty = false;
      saveMsg = "";
      selectedSongName = name;
      tab = "timeline";

      const song = songs.find((s) => s.name === name) ?? null;
      selectedSong = song;
      songDurationMs = song?.duration_ms ?? 0;

      // Load lighting files using paths from the songs API (relative to songs root)
      const loaded: LoadedLightFile[] = [];
      for (const filePath of song?.lighting_files ?? []) {
        try {
          const raw = await fetchLightingFile(filePath);
          try {
            const parsed = parseLightFile(raw);
            loaded.push({ path: filePath, raw, parsed });
          } catch (e: any) {
            loaded.push({
              path: filePath,
              raw,
              parsed: { sequences: [], shows: [] },
              parseError: e.message,
            });
          }
        } catch (e: any) {
          error = `Failed to load ${filePath}: ${e.message}`;
        }
      }
      lightFiles = loaded;
      rebuildMergedLightFile();

      // Raw editor defaults to first file
      rawFileIndex = 0;
      rawContent = loaded.length > 0 ? loaded[0].raw : "";

      // Load waveform
      try {
        const wf = await fetchWaveform(name);
        waveformTracks = wf.tracks;
      } catch {
        waveformTracks = [];
      }
    } catch (e: any) {
      error = e.message;
    }
  }

  // --- Merge/split light files ---

  /** Merge all loaded light files into a single LightFile for the timeline. */
  function rebuildMergedLightFile() {
    const allShows: LightShow[] = [];
    const allSequences: Sequence[] = [];
    let tempo: TempoSection | undefined;

    for (const lf of lightFiles) {
      if (lf.parseError) continue;
      if (!tempo && lf.parsed.tempo) tempo = lf.parsed.tempo;
      allShows.push(...lf.parsed.shows);
      allSequences.push(...lf.parsed.sequences);
    }

    mergedLightFile = { tempo, sequences: allSequences, shows: allShows };
  }

  /**
   * When the timeline editor changes the merged LightFile, split changes
   * back into individual files. We track shows/sequences by index:
   * the first N shows come from file 0, the next M from file 1, etc.
   */
  function onTimelineChange(lf: LightFile) {
    mergedLightFile = lf;
    dirty = true;

    // Distribute shows and sequences back to their source files
    let showOffset = 0;
    let seqOffset = 0;
    for (const loaded of lightFiles) {
      if (loaded.parseError) continue;
      const origShowCount = loaded.parsed.shows.length;
      const origSeqCount = loaded.parsed.sequences.length;

      loaded.parsed = {
        tempo: lf.tempo,
        shows: lf.shows.slice(showOffset, showOffset + origShowCount),
        sequences: lf.sequences.slice(seqOffset, seqOffset + origSeqCount),
      };

      showOffset += origShowCount;
      seqOffset += origSeqCount;
    }

    // Any new shows/sequences beyond the original counts go to the first file
    // (or create a new file if none exists)
    if (showOffset < lf.shows.length || seqOffset < lf.sequences.length) {
      if (lightFiles.length === 0) {
        // No light files yet — create one
        const newPath = `${selectedSongName?.replace(/[^a-zA-Z0-9_-]/g, "_") || "show"}.light`;
        lightFiles = [
          ...lightFiles,
          {
            path: newPath,
            raw: "",
            parsed: {
              tempo: lf.tempo,
              shows: lf.shows.slice(showOffset),
              sequences: lf.sequences.slice(seqOffset),
            },
          },
        ];
      } else {
        // Append to first file
        const first = lightFiles[0];
        if (!first.parseError) {
          first.parsed = {
            ...first.parsed,
            shows: [...first.parsed.shows, ...lf.shows.slice(showOffset)],
            sequences: [
              ...first.parsed.sequences,
              ...lf.sequences.slice(seqOffset),
            ],
          };
        }
      }
    }
  }

  // --- Save ---

  async function handleSave() {
    try {
      saving = true;
      error = "";
      saveMsg = "";

      // Save each light file
      for (const lf of lightFiles) {
        if (lf.parseError) continue;
        const content = serializeLightFile(lf.parsed);
        lf.raw = content;
        await saveLightingFile(lf.path, content);
      }

      // Reload the song list so lighting_files are up to date
      await loadSongs();

      dirty = false;
      saveMsg = get(t)("common.saved");
      setTimeout(() => (saveMsg = ""), 2000);
    } catch (e: any) {
      error = e.message;
    } finally {
      saving = false;
    }
  }

  // --- Raw editor ---

  function switchToRaw() {
    // Serialize current state for the selected file
    if (lightFiles.length > 0 && !lightFiles[rawFileIndex]?.parseError) {
      rawContent = serializeLightFile(lightFiles[rawFileIndex].parsed);
    }
    tab = "raw";
  }

  function switchToTimeline() {
    // Re-parse raw content back into the file
    if (tab === "raw" && lightFiles.length > 0) {
      try {
        const parsed = parseLightFile(rawContent);
        lightFiles[rawFileIndex].parsed = parsed;
        lightFiles[rawFileIndex].raw = rawContent;
        lightFiles[rawFileIndex].parseError = undefined;
        rebuildMergedLightFile();
      } catch (e: any) {
        error = `Parse error: ${e.message}. Fix in Raw DSL tab.`;
        return;
      }
    }
    tab = "timeline";
  }

  function onRawInput(e: Event) {
    rawContent = (e.target as HTMLTextAreaElement).value;
    dirty = true;
    validationResult = null;
  }

  function selectRawFile(index: number) {
    // Save current raw back
    if (lightFiles[rawFileIndex] && !lightFiles[rawFileIndex].parseError) {
      try {
        lightFiles[rawFileIndex].parsed = parseLightFile(rawContent);
        lightFiles[rawFileIndex].raw = rawContent;
      } catch {
        // Keep old
      }
    }
    rawFileIndex = index;
    rawContent = lightFiles[index]?.raw ?? "";
  }

  async function handleValidate() {
    try {
      validationResult = await validateLighting(rawContent);
    } catch (e: any) {
      validationResult = { valid: false, errors: [e.message] };
    }
  }

  // --- Add light file to song ---

  async function addLightFile() {
    const name = await showPrompt("Light show filename (e.g. verse_lights):");
    if (!name) return;
    const fileName = name.endsWith(".light") ? name : `${name}.light`;
    const showName = fileName.replace(/\.light$/, "");
    const defaultContent = `show "${showName}" {\n    @00:00.000\n}\n`;

    // Path relative to songs root (song base_dir + filename)
    const baseDir = selectedSong?.base_dir ?? "";
    const fullPath = baseDir ? `${baseDir}/${fileName}` : fileName;

    try {
      await saveLightingFile(fullPath, defaultContent);
      const parsed = parseLightFile(defaultContent);
      lightFiles = [
        ...lightFiles,
        { path: fullPath, raw: defaultContent, parsed },
      ];
      rebuildMergedLightFile();
      dirty = true;
    } catch (e: any) {
      error = e.message;
    }
  }

  // --- MIDI / DSL file management ---

  async function handleFileUpload(files: File[], mode: "dsl" | "midi") {
    if (!selectedSongName) return;
    uploading = true;
    try {
      for (const file of files) {
        let fileName = file.name;
        if (mode === "midi" && !fileName.startsWith("dmx_")) {
          fileName = `dmx_${fileName}`;
        }
        const renamedFile = new File([file], fileName, { type: file.type });
        const res = await uploadTrack(selectedSongName, renamedFile);
        if (!res.ok) throw new Error(`Upload failed: ${res.status}`);
      }
      await reloadCurrentSong();
      saveMsg = get(t)("common.uploaded");
      setTimeout(() => (saveMsg = ""), 2000);
    } catch (err: any) {
      error = err.message;
    } finally {
      uploading = false;
    }
  }

  async function handleBrowseImport(paths: string[]) {
    showFileBrowser = false;
    if (!selectedSongName || paths.length === 0) return;
    try {
      for (const path of paths) {
        let filename = path.split("/").pop() ?? path;
        if (
          !filename.startsWith("dmx_") &&
          (filename.endsWith(".mid") || filename.endsWith(".midi"))
        ) {
          filename = `dmx_${filename}`;
        }
        const res = await importFileToSong(selectedSongName, path, filename);
        if (!res.ok) {
          const data = await res.json().catch(() => null);
          throw new Error(data?.error ?? `Import failed: ${res.status}`);
        }
      }
      await reloadCurrentSong();
      saveMsg = get(t)("common.imported");
      setTimeout(() => (saveMsg = ""), 2000);
    } catch (err: any) {
      error = err.message;
    }
  }

  async function reloadCurrentSong() {
    await loadSongs();
    if (selectedSongName) {
      selectedSong = songs.find((s) => s.name === selectedSongName) ?? null;
    }
  }

  // --- Init ---

  loadSongs();
  loadVenueGroups();
</script>

<div class="lighting-editor">
  <div class="panel list-panel">
    <div class="panel-header">
      <h3>{$t("lightingEditor.songs")}</h3>
    </div>

    {#if loading}
      <p class="muted">
        <span class="spinner sm"></span>
        {$t("common.loading")}
      </p>
    {:else if songs.length === 0}
      <p class="muted">{$t("lightingEditor.noSongs")}</p>
    {:else}
      <ul class="song-list">
        {#each songs as song (song.name)}
          <li class:selected={selectedSongName === song.name}>
            <button class="song-item" onclick={() => selectSong(song.name)}>
              <span class="song-name">{song.name}</span>
              <span class="song-meta">
                {song.duration_display}
                {#if song.lighting_files.length > 0}
                  <span class="lighting-dot dsl" title="DSL lighting"></span>
                {:else if song.has_lighting}
                  <span class="lighting-dot midi-dmx" title="MIDI DMX lighting"
                  ></span>
                {/if}
              </span>
            </button>
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

    {#if !selectedSongName}
      <p class="muted center">{$t("lightingEditor.selectSong")}</p>
    {:else}
      <div class="detail-toolbar">
        <span class="detail-title">{selectedSongName}</span>
        <div class="tab-btns">
          <button
            class="tab-btn"
            class:active={tab === "timeline"}
            onclick={switchToTimeline}>{$t("lightingEditor.timeline")}</button
          >
          <button
            class="tab-btn"
            class:active={tab === "raw"}
            onclick={switchToRaw}>{$t("lightingEditor.rawDsl")}</button
          >
        </div>

        <div class="file-info">
          {lightFiles.length}
          {$t("lightingEditor.dsl")}
          {#if selectedSong && selectedSong.midi_dmx_files.length > 0}
            + {selectedSong.midi_dmx_files.length}
            {$t("lightingEditor.midiDmx")}
          {/if}
          <button class="btn btn-sm" onclick={addLightFile}
            >{$t("lightingEditor.addDsl")}</button
          >
          <button class="btn btn-sm" onclick={() => (showMidiDmxModal = true)}>
            {$t("lightingEditor.midiDmxBtn")}
          </button>
        </div>

        <div class="toolbar-actions">
          {#if saveMsg}
            <span class="save-msg success">{saveMsg}</span>
          {/if}
          {#if dirty}
            <span class="dirty-badge">{$t("common.unsaved")}</span>
          {/if}
          <button
            class="btn btn-primary btn-sm"
            onclick={handleSave}
            disabled={saving}
          >
            {saving ? $t("common.saving") : $t("common.save")}
          </button>
        </div>
      </div>

      {#if lightFiles.some((f) => f.parseError)}
        <div class="error-banner">
          {$t("lightingEditor.parseErrors")}
        </div>
      {/if}

      {#if tab === "timeline"}
        <TimelineEditor
          lightFile={mergedLightFile}
          groups={venueGroups}
          {sequenceNames}
          {songDurationMs}
          {waveformTracks}
          onchange={onTimelineChange}
        />
      {:else if tab === "raw"}
        <div class="raw-editor">
          {#if lightFiles.length > 1}
            <div class="raw-file-tabs">
              {#each lightFiles as lf, i (lf.path)}
                <button
                  class="raw-file-tab"
                  class:active={rawFileIndex === i}
                  class:has-error={!!lf.parseError}
                  onclick={() => selectRawFile(i)}
                >
                  {lf.path}
                </button>
              {/each}
            </div>
          {:else if lightFiles.length === 1}
            <div class="raw-file-label">{lightFiles[0].path}</div>
          {:else}
            <p class="muted">
              {$t("lightingEditor.noLightFiles")}
            </p>
          {/if}

          {#if lightFiles.length > 0}
            <textarea
              class="raw-textarea"
              value={rawContent}
              oninput={onRawInput}
              spellcheck="false"
            ></textarea>
            <div class="raw-actions">
              <button class="btn btn-sm" onclick={handleValidate}
                >{$t("common.validate")}</button
              >
              {#if validationResult}
                {#if validationResult.valid}
                  <span class="validation-ok">{$t("common.valid")}</span>
                {:else}
                  <div class="validation-errors">
                    {#each validationResult.errors ?? [] as err, i (i)}
                      <div class="validation-error">{err}</div>
                    {/each}
                  </div>
                {/if}
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</div>

{#if showMidiDmxModal && selectedSong}
  <div
    class="modal-overlay"
    onclick={() => (showMidiDmxModal = false)}
    onkeydown={(e) => e.key === "Escape" && (showMidiDmxModal = false)}
    role="dialog"
    aria-label={$t("lightingEditor.midiDmxFiles")}
    tabindex="-1"
  >
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="document"
    >
      <div class="modal-header">
        <h3>{$t("lightingEditor.midiDmxFiles")}</h3>
        <span class="modal-song">{selectedSongName}</span>
        <button class="btn btn-sm" onclick={() => (showMidiDmxModal = false)}>
          {$t("common.close")}
        </button>
      </div>

      <div class="modal-body">
        {#if selectedSong.midi_dmx_files.length > 0}
          <div class="modal-section">
            <span class="modal-section-label"
              >{$t("lightingEditor.currentFiles")}</span
            >
            <div class="midi-dmx-files">
              {#each selectedSong.midi_dmx_files as lf (lf)}
                <span class="midi-dmx-file" title={lf}>
                  {lf.replace(/^.*\//, "")}
                </span>
              {/each}
            </div>
          </div>
        {:else}
          <p class="muted">{$t("lightingEditor.noMidiDmxFiles")}</p>
        {/if}

        <div class="modal-section">
          <span class="modal-section-label">{$t("lightingEditor.upload")}</span>
          <FileUpload
            accept=".mid,.midi"
            label={uploading
              ? $t("common.uploading")
              : $t("lightingEditor.dropMidiDmx")}
            onupload={(files) => handleFileUpload(files, "midi")}
          />
        </div>

        <div class="modal-section">
          <span class="modal-section-label"
            >{$t("lightingEditor.importFromFs")}</span
          >
          <button class="btn" onclick={() => (showFileBrowser = true)}>
            {$t("lightingEditor.browseServer")}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

{#if showFileBrowser}
  <div class="modal-overlay">
    <div class="modal modal-browser">
      <FileBrowser
        filter={["midi"]}
        onselect={handleBrowseImport}
        oncancel={() => (showFileBrowser = false)}
      />
    </div>
  </div>
{/if}

<style>
  .lighting-editor {
    display: flex;
    gap: 16px;
    height: calc(100vh - 120px);
  }
  .panel {
    border-radius: var(--radius-lg);
  }
  .list-panel {
    width: 280px;
    flex-shrink: 0;
  }
  .detail-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .muted {
    color: var(--text-muted);
    font-size: 14px;
  }
  .center {
    text-align: center;
    margin-top: 40px;
  }
  /* Song list */
  .song-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .song-list li {
    border-radius: 6px;
    padding: 1px;
  }
  .song-list li.selected {
    background: rgba(94, 202, 234, 0.12);
  }
  .song-item {
    width: 100%;
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: none;
    border: none;
    color: var(--text);
    padding: 6px 8px;
    font-size: 14px;
    cursor: pointer;
    border-radius: 4px;
    text-align: left;
  }
  .song-item:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .song-name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .song-meta {
    color: var(--text-dim);
    font-size: 12px;
    font-family: var(--mono);
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .lighting-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  .lighting-dot.dsl {
    background: var(--accent);
  }
  .lighting-dot.midi-dmx {
    background: var(--text-dim);
    border: 1px solid var(--text-muted);
    width: 5px;
    height: 5px;
  }

  /* Detail toolbar */
  .detail-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    row-gap: 8px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .detail-title {
    font-size: 16px;
    font-weight: 600;
  }
  .tab-btns {
    display: flex;
    gap: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .tab-btn {
    background: var(--bg-input);
    border: none;
    color: var(--text-muted);
    font-size: 13px;
    padding: 4px 12px;
    cursor: pointer;
  }
  .tab-btn.active {
    background: var(--accent);
    color: white;
  }
  .tab-btn:not(.active):hover {
    background: rgba(255, 255, 255, 0.06);
  }
  .file-info {
    font-size: 13px;
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  /* Modal */
  .modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-modal);
    padding: 24px;
  }
  .modal {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    width: 100%;
    max-width: 560px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .modal-browser {
    max-width: 700px;
  }
  .modal-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .modal-header h3 {
    margin: 0;
    font-size: 15px;
  }
  .modal-song {
    font-size: 13px;
    color: var(--text-muted);
    flex: 1;
  }
  .modal-body {
    padding: 16px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .modal-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .modal-section-label {
    font-size: 12px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .midi-dmx-files {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .midi-dmx-file {
    font-size: 12px;
    font-family: var(--mono);
    color: var(--text-muted);
    background: rgba(255, 255, 255, 0.04);
    padding: 2px 6px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .toolbar-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .save-msg.success {
    color: var(--green);
    font-size: 13px;
  }
  .dirty-badge {
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--yellow-dim);
    color: var(--yellow);
  }

  /* Raw editor */
  .raw-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1;
    min-height: 0;
  }
  .raw-file-tabs {
    display: flex;
    gap: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    align-self: flex-start;
  }
  .raw-file-tab {
    background: var(--bg-input);
    border: none;
    border-right: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 12px;
    padding: 4px 10px;
    cursor: pointer;
    font-family: var(--mono);
  }
  .raw-file-tab:last-child {
    border-right: none;
  }
  .raw-file-tab.active {
    background: var(--accent);
    color: white;
  }
  .raw-file-tab.has-error {
    color: var(--red);
  }
  .raw-file-label {
    font-size: 12px;
    font-family: var(--mono);
    color: var(--text-muted);
    padding: 4px 0;
  }
  .raw-textarea {
    flex: 1;
    min-height: 300px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-family: var(--mono);
    font-size: 14px;
    padding: 12px;
    resize: vertical;
    tab-size: 4;
    line-height: 1.5;
  }
  .raw-textarea:focus {
    border-color: var(--border-focus);
    outline: none;
  }
  .raw-actions {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .validation-ok {
    color: var(--green);
    font-size: 14px;
    padding: 4px 0;
  }
  .validation-errors {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .validation-error {
    color: var(--red);
    font-size: 13px;
    font-family: var(--mono);
  }

  @media (max-width: 768px) {
    .lighting-editor {
      flex-direction: column;
      height: auto;
      min-height: calc(100vh - 120px);
    }
    .list-panel {
      width: 100%;
      max-height: 200px;
    }
    .detail-panel {
      min-height: 400px;
    }
    .detail-toolbar {
      flex-wrap: wrap;
      gap: 8px;
    }
  }
</style>
