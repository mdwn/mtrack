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
  import { showConfirm } from "../../lib/dialog.svelte";
  import { registerDirtyGuard } from "../../lib/dirtyGuard";
  import { playbackStore } from "../../lib/ws/stores";
  import FileBrowser from "./FileBrowser.svelte";
  import FileUpload from "./FileUpload.svelte";
  import FilePicker from "./FilePicker.svelte";
  import TrackEditor from "./TrackEditor.svelte";
  import SongLightingEditor from "./SongLightingEditor.svelte";
  import SectionTimelineEditor from "./SectionTimelineEditor.svelte";
  import SamplesSection from "../config/SamplesSection.svelte";
  import type { SampleBrowseTarget } from "../config/SamplesSection.svelte";
  import MidiEventEditor from "../config/MidiEventEditor.svelte";
  import type { MidiEvent } from "../config/MidiEventEditor.svelte";
  import NotificationsSection from "../config/NotificationsSection.svelte";
  import type { NotifBrowseTarget } from "../config/NotificationsSection.svelte";

  interface TrackEntry {
    name: string;
    file: string;
    file_channel?: number;
  }

  /* eslint-disable @typescript-eslint/no-explicit-any */

  interface Props {
    songName: string;
    initialTab?:
      | "tracks"
      | "midi"
      | "samples"
      | "lighting"
      | "notifications"
      | "sections"
      | "config";
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
  let saveOk = $state(false);
  let uploading = $state(false);
  let uploadMsg = $state("");
  let uploadOk = $state(false);

  // Lighting editor state (lifted up for unified save)
  let lightingDirty = $state(false);
  let lightingEditorRef: SongLightingEditor | undefined = $state();

  // Tab state
  type TabKey = "tracks" | "samples" | "sections" | "lighting" | "config";
  function getInitialTab(): TabKey {
    // Redirect legacy tab names to their merged destinations
    if (initialTab === "midi") return "tracks";
    if (initialTab === "notifications") return "config";
    return (initialTab as TabKey) ?? "tracks";
  }
  let activeTab = $state<TabKey>(getInitialTab());

  // Collapsible MIDI section within Tracks tab
  let midiSectionOpen = $state(false);

  // Collapsible Notifications section within Config tab
  let notifSectionOpen = $state(false);

  function setTab(tab: TabKey) {
    activeTab = tab;
    const base = `#/songs/${encodeURIComponent(songName)}`;
    window.location.hash = tab === "tracks" ? base : `${base}/${tab}`;
  }

  const tabs: { key: TabKey; labelKey: string }[] = [
    { key: "tracks", labelKey: "songs.detail.tabs.tracks" },
    { key: "samples", labelKey: "songs.detail.tabs.samples" },
    { key: "sections", labelKey: "songs.detail.tabs.sections" },
    { key: "lighting", labelKey: "songs.detail.tabs.lighting" },
    { key: "config", labelKey: "songs.detail.tabs.config" },
  ];

  // Per-song samples state
  let songSamples = $state<Record<string, any>>({});
  let songSamplesRef: SamplesSection | undefined = $state();
  let sampleBrowseTarget = $state<SampleBrowseTarget | null>(null);

  // MIDI event state
  let midiEvent = $state<MidiEvent | null>(null);

  // Notification audio state
  let notificationAudio = $state<Record<string, unknown>>({});

  // Loop playback state
  let loopPlayback = $state(false);

  // Section editor state
  let sections = $state<
    { name: string; start_measure: number; end_measure: number }[]
  >([]);
  let sectionsDirty = $state(false);

  // File browser state
  type BrowseTarget =
    | { kind: "track"; index: number }
    | { kind: "midi" }
    | { kind: "lighting" }
    | { kind: "sample" }
    | { kind: "notification" };
  let browseTarget = $state<BrowseTarget | null>(null);
  let notifBrowseTarget = $state<NotifBrowseTarget | null>(null);
  let songNotifRef: NotificationsSection | undefined = $state();

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
    } else if (browseTarget.kind === "sample") {
      onSampleBrowseSelect(paths);
      return;
    } else if (browseTarget.kind === "notification") {
      onNotifBrowseSelect(paths);
      return;
    }
    browseTarget = null;
  }

  let browseFilter = $derived.by(() => {
    if (!browseTarget) return [];
    if (browseTarget.kind === "track") return ["audio"];
    if (browseTarget.kind === "midi") return ["midi"];
    if (browseTarget.kind === "lighting") return ["lighting"];
    if (browseTarget.kind === "sample") return ["audio"];
    if (browseTarget.kind === "notification") return ["audio"];
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
        midiEvent = (parsed.midi_event as MidiEvent) ?? null;
        loopPlayback = parsed.loop_playback === true;
        sections = (parsed.sections ?? []).map(
          (s: Record<string, unknown>) => ({
            name: (s.name as string) ?? "",
            start_measure: (s.start_measure as number) ?? 1,
            end_measure: (s.end_measure as number) ?? 2,
          }),
        );
        const s = parsed.samples;
        songSamples =
          s && typeof s === "object" ? (s as Record<string, any>) : {};
        const na = parsed.notification_audio;
        notificationAudio =
          na && typeof na === "object" ? (na as Record<string, unknown>) : {};
      }
    } catch {
      parsedConfig = null;
      tracks = [];
      midiEvent = null;
      loopPlayback = false;
      sections = [];
      songSamples = {};
      notificationAudio = {};
    }
  }

  function buildYaml(): string {
    if (!parsedConfig) return editedYaml;
    const updated: Record<string, unknown> = {
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
    if (midiEvent) {
      updated.midi_event = midiEvent;
    } else {
      delete updated.midi_event;
    }
    if (loopPlayback) {
      updated.loop_playback = true;
    } else {
      delete updated.loop_playback;
    }
    if (sections.length > 0) {
      updated.sections = sections;
    } else {
      delete updated.sections;
    }
    if (Object.keys(songSamples).length > 0) {
      updated.samples = songSamples;
    } else {
      delete updated.samples;
    }
    if (Object.keys(notificationAudio).length > 0) {
      updated.notification_audio = notificationAudio;
    } else {
      delete updated.notification_audio;
    }
    const lightingEntries = parsedConfig.lighting as
      | { file: string }[]
      | undefined;
    if (lightingEntries && lightingEntries.length > 0) {
      updated.lighting = lightingEntries;
    } else {
      delete updated.lighting;
    }
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
      error =
        e instanceof Error
          ? e.message
          : get(t)("songs.detail.failedToLoadSong");
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
    // If midi_playback already exists, update its file; otherwise use legacy midi_file.
    const mp = parsedConfig.midi_playback as
      | Record<string, unknown>
      | undefined;
    if (mp) {
      parsedConfig = {
        ...parsedConfig,
        midi_playback: { ...mp, file: filename },
      };
    } else {
      parsedConfig = { ...parsedConfig, midi_file: filename };
    }
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

  function removeLightingFile(filename: string) {
    if (!parsedConfig) return;
    const existing =
      (parsedConfig.lighting as { file: string }[] | undefined) ?? [];
    const filtered = existing.filter((l) => l.file !== filename);
    parsedConfig = {
      ...parsedConfig,
      lighting: filtered.length > 0 ? filtered : undefined,
    };
    editedYaml = buildYaml();
  }

  function onMidiEventChange() {
    editedYaml = buildYaml();
  }

  function addMidiEvent() {
    midiEvent = { type: "program_change", channel: 1, program: 0 };
    editedYaml = buildYaml();
  }

  function removeMidiEvent() {
    midiEvent = null;
    editedYaml = buildYaml();
  }

  // When sections become dirty, rebuild YAML to update configDirty.
  $effect(() => {
    if (sectionsDirty) {
      editedYaml = buildYaml();
      sectionsDirty = false;
    }
  });

  function onSongSamplesChange() {
    editedYaml = buildYaml();
  }

  function onSampleBrowse(target: SampleBrowseTarget) {
    sampleBrowseTarget = target;
    browseTarget = { kind: "sample" };
  }

  function onSampleBrowseSelect(paths: string[]) {
    if (paths.length > 0 && sampleBrowseTarget && songSamplesRef) {
      songSamplesRef.applyBrowseResult(sampleBrowseTarget, paths[0]);
    }
    sampleBrowseTarget = null;
    browseTarget = null;
  }

  async function save() {
    if ($playbackStore.locked) {
      saveMsg = get(t)("common.locked");
      saveOk = false;
      return;
    }
    saving = true;
    saveMsg = "";
    saveOk = false;
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

      saveMsg = get(t)("common.saved");
      saveOk = true;
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
      uploadMsg = get(t)("common.locked");
      uploadOk = false;
      return;
    }

    // Check for existing files that would be overwritten.
    const existingNames = songFiles.map((f) => f.name);
    const conflicts = files.filter((f) => existingNames.includes(f.name));
    if (conflicts.length > 0) {
      const names = conflicts.map((f) => f.name).join(", ");
      if (
        !(await showConfirm(
          get(t)("songs.detail.confirmReplace", {
            values: { count: conflicts.length, names },
          }),
        ))
      )
        return;
    }

    uploading = true;
    uploadMsg = "";
    uploadOk = false;
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
        uploadMsg = get(t)("songs.detail.replacedFiles", {
          values: { count: files.length },
        });
      } else {
        uploadMsg = get(t)("songs.detail.uploadedFiles", {
          values: { count: files.length },
        });
      }
      uploadOk = true;
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
      uploadMsg = get(t)("common.locked");
      uploadOk = false;
      return;
    }

    const existingNames = songFiles.map((f) => f.name);
    if (existingNames.includes(files[0].name)) {
      if (
        !(await showConfirm(
          get(t)("songs.detail.confirmMidiReplace", {
            values: { name: files[0].name },
          }),
        ))
      )
        return;
    }

    uploading = true;
    uploadMsg = "";
    uploadOk = false;
    try {
      const res = await uploadTrack(songName, files[0]);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        uploadMsg = data?.error ?? `Upload failed (${res.status})`;
        return;
      }
      const data = await res.json().catch(() => null);
      uploadMsg = data?.replaced
        ? get(t)("songs.detail.midiReplaced")
        : get(t)("songs.detail.midiUploaded");
      uploadOk = true;
      setTimeout(() => (uploadMsg = ""), 3000);
      await load();
    } catch (e) {
      uploadMsg = e instanceof Error ? e.message : "Upload failed";
    } finally {
      uploading = false;
    }
  }

  let configDirty = $derived(editedYaml !== rawYaml);
  let anyDirty = $derived(configDirty || lightingDirty || sectionsDirty);

  $effect(() => {
    // Block in-app hash navigation while edits are pending.
    return registerDirtyGuard(
      () => anyDirty,
      get(t)("songs.detail.discardUnsaved"),
    );
  });

  $effect(() => {
    // Backstop for tab close / refresh.
    if (anyDirty) {
      const handler = (e: BeforeUnloadEvent) => {
        e.preventDefault();
      };
      window.addEventListener("beforeunload", handler);
      return () => window.removeEventListener("beforeunload", handler);
    }
  });
  let midiFile = $derived(
    parsedConfig
      ? ((parsedConfig.midi_playback as { file?: string })?.file ??
          (parsedConfig.midi_file as string | undefined) ??
          null)
      : null,
  );

  let excludeMidiChannels = $derived<number[]>(
    parsedConfig
      ? ((
          parsedConfig.midi_playback as {
            exclude_midi_channels?: number[];
          }
        )?.exclude_midi_channels ?? [])
      : [],
  );

  function setExcludeChannels(next: number[]) {
    if (!parsedConfig) return;
    const sorted = [...new Set(next)].sort((a, b) => a - b);

    // Ensure we use midi_playback format (upgrade from legacy midi_file if needed).
    const mp = (parsedConfig.midi_playback as Record<string, unknown>) ?? {};
    const file =
      (mp.file as string | undefined) ??
      (parsedConfig.midi_file as string | undefined);
    const updated: Record<string, unknown> = { ...parsedConfig };
    if (sorted.length > 0) {
      updated.midi_playback = {
        ...mp,
        ...(file ? { file } : {}),
        exclude_midi_channels: sorted,
      };
    } else {
      // No excluded channels — remove the field from midi_playback.
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { exclude_midi_channels: _removed, ...rest } = mp;
      if (Object.keys(rest).length > 0 || !file) {
        updated.midi_playback = rest;
      } else {
        // If only file remains and was originally midi_file, keep it simple.
        delete updated.midi_playback;
        updated.midi_file = file;
      }
    }
    // Clean up legacy midi_file if we have midi_playback with a file.
    if (
      updated.midi_playback &&
      (updated.midi_playback as Record<string, unknown>).file
    ) {
      delete updated.midi_file;
    }
    parsedConfig = updated;
    editedYaml = buildYaml();
  }

  function toggleExcludeChannel(channel: number) {
    if (!parsedConfig) return;
    const current = [...excludeMidiChannels];
    const idx = current.indexOf(channel);
    if (idx >= 0) current.splice(idx, 1);
    else current.push(channel);
    setExcludeChannels(current);
  }

  /** Exclude every channel — silences all MIDI playback. */
  function excludeAllChannels() {
    setExcludeChannels(Array.from({ length: 16 }, (_, i) => i + 1));
  }

  /** Reset to no exclusions — every channel plays. */
  function excludeNoChannels() {
    setExcludeChannels([]);
  }

  /**
   * Exclude every channel except 10 (the General MIDI drum channel) —
   * the common preset for live shows where mtrack runs the drums and
   * everything else is played live.
   */
  function excludeAllButDrums() {
    setExcludeChannels(
      Array.from({ length: 16 }, (_, i) => i + 1).filter((c) => c !== 10),
    );
  }

  function tabHasIndicator(key: TabKey): boolean {
    if (key === "tracks") return tracks.length > 0 || !!midiFile || !!midiEvent;
    if (key === "samples") return Object.keys(songSamples).length > 0;
    if (key === "lighting") return song?.has_lighting ?? false;
    if (key === "config")
      return configDirty || Object.keys(notificationAudio).length > 0;
    return false;
  }

  function onNotificationAudioChange() {
    editedYaml = buildYaml();
  }

  function onNotifBrowse(target: NotifBrowseTarget) {
    notifBrowseTarget = target;
    browseTarget = { kind: "notification" };
  }

  function onNotifBrowseSelect(paths: string[]) {
    if (paths.length > 0 && notifBrowseTarget && songNotifRef) {
      songNotifRef.applyBrowseResult(notifBrowseTarget, paths[0]);
    }
    notifBrowseTarget = null;
    browseTarget = null;
  }

  let notifUploading = $state(false);
  let notifUploadMsg = $state("");

  async function onNotifUpload(files: File[]) {
    if (files.length === 0) return;
    if ($playbackStore.locked) {
      notifUploadMsg = get(t)("common.locked");
      return;
    }
    notifUploading = true;
    notifUploadMsg = "";
    try {
      const res = await uploadTrack(songName, files[0]);
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        notifUploadMsg = data?.error ?? `Upload failed (${res.status})`;
        return;
      }
      notifUploadMsg = get(t)("notifications.uploaded", {
        values: { name: files[0].name },
      });
      setTimeout(() => (notifUploadMsg = ""), 3000);
      // Refresh song files so the file is available
      songFiles = await fetchSongFiles(songName);
    } catch (e: unknown) {
      notifUploadMsg = e instanceof Error ? e.message : String(e);
    } finally {
      notifUploading = false;
    }
  }
