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
  import YAML from "yaml";
  import {
    fetchSongs,
    fetchSongConfig,
    fetchSongFiles,
    fetchWaveform,
    updateSong,
    uploadTrack,
    uploadTracks,
    type SongFile,
    type SongFailure,
    type SongSummary,
    type WaveformTrack,
  } from "../../lib/api/songs";
  import { playbackStore } from "../../lib/ws/stores";
  import FileBrowser from "./FileBrowser.svelte";
  import FileUpload from "./FileUpload.svelte";
  import FilePicker from "./FilePicker.svelte";
  import TrackEditor from "./TrackEditor.svelte";
  import SongLightingEditor from "./SongLightingEditor.svelte";

  interface TrackEntry {
    name: string;
    file: string;
    file_channel?: number;
  }

  interface Props {
    songName: string;
    initialTab?: "tracks" | "midi" | "lighting" | "config";
  }

  let { songName, initialTab }: Props = $props();

  let song = $state<SongSummary | null>(null);
  let songFiles = $state<SongFile[]>([]);
  let waveformTracks = $state<WaveformTrack[]>([]);
  let rawYaml = $state("");
  let editedYaml = $state("");
  let parsedConfig = $state<Record<string, unknown> | null>(null);
  let tracks = $state<TrackEntry[]>([]);
  let loading = $state(true);
  let error = $state("");
  let failureError = $state<string | null>(null);
  let saving = $state(false);
  let saveMsg = $state("");
  let uploading = $state(false);
  let uploadMsg = $state("");

  // Lighting editor state (lifted up for unified save)
  let lightingDirty = $state(false);
  let lightingEditorRef: SongLightingEditor | undefined = $state();

  // Tab state
  type TabKey = "tracks" | "midi" | "lighting" | "config";
  function getInitialTab(): TabKey {
    return initialTab ?? "tracks";
  }
  let activeTab = $state<TabKey>(getInitialTab());

  function setTab(tab: TabKey) {
    activeTab = tab;
    const base = `#/songs/${encodeURIComponent(songName)}`;
    window.location.hash = tab === "tracks" ? base : `${base}/${tab}`;
  }

  const tabs: { key: TabKey; label: string }[] = [
    { key: "tracks", label: "Tracks" },
    { key: "midi", label: "MIDI" },
    { key: "lighting", label: "Lighting" },
    { key: "config", label: "Config" },
  ];

  // File browser state
  type BrowseTarget =
    | { kind: "track"; index: number }
    | { kind: "midi" }
    | { kind: "lighting" };
  let browseTarget = $state<BrowseTarget | null>(null);

  function openBrowser(target: BrowseTarget) {
    browseTarget = target;
  }

  function closeBrowser() {
    browseTarget = null;
  }

  function onBrowseSelect(paths: string[]) {
    if (!browseTarget) return;
    const target = browseTarget;
    if (target.kind === "track") {
      if (target.index >= 0) {
        const idx = target.index;
        const updated = tracks.map((t, i) =>
          i === idx ? { ...t, file: paths[0] } : t,
        );
        onTracksChange(updated);
      } else {
        const newTracks = paths.map((p) => {
          const filename = p.split("/").pop() ?? p;
          return { name: filename.replace(/\.[^.]+$/, ""), file: p };
        });
        onTracksChange([...tracks, ...newTracks]);
      }
    } else if (browseTarget.kind === "midi") {
      setMidiFile(paths[0]);
    } else if (browseTarget.kind === "lighting") {
      for (const p of paths) {
        setLightingFile(p);
      }
    }
    browseTarget = null;
  }

  let browseFilter = $derived.by(() => {
    if (!browseTarget) return [];
    if (browseTarget.kind === "track") return ["audio"];
    if (browseTarget.kind === "midi") return ["midi"];
    if (browseTarget.kind === "lighting") return ["lighting"];
    return [];
  });

  let browseMultiple = $derived(
    browseTarget?.kind === "track"
      ? browseTarget.index < 0
      : browseTarget?.kind === "lighting",
  );

  function parseConfig(yaml: string) {
    try {
      const parsed = YAML.parse(yaml);
      if (parsed && typeof parsed === "object") {
        parsedConfig = parsed;
        tracks = (parsed.tracks ?? []).map((t: Record<string, unknown>) => ({
          name: (t.name as string) ?? "",
          file: (t.file as string) ?? "",
          file_channel: t.file_channel as number | undefined,
        }));
      }
    } catch {
      parsedConfig = null;
      tracks = [];
    }
  }

  function buildYaml(): string {
    if (!parsedConfig) return editedYaml;
    const updated = {
      ...parsedConfig,
      kind: "song",
      tracks: tracks.map((t) => {
        const entry: Record<string, unknown> = { name: t.name, file: t.file };
        if (t.file_channel !== undefined) {
          entry.file_channel = t.file_channel;
        }
        return entry;
      }),
    };
    return YAML.stringify(updated, { lineWidth: 0 });
  }

  async function load() {
    loading = true;
    error = "";
    failureError = null;
    try {
      const [result, yaml] = await Promise.all([
        fetchSongs(),
        fetchSongConfig(songName),
      ]);
      song = result.songs.find((s) => s.name === songName) ?? null;

      // Check if this song is in the failures list.
      const failure = result.failures.find(
        (f: SongFailure) => f.name === songName,
      );
      if (failure) {
        failureError = failure.error;
        // Default to config tab so the user can fix the YAML.
        if (!initialTab) {
          activeTab = "config";
        }
      }

      rawYaml = yaml;
      editedYaml = yaml;
      fetchSongFiles(songName)
        .then((f) => (songFiles = f))
        .catch(() => {});
      parseConfig(yaml);
      fetchWaveform(songName)
        .then((w) => (waveformTracks = w.tracks))
        .catch(() => {});
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load song";
    } finally {
      loading = false;
    }
  }

  load();

  function onTracksChange(updated: TrackEntry[]) {
    tracks = updated;
    const yaml = buildYaml();
    editedYaml = yaml;
  }

  function onRawYamlInput(value: string) {
    editedYaml = value;
    parseConfig(value);
  }

  function setMidiFile(filename: string) {
    if (!parsedConfig) return;
    parsedConfig = { ...parsedConfig, midi_file: filename };
    editedYaml = buildYaml();
  }

  function setLightingFile(filename: string) {
    if (!parsedConfig) return;
    const existing =
      (parsedConfig.lighting as { file: string }[] | undefined) ?? [];
    if (!existing.some((l) => l.file === filename)) {
      parsedConfig = {
        ...parsedConfig,
        lighting: [...existing, { file: filename }],
      };
      editedYaml = buildYaml();
    }
  }

  async function save() {
    if ($playbackStore.locked) {
      saveMsg = "Player is locked. Unlock to make changes.";
      return;
    }
    saving = true;
    saveMsg = "";
    try {
      // Save song config if dirty.
      if (configDirty) {
        const res = await updateSong(songName, editedYaml);
        if (!res.ok) {
          const data = await res.json().catch(() => null);
          saveMsg =
            data?.error ?? data?.errors?.[0] ?? `Save failed (${res.status})`;
          return;
        }
        rawYaml = editedYaml;
      }

      // Save lighting files if dirty.
      if (lightingDirty && lightingEditorRef) {
        await lightingEditorRef.saveLightingFiles();
      }

      saveMsg = "Saved";
      setTimeout(() => (saveMsg = ""), 2000);
      const result = await fetchSongs();
      song = result.songs.find((s) => s.name === songName) ?? null;
      // Clear failure banner if the song now loads successfully.
      const stillFailed = result.failures.find(
        (f: SongFailure) => f.name === songName,
      );
      failureError = stillFailed ? stillFailed.error : null;
    } catch (e) {
      saveMsg = e instanceof Error ? e.message : "Save failed";
    } finally {
      saving = false;
    }
  }

  async function handleTrackUpload(files: File[]) {
    if ($playbackStore.locked) {
      uploadMsg = "Player is locked. Unlock to make changes.";
      return;
    }

    // Check for existing files that would be overwritten.
    const existingNames = songFiles.map((f) => f.name);
    const conflicts = files.filter((f) => existingNames.includes(f.name));
    if (conflicts.length > 0) {
      const names = conflicts.map((f) => f.name).join(", ");
      if (
        !confirm(
          `The following file${conflicts.length > 1 ? "s" : ""} will be replaced:\n${names}\n\nContinue?`,
        )
      )
        return;
    }

    uploading = true;
    uploadMsg = "";
    try {
      let res: Response;
      if (files.length === 1) {
        res = await uploadTrack(songName, files[0]);
      } else {
        res = await uploadTracks(songName, files);
      }
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        uploadMsg = data?.error ?? `Upload failed (${res.status})`;
        return;
      }
      const data = await res.json().catch(() => null);
      if (data?.replaced) {
        uploadMsg = `Replaced ${files.length} file${files.length !== 1 ? "s" : ""}`;
      } else {
        uploadMsg = `Uploaded ${files.length} file${files.length !== 1 ? "s" : ""}`;
      }
      setTimeout(() => (uploadMsg = ""), 3000);
      await load();
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : "Upload failed";
    } finally {
      uploading = false;
    }
  }

  async function handleMidiUpload(files: File[]) {
    if ($playbackStore.locked) {
      uploadMsg = "Player is locked. Unlock to make changes.";
      return;
    }

    const existingNames = songFiles.map((f) => f.name);
    if (existingNames.includes(files[0].name)) {
      if (
        !confirm(
          `"${files[0].name}" already exists and will be replaced.\n\nContinue?`,
        )
      )
        return;
    }

    uploading = true;
    uploadMsg = "";
    try {
      const res = await uploadTrack(songName, files[0]);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        uploadMsg = data?.error ?? `Upload failed (${res.status})`;
        return;
      }
      const data = await res.json().catch(() => null);
      uploadMsg = data?.replaced ? "MIDI file replaced" : "MIDI file uploaded";
      setTimeout(() => (uploadMsg = ""), 3000);
      await load();
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : "Upload failed";
    } finally {
      uploading = false;
    }
  }

  let configDirty = $derived(editedYaml !== rawYaml);
  let anyDirty = $derived(configDirty || lightingDirty);
  let midiFile = $derived(
    parsedConfig
      ? ((parsedConfig.midi_playback as { file?: string })?.file ??
          (parsedConfig.midi_file as string | undefined) ??
          null)
      : null,
  );

  function tabHasIndicator(key: TabKey): boolean {
    if (key === "tracks") return tracks.length > 0;
    if (key === "midi") return !!midiFile;
    if (key === "lighting") return song?.has_lighting ?? false;
    if (key === "config") return configDirty;
    return false;
  }
