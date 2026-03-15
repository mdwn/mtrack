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
    type SongSummary,
    type WaveformTrack,
  } from "../../lib/api/songs";
  import FileBrowser from "./FileBrowser.svelte";
  import FileUpload from "./FileUpload.svelte";
  import FilePicker from "./FilePicker.svelte";
  import TrackEditor from "./TrackEditor.svelte";

  interface TrackEntry {
    name: string;
    file: string;
    file_channel?: number;
  }

  interface Props {
    songName: string;
  }

  let { songName }: Props = $props();

  let song = $state<SongSummary | null>(null);
  let songFiles = $state<SongFile[]>([]);
  let waveformTracks = $state<WaveformTrack[]>([]);
  let rawYaml = $state("");
  let editedYaml = $state("");
  let parsedConfig = $state<Record<string, unknown> | null>(null);
  let tracks = $state<TrackEntry[]>([]);
  let loading = $state(true);
  let error = $state("");
  let saving = $state(false);
  let saveMsg = $state("");
  let uploading = $state(false);
  let uploadMsg = $state("");
  let showRawYaml = $state(false);

  // File browser state: null = closed, otherwise describes what we're browsing for
  type BrowseTarget =
    | { kind: "track"; index: number } // index = -1 means "add new tracks"
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
        // Replace file for existing track
        const idx = target.index;
        const updated = tracks.map((t, i) =>
          i === idx ? { ...t, file: paths[0] } : t,
        );
        onTracksChange(updated);
      } else {
        // Add new tracks from selected files
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
      tracks: tracks.map((t) => {
        const entry: Record<string, unknown> = { name: t.name, file: t.file };
        if (t.file_channel !== undefined && t.file_channel !== 1) {
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
    try {
      const [songs, yaml] = await Promise.all([
        fetchSongs(),
        fetchSongConfig(songName),
      ]);
      song = songs.find((s) => s.name === songName) ?? null;
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
    // Add if not already present
    if (!existing.some((l) => l.file === filename)) {
      parsedConfig = {
        ...parsedConfig,
        lighting: [...existing, { file: filename }],
      };
      editedYaml = buildYaml();
    }
  }

  async function save() {
    saving = true;
    saveMsg = "";
    try {
      const res = await updateSong(songName, editedYaml);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        saveMsg =
          data?.error ?? data?.errors?.[0] ?? `Save failed (${res.status})`;
        return;
      }
      rawYaml = editedYaml;
      saveMsg = "Saved";
      setTimeout(() => (saveMsg = ""), 2000);
      // Re-fetch to update summary
      const songs = await fetchSongs();
      song = songs.find((s) => s.name === songName) ?? null;
    } catch (e) {
      saveMsg = e instanceof Error ? e.message : "Save failed";
    } finally {
      saving = false;
    }
  }

  async function handleTrackUpload(files: File[]) {
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
      uploadMsg = `Uploaded ${files.length} file${files.length !== 1 ? "s" : ""}`;
      setTimeout(() => (uploadMsg = ""), 3000);
      await load();
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : "Upload failed";
    } finally {
      uploading = false;
    }
  }

  async function handleMidiUpload(files: File[]) {
    uploading = true;
    uploadMsg = "";
    try {
      const res = await uploadTrack(songName, files[0]);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        uploadMsg = data?.error ?? `Upload failed (${res.status})`;
        return;
      }
      uploadMsg = "MIDI file uploaded";
      setTimeout(() => (uploadMsg = ""), 3000);
      await load();
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : "Upload failed";
    } finally {
      uploading = false;
    }
  }

  async function handleLightingUpload(files: File[]) {
    uploading = true;
    uploadMsg = "";
    try {
      const res = await uploadTrack(songName, files[0]);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        uploadMsg = data?.error ?? `Upload failed (${res.status})`;
        return;
      }
      uploadMsg = "Lighting file uploaded";
      setTimeout(() => (uploadMsg = ""), 3000);
      await load();
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : "Upload failed";
    } finally {
      uploading = false;
    }
  }

  let configDirty = $derived(editedYaml !== rawYaml);
  let midiFile = $derived(
    parsedConfig
      ? ((parsedConfig.midi_playback as { file?: string })?.file ??
          (parsedConfig.midi_file as string | undefined) ??
          null)
      : null,
  );
  let lightingFiles = $derived(
    parsedConfig
      ? ((parsedConfig.lighting as { file: string }[] | undefined) ?? []).map(
          (l) => l.file,
        )
      : [],
  );
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
        {#if song?.has_midi}
          <span class="badge midi">MIDI</span>
        {/if}
        {#if song?.has_lighting}
          <span class="badge lighting">LIGHT</span>
        {/if}
      </div>
    </div>

    {#if song}
      <div class="meta">
        <span>{song.duration_display}</span>
        <span>{song.track_count} track{song.track_count !== 1 ? "s" : ""}</span>
        {#if song.sample_format}
          <span>{song.num_channels}ch {song.sample_format}</span>
        {/if}
      </div>
    {/if}

    <!-- Save bar -->
    <div class="save-bar" class:dirty={configDirty}>
      <div class="save-info">
        {#if saveMsg}
          <span class="save-msg" class:error={saveMsg !== "Saved"}
            >{saveMsg}</span
          >
        {/if}
        {#if configDirty}
          <span class="unsaved">Unsaved changes</span>
        {/if}
      </div>
      <button
        class="btn btn-primary"
        onclick={save}
        disabled={saving || !configDirty}
      >
        {saving ? "Saving..." : "Save"}
      </button>
    </div>

    <!-- Tracks -->
    <div class="card section">
      <div class="card-header">
        <span class="card-title">Audio Tracks</span>
      </div>
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
    </div>

    <!-- MIDI -->
    <div class="card section" class:has-feature={!!midiFile}>
      <div class="card-header">
        <span class="card-title">MIDI</span>
        {#if midiFile}
          <span class="feature-indicator midi">{midiFile}</span>
        {/if}
      </div>
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
    </div>

    <!-- Lighting -->
    <div class="card section" class:has-feature={lightingFiles.length > 0}>
      <div class="card-header">
        <span class="card-title">Lighting</span>
        {#if lightingFiles.length > 0}
          <span class="feature-indicator lighting"
            >{lightingFiles.length} show{lightingFiles.length !== 1
              ? "s"
              : ""}</span
          >
        {/if}
      </div>
      {#if lightingFiles.length > 0}
        <ul class="ref-list">
          {#each lightingFiles as file (file)}
            <li>{file}</li>
          {/each}
        </ul>
      {/if}
      <FilePicker
        files={songFiles}
        fileType="lighting"
        label="Use existing lighting file from song directory"
        onpick={setLightingFile}
      />
      <div class="browse-row">
        <button class="btn" onclick={() => openBrowser({ kind: "lighting" })}>
          Browse Filesystem...
        </button>
      </div>
      <div class="upload-area">
        <FileUpload
          accept=".light"
          label={uploading
            ? "Uploading..."
            : "Drop .light file here or click to upload"}
          onupload={handleLightingUpload}
        />
      </div>
    </div>

    {#if uploadMsg}
      <div class="msg" class:error={uploadMsg.toLowerCase().includes("fail")}>
        {uploadMsg}
      </div>
    {/if}

    <!-- Config YAML -->
    <div class="card section">
      <div class="card-header">
        <span class="card-title">Configuration (YAML)</span>
        <button class="btn" onclick={() => (showRawYaml = !showRawYaml)}>
          {showRawYaml ? "Hide" : "Edit Raw"}
        </button>
      </div>
      {#if showRawYaml}
        <textarea
          class="config-editor"
          value={editedYaml}
          oninput={(e) => onRawYamlInput(e.currentTarget.value)}
        ></textarea>
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
    max-width: 800px;
    margin: 0 auto;
  }
  .back-link {
    display: inline-block;
    margin-bottom: 12px;
    font-size: 13px;
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
    font-size: 10px;
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
  .meta {
    display: flex;
    gap: 16px;
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 16px;
  }
  .save-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    margin-bottom: 16px;
    border-radius: var(--radius);
    background: var(--bg-card);
    border: 1px solid var(--border);
    transition: border-color 0.15s;
    position: sticky;
    top: 48px;
    z-index: 10;
  }
  .save-bar.dirty {
    border-color: var(--accent);
  }
  .save-info {
    display: flex;
    align-items: center;
    gap: 10px;
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
  .section {
    margin-bottom: 16px;
  }
  .has-feature {
    border-color: var(--text-dim);
  }
  .feature-indicator {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.3px;
    padding: 1px 6px;
    border-radius: 3px;
    font-family: var(--mono);
  }
  .feature-indicator.midi {
    background: rgba(59, 130, 246, 0.15);
    color: var(--blue);
  }
  .feature-indicator.lighting {
    background: rgba(234, 179, 8, 0.15);
    color: var(--yellow);
  }
  .ref-list {
    list-style: none;
    margin-bottom: 8px;
  }
  .ref-list li {
    padding: 4px 8px;
    font-size: 12px;
    font-family: var(--mono);
    color: var(--text);
    border-bottom: 1px solid var(--border);
  }
  .ref-list li:last-child {
    border-bottom: none;
  }
  .upload-area {
    margin-top: 8px;
  }
  .msg {
    font-size: 12px;
    color: var(--green);
    margin-bottom: 12px;
  }
  .msg.error {
    color: var(--red);
  }
  .config-editor {
    width: 100%;
    min-height: 300px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-input);
    color: var(--text);
    font-family: var(--mono);
    font-size: 13px;
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
</style>