</script>

<div class="detail">
  <a class="back-link" href="#/songs" aria-label={$t("songs.detail.allSongs")}
    >{$t("songs.detail.allSongs")}</a
  >

  {#if loading}
    <div class="status">{$t("common.loading")}</div>
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
        {#if song?.loop_playback}
          <span class="badge loop">LOOP</span>
        {/if}
      </div>
    </div>

    {#if failureError}
      <div class="failure-banner">
        <strong>{$t("songs.detail.failedToLoad")}</strong>
        <div class="failure-detail">{failureError}</div>
      </div>
    {/if}

    {#if song && !failureError}
      <div class="meta">
        <span>{song.duration_display}</span>
        <span
          >{$t("songs.trackCount", {
            values: { count: song.track_count },
          })}</span
        >
        {#if song.sample_format}
          <span>{song.num_channels}ch {song.sample_format}</span>
        {/if}
      </div>

      {#if song.beat_grid}
        <div class="beat-grid-summary">
          <span class="beat-grid-label">Beat Grid:</span>
          <span class="beat-grid-stat">{song.beat_grid.beats.length} beats</span
          >
          <span class="beat-grid-stat"
            >{song.beat_grid.measure_starts.length} measures</span
          >
        </div>
      {/if}

      <div class="field">
        <label for="loop-playback">
          <input
            id="loop-playback"
            type="checkbox"
            checked={loopPlayback}
            onchange={(e) => {
              loopPlayback = (e.currentTarget as HTMLInputElement).checked;
              editedYaml = buildYaml();
            }}
          />
          {$t("songs.detail.loopPlayback")}
        </label>
        <span class="field-hint">{$t("songs.detail.loopPlaybackHint")}</span>
      </div>
    {/if}

    <!-- Tab bar with inline save -->
    <div class="tab-bar" role="tablist">
      {#each tabs as tab_item (tab_item.key)}
        <button
          class="tab"
          class:active={activeTab === tab_item.key}
          role="tab"
          id="song-tab-{tab_item.key}"
          aria-selected={activeTab === tab_item.key}
          aria-controls="song-tabpanel-{tab_item.key}"
          tabindex={activeTab === tab_item.key ? 0 : -1}
          onclick={() => setTab(tab_item.key)}
        >
          {$t(tab_item.labelKey)}
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
          <span class="save-msg" class:error={!saveOk}>{saveMsg}</span>
        {/if}
        {#if anyDirty}
          <span class="unsaved">{$t("common.unsaved")}</span>
        {/if}
        <button
          class="btn btn-sm"
          class:btn-primary={anyDirty && !$playbackStore.locked}
          onclick={save}
          disabled={saving || !anyDirty || $playbackStore.locked}
          title={$playbackStore.locked ? $t("common.locked") : null}
        >
          {saving ? $t("common.saving") : $t("common.save")}
        </button>
      </div>
    </div>

    <!-- Tab content -->
    <div
      class="tab-content"
      role="tabpanel"
      id="song-tabpanel-{activeTab}"
      aria-labelledby="song-tab-{activeTab}"
    >
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
              ? $t("common.uploading")
              : $t("songs.detail.dropAudio")}
            multiple={true}
            onupload={handleTrackUpload}
          />
        </div>
        {#if uploadMsg}
          <div class="msg" class:error={!uploadOk}>
            {uploadMsg}
          </div>
        {/if}

        <!-- Collapsible MIDI section -->
        <div class="collapsible-section">
          <button
            class="collapsible-header"
            onclick={() => (midiSectionOpen = !midiSectionOpen)}
          >
            <span class="collapsible-chevron" class:open={midiSectionOpen}
              >&#9662;</span
            >
            <span class="section-label">{$t("songs.detail.tabs.midi")}</span>
            {#if midiFile || midiEvent}
              <span class="tab-dot"></span>
            {/if}
          </button>
          {#if midiSectionOpen}
            <div class="collapsible-body">
              {#if midiFile}
                <div class="feature-row">
                  <span class="feature-label"
                    >{$t("songs.detail.currentMidiFile")}</span
                  >
                  <span class="feature-value">{midiFile}</span>
                </div>
              {:else}
                <p class="muted">{$t("songs.detail.noMidi")}</p>
              {/if}
              <FilePicker
                files={songFiles}
                fileType="midi"
                label={$t("songs.detail.useExistingMidi")}
                onpick={setMidiFile}
              />
              <div class="browse-row">
                <button
                  class="btn"
                  onclick={() => openBrowser({ kind: "midi" })}
                >
                  {$t("samples.browseFilesystem")}
                </button>
              </div>
              <div class="upload-area">
                <FileUpload
                  accept=".mid"
                  label={uploading
                    ? $t("common.uploading")
                    : $t("songs.detail.dropMidi")}
                  onupload={handleMidiUpload}
                />
              </div>

              {#if midiFile}
                <div class="midi-event-section">
                  <div class="section-header">
                    <span class="section-label"
                      >{$t("songs.detail.excludeChannels")}</span
                    >
                  </div>
                  <p class="muted hint-text">
                    {$t("songs.detail.excludeChannelsHint")}
                  </p>
                  <div class="channel-presets">
                    <button
                      type="button"
                      class="channel-preset"
                      onclick={excludeNoChannels}
                      disabled={excludeMidiChannels.length === 0}
                      >{$t("songs.detail.excludeNone")}</button
                    >
                    <button
                      type="button"
                      class="channel-preset"
                      onclick={excludeAllChannels}
                      disabled={excludeMidiChannels.length === 16}
                      >{$t("songs.detail.excludeAll")}</button
                    >
                    <button
                      type="button"
                      class="channel-preset"
                      onclick={excludeAllButDrums}
                      title={$t("songs.detail.excludeAllButDrumsHint")}
                      >{$t("songs.detail.excludeAllButDrums")}</button
                    >
                  </div>
                  <div class="channel-grid">
                    {#each Array.from({ length: 16 }, (_, i) => i + 1) as ch (ch)}
                      <label
                        class="channel-toggle"
                        class:excluded={excludeMidiChannels.includes(ch)}
                      >
                        <input
                          type="checkbox"
                          checked={excludeMidiChannels.includes(ch)}
                          onchange={() => toggleExcludeChannel(ch)}
                        />
                        {ch}
                      </label>
                    {/each}
                  </div>
                </div>
              {/if}

              <div class="midi-event-section">
                <div class="section-header">
                  <span class="section-label"
                    >{$t("songs.detail.midiEvent")}</span
                  >
                  {#if !midiEvent}
                    <button class="btn btn-sm" onclick={addMidiEvent}
                      >{$t("songs.detail.addMidiEvent")}</button
                    >
                  {:else}
                    <button
                      class="btn btn-sm btn-danger"
                      onclick={removeMidiEvent}
                      >{$t("songs.detail.removeMidiEvent")}</button
                    >
                  {/if}
                </div>
                <p class="muted hint-text">
                  {$t("songs.detail.midiEventHint")}
                </p>
                {#if midiEvent}
                  <MidiEventEditor
                    bind:event={midiEvent}
                    onchange={onMidiEventChange}
                    idPrefix="song-midi-event"
                  />
                {/if}
              </div>
            </div>
          {/if}
        </div>
      {:else if activeTab === "samples"}
        {#if Object.keys(songSamples).length === 0}
          <p class="muted">{$t("songs.detail.noSamples")}</p>
        {/if}
        <SamplesSection
          bind:this={songSamplesRef}
          bind:samples={songSamples}
          onchange={onSongSamplesChange}
          onbrowse={onSampleBrowse}
        />
      {:else if activeTab === "sections"}
        {#if song}
          <SectionTimelineEditor
            {song}
            {waveformTracks}
            bind:sections
            bind:dirty={sectionsDirty}
          />
        {/if}
      {:else if activeTab === "lighting"}
        {#if song}
          <SongLightingEditor
            bind:this={lightingEditorRef}
            bind:dirty={lightingDirty}
            {song}
            onreload={load}
            onaddlightfile={setLightingFile}
            onremovelightfile={removeLightingFile}
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

        <!-- Collapsible Notifications section -->
        <div class="collapsible-section">
          <button
            class="collapsible-header"
            onclick={() => (notifSectionOpen = !notifSectionOpen)}
          >
            <span class="collapsible-chevron" class:open={notifSectionOpen}
              >&#9662;</span
            >
            <span class="section-label"
              >{$t("songs.detail.tabs.notifications")}</span
            >
            {#if Object.keys(notificationAudio).length > 0}
              <span class="tab-dot"></span>
            {/if}
          </button>
          {#if notifSectionOpen}
            <div class="collapsible-body">
              <p class="muted hint-text">
                {$t("songs.detail.notificationHint")}
              </p>
              <NotificationsSection
                bind:this={songNotifRef}
                bind:notifications={notificationAudio}
                onchange={onNotificationAudioChange}
                onbrowse={onNotifBrowse}
                onupload={onNotifUpload}
                uploadMsg={notifUploadMsg}
                uploading={notifUploading}
                sectionNames={sections.map((s) => s.name)}
              />
            </div>
          {/if}
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
        initialPath={song?.base_dir}
        onselect={onBrowseSelect}
        oncancel={closeBrowser}
      />
    </div>
  </div>
{/if}

<style>
  .browser-overlay {
    position: fixed;
    inset: 0;
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
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-bottom: 16px;
    font-family: var(--nc-font-display);
    font-weight: 600;
    font-size: 14px;
    color: var(--nc-cyan-600);
    text-decoration: none;
    transition: color var(--nc-dur-fast) var(--nc-ease);
  }
  :global(.nc--dark) .back-link {
    color: var(--nc-cyan-300);
  }
  .back-link::before {
    content: "‹";
    font-size: 18px;
    line-height: 0.7;
  }
  .back-link:hover {
    color: var(--nc-cyan-700);
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 6px;
    flex-wrap: wrap;
  }
  .song-title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 32px;
    line-height: 1.1;
    letter-spacing: -0.02em;
    margin: 0;
    color: var(--nc-fg-1);
  }
  .badges {
    display: flex;
    gap: 4px;
  }
  /* Map legacy badge classes onto Nonchord badge kinds */
  .badge {
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    line-height: 1;
    padding: 5px 8px;
    border-radius: 5px;
    border: 1px solid var(--card-border);
    background: var(--nc-bg-3);
    color: var(--nc-fg-2);
  }
  .badge.midi {
    background: rgba(94, 202, 234, 0.15);
    color: var(--nc-cyan-600);
    border-color: rgba(94, 202, 234, 0.4);
  }
  :global(.nc--dark) .badge.midi {
    color: var(--nc-cyan-300);
  }
  .badge.lighting {
    background: rgba(242, 181, 68, 0.18);
    color: #b47a1a;
    border-color: rgba(242, 181, 68, 0.4);
  }
  :global(.nc--dark) .badge.lighting {
    color: var(--nc-warn);
  }
  .badge.midi-dmx {
    background: rgba(77, 192, 138, 0.15);
    color: #2a8e5e;
    border-color: rgba(77, 192, 138, 0.4);
  }
  :global(.nc--dark) .badge.midi-dmx {
    color: #6bd9a4;
  }
  .badge.loop {
    background: var(--nc-cyan-400);
    color: var(--nc-ink);
    border-color: var(--nc-cyan-500);
  }
  .badge.failed {
    background: rgba(232, 75, 75, 0.12);
    color: var(--nc-error);
    border-color: rgba(232, 75, 75, 0.45);
  }
  .failure-banner {
    background: rgba(232, 75, 75, 0.08);
    border: 1px solid rgba(232, 75, 75, 0.3);
    border-radius: var(--nc-radius-md);
    padding: 12px 16px;
    margin-bottom: 16px;
    font-size: 14px;
    color: var(--nc-fg-1);
  }
  .failure-detail {
    margin-top: 6px;
    font-size: 13px;
    color: var(--nc-error);
    font-family: var(--nc-font-mono);
  }
  .meta {
    display: flex;
    gap: 16px;
    font-family: var(--nc-font-mono);
    font-size: 13px;
    color: var(--nc-fg-3);
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .field label:has(input[type="checkbox"]) {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    cursor: pointer;
    color: var(--nc-fg-1);
  }
  .field-hint {
    display: block;
    font-size: 12px;
    color: var(--nc-fg-3);
    margin-top: 4px;
    margin-left: 24px;
  }
  .beat-grid-summary {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 13px;
    color: var(--nc-fg-3);
    margin-bottom: 16px;
    flex-wrap: wrap;
  }
  .beat-grid-label {
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    font-size: 11px;
    color: var(--nc-fg-3);
  }
  .beat-grid-stat {
    font-family: var(--nc-font-mono);
  }
  .tab-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    border-bottom: 1px solid var(--card-border);
    overflow-x: auto;
    margin-bottom: 24px;
    position: sticky;
    top: 56px;
    z-index: 10;
    background: var(--nc-bg-1);
    padding-top: 4px;
    -webkit-overflow-scrolling: touch;
  }
  .tab-bar::-webkit-scrollbar {
    display: none;
  }
  /* Phone-only fade on the right edge so the user knows there's more to scroll. */
  @media (max-width: 720px) {
    .tab-bar {
      mask-image: linear-gradient(
        to right,
        black 0,
        black calc(100% - 28px),
        transparent 100%
      );
      -webkit-mask-image: linear-gradient(
        to right,
        black 0,
        black calc(100% - 28px),
        transparent 100%
      );
    }
  }
  .tab {
    position: relative;
    padding: 12px 18px 14px;
    font-family: var(--nc-font-display);
    font-weight: 600;
    font-size: 14px;
    line-height: 1;
    color: var(--nc-fg-2);
    background: transparent;
    border: none;
    cursor: pointer;
    white-space: nowrap;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    transition: color var(--nc-dur-fast) var(--nc-ease);
  }
  .tab:hover {
    color: var(--nc-fg-1);
  }
  .tab.active {
    color: var(--nc-fg-1);
  }
  .tab.active::after {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: -1px;
    height: 2px;
    background: var(--nc-cyan-400);
    border-radius: 1px;
  }
  .tab-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--nc-pink-400);
  }
  .tab-dot.config-dot {
    background: var(--nc-cyan-500);
  }
  .tab-save {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 4px;
  }
  .unsaved {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-cyan-600);
  }
  :global(.nc--dark) .unsaved {
    color: var(--nc-cyan-300);
  }
  .save-msg {
    font-size: 12px;
    color: var(--nc-success);
  }
  .save-msg.error {
    color: var(--nc-error);
  }
  .tab-content {
    min-height: 200px;
  }
  .upload-area {
    margin-top: 8px;
  }
  .msg {
    font-size: 13px;
    color: var(--nc-success);
    margin-top: 8px;
  }
  .msg.error {
    color: var(--nc-error);
  }
  .muted {
    color: var(--nc-fg-2);
    font-size: 14px;
    margin-bottom: 12px;
  }
  .midi-event-section {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--card-border);
  }
  .section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }
  .section-label {
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--nc-fg-3);
  }
  .hint-text {
    font-size: 13px;
    margin-bottom: 12px;
  }
  .channel-presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 12px;
  }
  .channel-preset {
    font-family: var(--nc-font-display);
    font-weight: 600;
    font-size: 12px;
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--nc-border-1);
    background: var(--nc-bg-2);
    color: var(--nc-fg-2);
    cursor: pointer;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .channel-preset:hover:not(:disabled) {
    background: var(--nc-bg-3);
    color: var(--nc-fg-1);
  }
  .channel-preset:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .channel-grid {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 8px;
  }
  .channel-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 8px 6px;
    font-family: var(--nc-font-mono);
    font-size: 12px;
    font-weight: 600;
    border: 1px solid var(--card-border);
    border-radius: 8px;
    cursor: pointer;
    color: var(--nc-fg-2);
    background: rgba(94, 202, 234, 0.08);
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .channel-toggle:hover {
    background: rgba(94, 202, 234, 0.16);
  }
  .channel-toggle.excluded {
    background: var(--nc-bg-2);
    border-color: var(--card-border);
    color: var(--nc-fg-4);
    text-decoration: line-through;
  }
  .channel-toggle input {
    display: none;
  }
  @media (max-width: 600px) {
    .channel-grid {
      grid-template-columns: repeat(8, 1fr);
      gap: 6px;
    }
    .channel-toggle {
      height: 44px;
    }
  }
  .feature-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 0;
    margin-bottom: 8px;
    font-size: 14px;
  }
  .feature-label {
    color: var(--nc-fg-3);
    font-weight: 600;
  }
  .feature-value {
    font-family: var(--nc-font-mono);
    color: var(--nc-fg-1);
  }
  .config-section {
    display: flex;
    flex-direction: column;
  }
  .config-editor {
    width: 100%;
    min-height: 400px;
    padding: 16px;
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    background: var(--inset-bg);
    color: var(--nc-fg-1);
    font-family: var(--nc-font-mono);
    font-size: 13px;
    line-height: 1.6;
    resize: vertical;
    outline: none;
    transition:
      border-color var(--nc-dur-fast) var(--nc-ease),
      box-shadow var(--nc-dur-fast) var(--nc-ease);
  }
  .config-editor:focus {
    border-color: var(--nc-cyan-400);
    box-shadow: var(--nc-glow-cyan);
  }
  .status {
    text-align: center;
    padding: 48px 16px;
    color: var(--nc-fg-2);
  }
  .status.error {
    color: var(--nc-error);
  }
  .collapsible-section {
    margin-top: 20px;
    border-top: 1px solid var(--card-border);
  }
  .collapsible-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 14px 0;
    background: none;
    border: none;
    cursor: pointer;
    font: inherit;
    color: var(--nc-fg-3);
  }
  .collapsible-header:hover {
    color: var(--nc-fg-1);
  }
  .collapsible-chevron {
    font-size: 11px;
    transition: transform 0.15s var(--nc-ease);
    transform: rotate(-90deg);
  }
  .collapsible-chevron.open {
    transform: rotate(0deg);
  }
  .collapsible-body {
    padding-bottom: 8px;
  }
  @media (max-width: 720px) {
    .song-title {
      font-size: 26px;
    }
    .tab {
      padding: 10px 14px 12px;
      font-size: 13px;
    }
    .tab-save {
      margin-left: 0;
    }
  }
</style>
