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
  } from "../../lib/api/config";
  import {
    fetchWaveform,
    uploadTrack,
    importFileToSong,
    type SongSummary,
    type WaveformTrack,
  } from "../../lib/api/songs";
  import type {
    LightFile,
    LightShow,
    Sequence,
    TempoSection,
  } from "../../lib/lighting/types";
  import { parseLightFile } from "../../lib/lighting/parser";
  import { serializeLightFile } from "../../lib/lighting/serializer";
  import TimelineEditor from "../lighting/timeline/TimelineEditor.svelte";
  import FileUpload from "./FileUpload.svelte";
  import FileBrowser from "./FileBrowser.svelte";
  import { t } from "svelte-i18n";
  import { playerClient } from "../../lib/grpc/client";
  import { playbackStore, wsConnected } from "../../lib/ws/stores";
  import { create } from "@bufbuild/protobuf";
  import { DurationSchema } from "@bufbuild/protobuf/wkt";
  import { get } from "svelte/store";

  interface LoadedLightFile {
    path: string;
    raw: string;
    parsed: LightFile;
    parseError?: string;
  }

  interface Props {
    song: SongSummary;
    onreload: () => void;
    dirty?: boolean;
    onchange?: () => void;
  }

  let { song, onreload, dirty = $bindable(false), onchange }: Props = $props();

  let error = $state("");
  let tab = $state<"timeline" | "raw">("timeline");

  let lightFiles = $state<LoadedLightFile[]>([]);
  let waveformTracks = $state<WaveformTrack[]>([]);
  let mergedLightFile = $state<LightFile>({ sequences: [], shows: [] });
  let rawFileIndex = $state(0);
  let rawContent = $state("");
  let validationResult = $state<{ valid: boolean; errors?: string[] } | null>(
    null,
  );
  let venueGroups = $state<string[]>([]);
  let sequenceNames = $derived(mergedLightFile.sequences.map((s) => s.name));
  let uploading = $state(false);
  let showMidiDmxModal = $state(false);
  let showFileBrowser = $state(false);

  // Playback state derived from the WebSocket playback store
  let pbState = $state(get(playbackStore));
  const unsubPlayback = playbackStore.subscribe((v) => (pbState = v));

  // WebSocket connection state
  let connected = $state(get(wsConnected));
  const unsubConnected = wsConnected.subscribe((v) => (connected = v));

  // Client-side interpolation for smooth playhead
  let lastKnownMs = $state(0);
  let lastUpdateTime = $state(0);
  let interpolatedMs = $state<number | null>(null);
  let rafId = 0;

  $effect(() => {
    const isOurSong = pbState.is_playing && pbState.song_name === song.name;
    if (isOurSong) {
      lastKnownMs = pbState.elapsed_ms;
      lastUpdateTime = performance.now();
      startInterpolation();
    } else {
      stopInterpolation();
      interpolatedMs = null;
    }
  });

  function startInterpolation() {
    if (rafId) return;
    function tick() {
      const now = performance.now();
      const elapsed = now - lastUpdateTime;
      // Cap interpolation drift to 2s — beyond that, hold at lastKnownMs
      // until the next WebSocket update arrives (prevents large jumps on reconnect).
      const capped = Math.min(elapsed, 2000);
      interpolatedMs = Math.min(
        lastKnownMs + capped,
        song.duration_ms || Infinity,
      );
      rafId = requestAnimationFrame(tick);
    }
    rafId = requestAnimationFrame(tick);
  }

  function stopInterpolation() {
    if (rafId) {
      cancelAnimationFrame(rafId);
      rafId = 0;
    }
  }

  let isPlaying = $derived(
    pbState.is_playing && pbState.song_name === song.name,
  );
  let playheadMs = $derived(isPlaying ? interpolatedMs : null);

  async function playFromMs(ms: number) {
    // Auto-save if dirty
    if (dirty) {
      try {
        await saveLightingFiles();
      } catch (e) {
        error = `Auto-save failed: ${e instanceof Error ? e.message : e}`;
        return;
      }
    }

    // If already playing, stop first then play from new position
    if (pbState.is_playing) {
      try {
        await playerClient.stop({});
        // Brief wait for stop to complete
        await new Promise((r) => setTimeout(r, 100));
      } catch {
        // Ignore stop errors (might not be playing)
      }
    }

    try {
      const seconds = BigInt(Math.floor(ms / 1000));
      const nanos = Math.round((ms % 1000) * 1_000_000);
      await playerClient.playSongFrom({
        songName: song.name,
        startTime: create(DurationSchema, { seconds, nanos }),
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg.includes("already playing")) {
        error = "Another song is playing. Stop it first.";
      } else {
        error = `Playback failed: ${msg}`;
      }
    }
  }

  async function stopPlayback() {
    try {
      await playerClient.stop({});
    } catch (e) {
      console.error("stop failed:", e);
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

  async function loadLighting() {
    try {
      error = "";
      dirty = false;
      tab = "timeline";

      const loaded: LoadedLightFile[] = [];
      for (const filePath of song.lighting_files) {
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

      rawFileIndex = 0;
      rawContent = loaded.length > 0 ? loaded[0].raw : "";

      try {
        const wf = await fetchWaveform(song.name);
        waveformTracks = wf.tracks;
      } catch {
        waveformTracks = [];
      }
    } catch (e: any) {
      error = e.message;
    }
  }

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

  function onTimelineChange(lf: LightFile) {
    mergedLightFile = lf;
    dirty = true;
    onchange?.();

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

    if (showOffset < lf.shows.length || seqOffset < lf.sequences.length) {
      if (lightFiles.length === 0) {
        const newPath = `${song.name.replace(/[^a-zA-Z0-9_-]/g, "_") || "show"}.light`;
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

  /** Save all modified lighting files. Called by parent's unified Save button. */
  export async function saveLightingFiles() {
    for (const lf of lightFiles) {
      if (lf.parseError) continue;
      const content = serializeLightFile(lf.parsed);
      lf.raw = content;
      await saveLightingFile(lf.path, content);
    }
    dirty = false;
  }

  function switchToRaw() {
    if (lightFiles.length > 0 && !lightFiles[rawFileIndex]?.parseError) {
      rawContent = serializeLightFile(lightFiles[rawFileIndex].parsed);
    }
    tab = "raw";
  }

  function switchToTimeline() {
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
    onchange?.();
    validationResult = null;
  }

  function selectRawFile(index: number) {
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

  async function addLightFile() {
    const name = prompt("Light show filename (e.g. verse_lights):");
    if (!name) return;
    const fileName = name.endsWith(".light") ? name : `${name}.light`;
    const showName = fileName.replace(/\.light$/, "");
    const defaultContent = `show "${showName}" {\n    @00:00.000\n}\n`;

    const baseDir = song.base_dir;
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
      onchange?.();
    } catch (e: any) {
      error = e.message;
    }
  }

  async function handleFileUpload(files: File[], mode: "dsl" | "midi") {
    uploading = true;
    try {
      for (const file of files) {
        let fileName = file.name;
        if (mode === "midi" && !fileName.startsWith("dmx_")) {
          fileName = `dmx_${fileName}`;
        }
        const renamedFile = new File([file], fileName, { type: file.type });
        const res = await uploadTrack(song.name, renamedFile);
        if (!res.ok) throw new Error(`Upload failed: ${res.status}`);
      }
      onreload();
    } catch (err: any) {
      error = err.message;
    } finally {
      uploading = false;
    }
  }

  async function handleBrowseImport(paths: string[]) {
    showFileBrowser = false;
    if (paths.length === 0) return;
    try {
      for (const path of paths) {
        let filename = path.split("/").pop() ?? path;
        if (
          !filename.startsWith("dmx_") &&
          (filename.endsWith(".mid") || filename.endsWith(".midi"))
        ) {
          filename = `dmx_${filename}`;
        }
        const res = await importFileToSong(song.name, path, filename);
        if (!res.ok) {
          const data = await res.json().catch(() => null);
          throw new Error(data?.error ?? `Import failed: ${res.status}`);
        }
      }
      onreload();
    } catch (err: any) {
      error = err.message;
    }
  }

  let initialized = false;
  $effect(() => {
    // Only run once on mount — not on every state change.
    if (initialized) return;
    initialized = true;
    loadLighting();
    loadVenueGroups();
    return () => {
      unsubPlayback();
      unsubConnected();
      stopInterpolation();
    };
  });
</script>

<div class="lighting-section">
  {#if !connected}
    <div class="warn-banner">
      {$t("songLighting.notConnected")}
    </div>
  {/if}
  {#if error}
    <div class="error-banner">
      {error}
      <button class="error-dismiss" onclick={() => (error = "")}
        >&#10005;</button
      >
    </div>
  {/if}

  <div class="detail-toolbar">
    <div class="tab-btns">
      <button
        class="tab-btn"
        class:active={tab === "timeline"}
        onclick={switchToTimeline}>{$t("songLighting.timeline")}</button
      >
      <button class="tab-btn" class:active={tab === "raw"} onclick={switchToRaw}
        >{$t("songLighting.rawDsl")}</button
      >
    </div>

    <div class="file-info">
      {$t("songLighting.dslCount", { values: { count: lightFiles.length } })}
      {#if song.midi_dmx_files.length > 0}
        {$t("songLighting.midiDmxCount", {
          values: { count: song.midi_dmx_files.length },
        })}
      {/if}
      <button class="btn btn-sm" onclick={addLightFile}
        >{$t("songLighting.addDsl")}</button
      >
      <button class="btn btn-sm" onclick={() => (showMidiDmxModal = true)}>
        {$t("songLighting.midiDmxBtn")}
      </button>
    </div>
  </div>

  {#if lightFiles.some((f) => f.parseError)}
    <div class="error-banner">
      {$t("songLighting.parseErrors")}
    </div>
  {/if}

  {#if tab === "timeline"}
    <TimelineEditor
      lightFile={mergedLightFile}
      groups={venueGroups}
      {sequenceNames}
      songDurationMs={song.duration_ms}
      {waveformTracks}
      {isPlaying}
      {playheadMs}
      onchange={onTimelineChange}
      onplay={playFromMs}
      onstop={stopPlayback}
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
        <p class="muted">{$t("songLighting.noLightFiles")}</p>
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
</div>

{#if showMidiDmxModal}
  <div
    class="modal-overlay"
    onclick={() => (showMidiDmxModal = false)}
    onkeydown={(e) => e.key === "Escape" && (showMidiDmxModal = false)}
    role="dialog"
    aria-label="MIDI DMX Files"
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
        <h3>{$t("songLighting.midiDmxFiles")}</h3>
        <span class="modal-song">{song.name}</span>
        <button class="btn btn-sm" onclick={() => (showMidiDmxModal = false)}
          >{$t("common.close")}</button
        >
      </div>
      <div class="modal-body">
        {#if song.midi_dmx_files.length > 0}
          <div class="modal-section">
            <span class="modal-section-label"
              >{$t("songLighting.currentFiles")}</span
            >
            <div class="midi-dmx-files">
              {#each song.midi_dmx_files as lf (lf)}
                <span class="midi-dmx-file" title={lf}
                  >{lf.replace(/^.*\//, "")}</span
                >
              {/each}
            </div>
          </div>
        {:else}
          <p class="muted">{$t("songLighting.noMidiDmxFiles")}</p>
        {/if}
        <div class="modal-section">
          <span class="modal-section-label">{$t("songLighting.upload")}</span>
          <FileUpload
            accept=".mid,.midi"
            label={uploading
              ? $t("common.uploading")
              : $t("songLighting.dropMidiDmx")}
            onupload={(files) => handleFileUpload(files, "midi")}
          />
        </div>
        <div class="modal-section">
          <span class="modal-section-label"
            >{$t("songLighting.importFromFs")}</span
          >
          <button class="btn" onclick={() => (showFileBrowser = true)}>
            {$t("songLighting.browseServer")}
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
  .lighting-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: calc(100vh - 280px);
  }
  .warn-banner {
    background: rgba(234, 179, 8, 0.15);
    color: #eab308;
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 13px;
  }
  .error-banner {
    background: rgba(220, 38, 38, 0.15);
    color: #ef4444;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 14px;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .error-dismiss {
    background: none;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 15px;
    padding: 0 4px;
    margin-left: 8px;
    opacity: 0.7;
  }
  .error-dismiss:hover {
    opacity: 1;
  }
  .detail-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    row-gap: 8px;
    flex-wrap: wrap;
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
  .muted {
    color: var(--text-muted);
    font-size: 14px;
  }
  .raw-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
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
    z-index: 200;
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
</style>