</script>

<div class="detail">
  <a class="back-link" href="#/songs">&larr; All Songs</a>

  {#if loading}
    <div class="status">Loading...</div>
  {:else if error}
    <div class="status error">{error}</div>
  {:else}
    <div class="title-row">
      <h2 class="song-title">{songName}</h2>
      <div class="badges">
        {#if failureError}
          <span class="badge failed">ERROR</span>
        {/if}
        {#if song?.has_midi}
          <span class="badge midi">MIDI</span>
        {/if}
        {#if song && song.lighting_files.length > 0}
          <span class="badge lighting">LIGHT</span>
        {/if}
        {#if song && song.midi_dmx_files.length > 0}
          <span class="badge midi-dmx">MIDI DMX</span>
        {/if}
      </div>
    </div>

    {#if failureError}
      <div class="failure-banner">
        <strong
          >This song failed to load and will not be playable until it's valid.</strong
        >
        <div class="failure-detail">{failureError}</div>
      </div>
    {/if}

    {#if song && !failureError}
      <div class="meta">
        <span>{song.duration_display}</span>
        <span>{song.track_count} track{song.track_count !== 1 ? "s" : ""}</span>
        {#if song.sample_format}
          <span>{song.num_channels}ch {song.sample_format}</span>
        {/if}
      </div>
    {/if}

    <!-- Tab bar with inline save -->
    <div class="tab-bar" role="tablist">
      {#each tabs as tab_item (tab_item.key)}
        <button
          class="tab"
          class:active={activeTab === tab_item.key}
          role="tab"
          aria-selected={activeTab === tab_item.key}
          onclick={() => setTab(tab_item.key)}
        >
          {tab_item.label}
          {#if tabHasIndicator(tab_item.key)}
            <span
              class="tab-dot"
              class:config-dot={tab_item.key === "config" && configDirty}
            ></span>
          {/if}
        </button>
      {/each}
      <div class="tab-save">
        {#if saveMsg}
          <span class="save-msg" class:error={saveMsg !== "Saved"}
            >{saveMsg}</span
          >
        {/if}
        {#if anyDirty}
          <span class="unsaved">Unsaved</span>
        {/if}
        <button
          class="btn btn-primary btn-sm"
          onclick={save}
          disabled={saving || !anyDirty}
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </div>

    <!-- Tab content -->
    <div class="tab-content">
      {#if activeTab === "tracks"}
        <TrackEditor
          {tracks}
          files={songFiles}
          {waveformTracks}
          onchange={onTracksChange}
          onbrowse={(index) => openBrowser({ kind: "track", index })}
        />
        <div class="upload-area">
          <FileUpload
            accept=".wav,.flac,.mp3,.ogg,.aac,.m4a,.mp4,.aiff,.aif"
            label={uploading
              ? "Uploading..."
              : "Drop audio files here or click to upload"}
            multiple={true}
            onupload={handleTrackUpload}
          />
        </div>
        {#if uploadMsg}
          <div
            class="msg"
            class:error={uploadMsg.toLowerCase().includes("fail") ||
              uploadMsg.toLowerCase().includes("locked")}
          >
            {uploadMsg}
          </div>
        {/if}
      {:else if activeTab === "midi"}
        {#if midiFile}
          <div class="feature-row">
            <span class="feature-label">Current MIDI file:</span>
            <span class="feature-value">{midiFile}</span>
          </div>
        {:else}
          <p class="muted">No MIDI file configured for this song.</p>
        {/if}
        <FilePicker
          files={songFiles}
          fileType="midi"
          label="Use existing MIDI file from song directory"
          onpick={setMidiFile}
        />
        <div class="browse-row">
          <button class="btn" onclick={() => openBrowser({ kind: "midi" })}>
            Browse Filesystem...
          </button>
        </div>
        <div class="upload-area">
          <FileUpload
            accept=".mid"
            label={uploading
              ? "Uploading..."
              : "Drop .mid file here or click to upload"}
            onupload={handleMidiUpload}
          />
        </div>
        {#if uploadMsg}
          <div
            class="msg"
            class:error={uploadMsg.toLowerCase().includes("fail") ||
              uploadMsg.toLowerCase().includes("locked")}
          >
            {uploadMsg}
          </div>
        {/if}
      {:else if activeTab === "lighting"}
        {#if song}
          <SongLightingEditor
            bind:this={lightingEditorRef}
            bind:dirty={lightingDirty}
            {song}
            onreload={load}
          />
        {/if}
      {:else if activeTab === "config"}
        <div class="config-section">
          <textarea
            class="config-editor"
            value={editedYaml}
            oninput={(e) => onRawYamlInput(e.currentTarget.value)}
          ></textarea>
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if browseTarget}
  <div class="browser-overlay">
    <div class="browser-modal">
      <FileBrowser
        filter={browseFilter}
        multiple={browseMultiple}
        onselect={onBrowseSelect}
        oncancel={closeBrowser}
      />
    </div>
  </div>
{/if}

<style>
  .browser-overlay {
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
  .browser-modal {
    width: 100%;
    max-width: 700px;
    max-height: 90vh;
    overflow: hidden;
  }
  .browse-row {
    margin-top: 8px;
  }
  .detail {
    margin: 0 auto;
  }
  .back-link {
    display: inline-block;
    margin-bottom: 12px;
    font-size: 14px;
    color: var(--accent);
    text-decoration: none;
  }
  .back-link:hover {
    text-decoration: underline;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 4px;
  }
  .song-title {
    font-size: 22px;
    font-weight: 600;
  }
  .badges {
    display: flex;
    gap: 4px;
  }
  .badge {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.5px;
    padding: 2px 8px;
    border-radius: 3px;
    line-height: 1.2;
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
  .failure-banner {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: var(--radius);
    padding: 12px 16px;
    margin-bottom: 16px;
    font-size: 14px;
    color: var(--text);
  }
  .failure-detail {
    margin-top: 6px;
    font-size: 13px;
    color: var(--red);
    font-family: var(--mono);
  }
  .meta {
    display: flex;
    gap: 16px;
    font-size: 14px;
    color: var(--text-muted);
    margin-bottom: 16px;
  }
  .tab-bar {
    display: flex;
    align-items: center;
    gap: 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    margin-bottom: 16px;
    position: sticky;
    top: 48px;
    z-index: 10;
    background: var(--bg);
    padding-top: 4px;
  }
  .tab {
    position: relative;
    padding: 10px 16px;
    font-size: 14px;
    font-weight: 500;
    font-family: var(--sans);
    color: var(--text-muted);
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
    white-space: nowrap;
    transition:
      color 0.15s,
      border-color 0.15s;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .tab-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--green);
    margin-left: 6px;
    vertical-align: middle;
  }
  .tab-dot.config-dot {
    background: var(--accent);
  }
  .tab-save {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .unsaved {
    font-size: 12px;
    color: var(--accent);
  }
  .save-msg {
    font-size: 12px;
    color: var(--green);
  }
  .save-msg.error {
    color: var(--red);
  }
  .tab-content {
    min-height: 200px;
  }
  .upload-area {
    margin-top: 8px;
  }
  .msg {
    font-size: 13px;
    color: var(--green);
    margin-top: 8px;
  }
  .msg.error {
    color: var(--red);
  }
  .muted {
    color: var(--text-muted);
    font-size: 14px;
    margin-bottom: 12px;
  }
  .feature-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0;
    margin-bottom: 8px;
    font-size: 14px;
  }
  .feature-label {
    color: var(--text-muted);
  }
  .feature-value {
    font-family: var(--mono);
    color: var(--text);
  }
  .config-section {
    display: flex;
    flex-direction: column;
  }
  .config-editor {
    width: 100%;
    min-height: 400px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-input);
    color: var(--text);
    font-family: var(--mono);
    font-size: 14px;
    line-height: 1.5;
    resize: vertical;
    outline: none;
  }
  .config-editor:focus {
    border-color: var(--border-focus);
  }
  .status {
    text-align: center;
    padding: 48px 16px;
    color: var(--text-muted);
  }
  .status.error {
    color: var(--red);
  }
  @media (max-width: 600px) {
    .tab {
      padding: 8px 12px;
      font-size: 13px;
    }
  }
</style>
