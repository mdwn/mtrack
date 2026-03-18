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
  import { SvelteSet } from "svelte/reactivity";
  import YAML from "yaml";
  import {
    fetchConfigStore,
    fetchAudioDevices,
    fetchMidiDevices,
    addProfile,
    updateProfile,
    deleteProfile,
    updateSamples,
    fetchProfileFiles,
    fetchProfileFile,
    saveProfileFile,
    deleteProfileFile,
    type AudioDeviceInfo,
    type MidiDeviceInfo,
    type ProfileFileInfo,
  } from "../lib/api/config";
  import { fetchSongs } from "../lib/api/songs";
  import ProfileCard from "../components/config/ProfileCard.svelte";
  import ProfileEditor from "../components/config/ProfileEditor.svelte";
  import SamplesSection, {
    type SampleBrowseTarget,
  } from "../components/config/SamplesSection.svelte";
  import FileBrowser from "../components/songs/FileBrowser.svelte";

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();

  // Parse: #/config, #/config/ProfileName, #/config/ProfileName/section
  let routeProfile = $derived.by(() => {
    const prefix = "#/config/";
    if (!currentHash.startsWith(prefix) || currentHash.length <= prefix.length)
      return null;
    const rest = decodeURIComponent(currentHash.slice(prefix.length));
    const slashIdx = rest.indexOf("/");
    return slashIdx >= 0 ? rest.slice(0, slashIdx) : rest;
  });

  let routeSection = $derived.by(() => {
    const prefix = "#/config/";
    if (!currentHash.startsWith(prefix)) return undefined;
    const rest = decodeURIComponent(currentHash.slice(prefix.length));
    const slashIdx = rest.indexOf("/");
    if (slashIdx < 0) return undefined;
    return rest.slice(slashIdx + 1) as string;
  });

  let configYaml = $state("");
  let checksum = $state("");
  let profiles = $state<any[]>([]);
  let selectedIndex = $state<number | null>(null);
  let isNew = $state(false);
  let loading = $state(true);
  let error = $state("");
  let saving = $state(false);
  let saveMsg = $state("");
  let dirty = $state(false);
  let audioDevices = $state<AudioDeviceInfo[]>([]);
  let midiDevices = $state<MidiDeviceInfo[]>([]);
  let trackNames = $state<string[]>([]);

  // Snapshot of the profile at load time for dirty tracking
  let savedSnapshot = $state("");

  // File-based profiles mode
  let profilesDir = $state<string | null>(null);
  let profileFiles = $state<ProfileFileInfo[]>([]);
  let selectedFilename = $state<string | null>(null);

  // Samples file awareness
  let samplesFile = $state<string | null>(null);

  async function loadConfig() {
    try {
      loading = true;
      error = "";
      const snapshot = await fetchConfigStore();
      configYaml = snapshot.yaml;
      checksum = snapshot.checksum;
      parseProfiles();
      if (profilesDir) {
        await loadProfileFiles();
      }
    } catch (e: any) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  let sampleNames = $state<string[]>([]);
  let samplesMap = $state<Record<string, any>>({});
  let samplesDirty = $state(false);
  let samplesSaving = $state(false);
  let samplesSaveMsg = $state("");
  let samplesSnapshot = $state("");

  function parseProfiles() {
    try {
      const parsed = YAML.parse(configYaml);
      profilesDir = parsed?.profiles_dir || null;
      samplesFile = parsed?.samples_file || null;
      if (!profilesDir) {
        profiles = parsed?.profiles || [];
      }
      // Extract sample definitions from the config's samples map
      const samples = parsed?.samples;
      if (samples && typeof samples === "object") {
        samplesMap = samples;
        sampleNames = Object.keys(samples).sort();
      } else {
        samplesMap = {};
        sampleNames = [];
      }
      samplesSnapshot = JSON.stringify(samplesMap);
      samplesDirty = false;
    } catch {
      profiles = [];
      samplesMap = {};
      sampleNames = [];
    }
  }

  async function loadProfileFiles() {
    try {
      profileFiles = await fetchProfileFiles();
    } catch (e: any) {
      console.error("Failed to load profile files:", e);
      profileFiles = [];
    }
  }

  async function loadDevices() {
    const [audioResult, midiResult] = await Promise.allSettled([
      fetchAudioDevices(),
      fetchMidiDevices(),
    ]);
    if (audioResult.status === "fulfilled") {
      audioDevices = audioResult.value;
    } else {
      console.error("Failed to load audio devices:", audioResult.reason);
    }
    if (midiResult.status === "fulfilled") {
      midiDevices = midiResult.value;
    } else {
      console.error("Failed to load MIDI devices:", midiResult.reason);
    }
  }

  async function loadTrackNames() {
    try {
      const result = await fetchSongs();
      const names = new SvelteSet<string>();
      for (const song of result.songs) {
        for (const track of song.tracks) {
          names.add(track);
        }
      }
      trackNames = [...names].sort();
    } catch (e: any) {
      console.error("Failed to load track names:", e);
    }
  }

  // --- File-based profile operations ---

  async function selectFileProfile(filename: string) {
    saving = false;
    saveMsg = "";
    dirty = false;
    isNew = false;
    try {
      const data = await fetchProfileFile(filename);
      selectedFilename = filename;
      selectedIndex = 0;
      profiles = [data.profile as any];
      savedSnapshot = JSON.stringify(data.profile);
      updateConfigUrl(filename.replace(/\.\w+$/, ""));
    } catch (e: any) {
      error = e.message;
    }
  }

  function addNewFileProfile() {
    const name = prompt("Profile filename (without extension):");
    if (!name) return;
    const empty: any = {};
    profiles = [empty];
    selectedIndex = 0;
    selectedFilename = name;
    isNew = true;
    savedSnapshot = JSON.stringify(empty);
    dirty = false;
    saveMsg = "";
  }

  async function saveFileProfile() {
    if (selectedIndex === null || !selectedFilename) return;
    saving = true;
    saveMsg = "";
    try {
      await saveProfileFile(selectedFilename, profiles[selectedIndex]);
      savedSnapshot = JSON.stringify(profiles[selectedIndex]);
      isNew = false;
      dirty = false;
      saveMsg = "Saved";
      setTimeout(() => (saveMsg = ""), 2000);
      await loadProfileFiles();
    } catch (e: any) {
      saveMsg = e.message;
    } finally {
      saving = false;
    }
  }

  async function removeFileProfile() {
    if (!selectedFilename) return;
    if (!confirm("Delete this profile?")) return;
    saving = true;
    saveMsg = "";
    try {
      await deleteProfileFile(selectedFilename);
      selectedIndex = null;
      selectedFilename = null;
      isNew = false;
      dirty = false;
      await loadProfileFiles();
    } catch (e: any) {
      saveMsg = e.message;
    } finally {
      saving = false;
    }
  }

  function goBackFile() {
    if (dirty && !confirm("Discard unsaved changes?")) return;
    selectedIndex = null;
    selectedFilename = null;
    isNew = false;
    dirty = false;
    updateConfigUrl();
    saveMsg = "";
  }

  // --- Inline profile operations ---

  function selectProfile(index: number) {
    selectedIndex = index;
    isNew = false;
    savedSnapshot = JSON.stringify(profiles[index]);
    dirty = false;
    saveMsg = "";
    const name = profiles[index]?.hostname || `Profile #${index}`;
    updateConfigUrl(name);
  }

  function addNewProfile() {
    const empty: any = {};
    profiles.push(empty);
    selectedIndex = profiles.length - 1;
    isNew = true;
    savedSnapshot = JSON.stringify(empty);
    dirty = false;
    saveMsg = "";
  }

  function goBack() {
    if (dirty && !confirm("Discard unsaved changes?")) return;
    selectedIndex = null;
    isNew = false;
    dirty = false;
    saveMsg = "";
    updateConfigUrl();
  }

  function onProfileChange() {
    if (selectedIndex !== null) {
      dirty = JSON.stringify(profiles[selectedIndex]) !== savedSnapshot;
    }
  }

  function applySnapshot(snapshot: { yaml: string; checksum: string }) {
    configYaml = snapshot.yaml;
    checksum = snapshot.checksum;
    parseProfiles();
  }

  async function saveProfile() {
    if (selectedIndex === null) return;
    saving = true;
    saveMsg = "";
    try {
      const profile = profiles[selectedIndex];
      let snapshot;
      if (isNew) {
        snapshot = await addProfile(profile, checksum);
      } else {
        snapshot = await updateProfile(selectedIndex, profile, checksum);
      }
      applySnapshot(snapshot);
      isNew = false;
      if (selectedIndex < profiles.length) {
        savedSnapshot = JSON.stringify(profiles[selectedIndex]);
      }
      dirty = false;
      saveMsg = "Saved";
      setTimeout(() => (saveMsg = ""), 2000);
    } catch (e: any) {
      saveMsg = e.message;
    } finally {
      saving = false;
    }
  }

  async function removeProfile() {
    if (selectedIndex === null) return;
    if (!confirm("Delete this profile?")) return;
    saving = true;
    saveMsg = "";
    try {
      const snapshot = await deleteProfile(selectedIndex, checksum);
      applySnapshot(snapshot);
      selectedIndex = null;
      isNew = false;
      dirty = false;
    } catch (e: any) {
      saveMsg = e.message;
    } finally {
      saving = false;
    }
  }

  function onSamplesChange() {
    samplesDirty = JSON.stringify(samplesMap) !== samplesSnapshot;
    // Update sampleNames for trigger dropdowns
    sampleNames = Object.keys(samplesMap).sort();
  }

  async function saveSamples() {
    samplesSaving = true;
    samplesSaveMsg = "";
    try {
      const snapshot = await updateSamples(samplesMap, checksum);
      applySnapshot(snapshot);
      samplesSnapshot = JSON.stringify(samplesMap);
      samplesDirty = false;
      samplesSaveMsg = "Saved";
      setTimeout(() => (samplesSaveMsg = ""), 2000);
    } catch (e: any) {
      samplesSaveMsg = e.message;
    } finally {
      samplesSaving = false;
    }
  }

  // Sample file browser state
  let sampleBrowseTarget = $state<SampleBrowseTarget | null>(null);
  let samplesRef: SamplesSection | undefined = $state();

  function onSampleBrowse(target: SampleBrowseTarget) {
    sampleBrowseTarget = target;
  }

  function onSampleBrowseSelect(paths: string[]) {
    if (paths.length > 0 && sampleBrowseTarget && samplesRef) {
      samplesRef.applyBrowseResult(sampleBrowseTarget, paths[0]);
    }
    sampleBrowseTarget = null;
  }

  function closeSampleBrowser() {
    sampleBrowseTarget = null;
  }

  function updateConfigUrl(profileName?: string | null, section?: string) {
    if (profileName) {
      const base = `#/config/${encodeURIComponent(profileName)}`;
      window.location.hash = section ? `${base}/${section}` : base;
    } else {
      window.location.hash = "#/config";
    }
  }

  $effect(() => {
    loadConfig();
    loadDevices();
    loadTrackNames();
  });

  // Auto-select profile from URL after data loads
  $effect(() => {
    if (loading || !routeProfile) return;
    if (profilesDir) {
      // File-based: match by filename (without extension)
      const match = profileFiles.find(
        (f) => f.filename.replace(/\.\w+$/, "") === routeProfile,
      );
      if (match && selectedFilename !== match.filename) {
        selectFileProfile(match.filename);
      }
    } else {
      // Inline: match by hostname or index
      const idx = profiles.findIndex(
        (p: any) =>
          (p.hostname || `Profile #${profiles.indexOf(p)}`) === routeProfile,
      );
      if (idx >= 0 && selectedIndex !== idx) {
        selectProfile(idx);
      }
    }
  });
</script>

{#if loading}
  <div class="page-placeholder">
    <p>Loading configuration...</p>
  </div>
{:else if error}
  <div class="page-placeholder">
    <h2>Error</h2>
    <p>{error}</p>
    <div class="error-actions">
      <button class="btn" onclick={loadConfig}>Retry</button>
      <button class="btn" onclick={() => (error = "")}>Dismiss</button>
    </div>
  </div>
{:else if profilesDir}
  <!-- File-based profiles mode -->
  {#if selectedIndex !== null && profiles[selectedIndex]}
    <!-- Detail View (file-based) -->
    <div class="detail-view">
      <div class="detail-toolbar">
        <button class="btn" onclick={goBackFile}>Back</button>
        <span class="detail-title">
          {isNew ? "New Profile" : selectedFilename || "Profile"}
        </span>
        <div class="toolbar-actions">
          {#if saveMsg}
            <span class="save-msg" class:save-error={saveMsg !== "Saved"}
              >{saveMsg}</span
            >
          {/if}
          {#if !isNew}
            <button
              class="btn btn-danger"
              onclick={removeFileProfile}
              disabled={saving}>Delete</button
            >
          {/if}
          <button
            class="btn btn-primary"
            onclick={saveFileProfile}
            disabled={saving || !dirty}
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>

      <ProfileEditor
        bind:profile={profiles[selectedIndex]}
        {audioDevices}
        {midiDevices}
        {trackNames}
        {sampleNames}
        initialSection={routeSection}
        onrefreshDevices={loadDevices}
        onchange={onProfileChange}
        onsectionchange={(section) =>
          updateConfigUrl(selectedFilename?.replace(/\.\w+$/, ""), section)}
      />
    </div>
  {:else}
    <!-- List View (file-based) -->
    <div class="list-view">
      <div class="list-header">
        <h2>Hardware Profiles</h2>
        <div class="toolbar-actions">
          <button class="btn btn-primary" onclick={addNewFileProfile}
            >Add Profile</button
          >
        </div>
      </div>

      {#if profileFiles.length === 0}
        <div class="empty-state">
          <p>No profiles in directory.</p>
          <p>Add a profile to configure audio, MIDI, DMX, and controllers.</p>
        </div>
      {:else}
        <div class="profile-list">
          {#each profileFiles as pf (pf.filename)}
            <button
              class="profile-file-row"
              onclick={() => selectFileProfile(pf.filename)}
            >
              <span class="pf-name">{pf.filename}</span>
              {#if pf.hostname}
                <span class="pf-hostname">{pf.hostname}</span>
              {/if}
              <div class="pf-badges">
                {#if pf.has_audio}<span class="pf-badge pf-audio">AUDIO</span
                  >{/if}
                {#if pf.has_midi}<span class="pf-badge pf-midi">MIDI</span>{/if}
                {#if pf.has_dmx}<span class="pf-badge pf-dmx">DMX</span>{/if}
                {#if pf.has_trigger}<span class="pf-badge pf-trigger"
                    >TRIGGER</span
                  >{/if}
                {#if pf.has_controllers}<span class="pf-badge pf-ctrl"
                    >CTRL</span
                  >{/if}
              </div>
            </button>
          {/each}
        </div>
      {/if}

      <!-- Samples Section -->
      <div class="samples-top-section">
        <div class="list-header">
          <h2>Samples</h2>
          <div class="toolbar-actions">
            {#if samplesFile}
              <span class="info-badge">from {samplesFile}</span>
            {:else}
              {#if samplesSaveMsg}
                <span
                  class="save-msg"
                  class:save-error={samplesSaveMsg !== "Saved"}
                  >{samplesSaveMsg}</span
                >
              {/if}
              <button
                class="btn btn-primary"
                onclick={saveSamples}
                disabled={samplesSaving || !samplesDirty}
              >
                {samplesSaving ? "Saving..." : "Save Samples"}
              </button>
            {/if}
          </div>
        </div>
        {#if samplesFile}
          <div class="info-banner">
            Samples loaded from <code>{samplesFile}</code>. Edit the file
            directly to make changes.
          </div>
        {/if}
        <SamplesSection
          bind:this={samplesRef}
          bind:samples={samplesMap}
          onchange={onSamplesChange}
          onbrowse={onSampleBrowse}
        />
      </div>
    </div>
  {/if}
{:else if selectedIndex !== null && profiles[selectedIndex]}
  <!-- Detail View (inline) -->
  <div class="detail-view">
    <div class="detail-toolbar">
      <button class="btn" onclick={goBack}>Back</button>
      <span class="detail-title">
        {isNew
          ? "New Profile"
          : profiles[selectedIndex].hostname || `Profile #${selectedIndex}`}
      </span>
      <div class="toolbar-actions">
        {#if saveMsg}
          <span class="save-msg" class:save-error={saveMsg !== "Saved"}
            >{saveMsg}</span
          >
        {/if}
        {#if !isNew}
          <button
            class="btn btn-danger"
            onclick={removeProfile}
            disabled={saving}>Delete</button
          >
        {/if}
        <button
          class="btn btn-primary"
          onclick={saveProfile}
          disabled={saving || !dirty}
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </div>

    <ProfileEditor
      bind:profile={profiles[selectedIndex]}
      {audioDevices}
      {midiDevices}
      {trackNames}
      {sampleNames}
      initialSection={routeSection}
      onrefreshDevices={loadDevices}
      onchange={onProfileChange}
      onsectionchange={(section) => {
        if (selectedIndex === null) return;
        const name =
          profiles[selectedIndex]?.hostname || `Profile #${selectedIndex}`;
        updateConfigUrl(name, section);
      }}
    />
  </div>
{:else}
  <!-- List View (inline) -->
  <div class="list-view">
    <div class="list-header">
      <h2>Hardware Profiles</h2>
      <button class="btn btn-primary" onclick={addNewProfile}
        >Add Profile</button
      >
    </div>

    {#if profiles.length === 0}
      <div class="empty-state">
        <p>No profiles configured.</p>
        <p>Add a profile to configure audio, MIDI, DMX, and controllers.</p>
      </div>
    {:else}
      <div class="profile-list">
        {#each profiles as profile, i (i)}
          <ProfileCard {profile} index={i} onclick={() => selectProfile(i)} />
        {/each}
      </div>
    {/if}

    <!-- Samples Section -->
    <div class="samples-top-section">
      <div class="list-header">
        <h2>Samples</h2>
        <div class="toolbar-actions">
          {#if samplesFile}
            <span class="info-badge">from {samplesFile}</span>
          {:else}
            {#if samplesSaveMsg}
              <span
                class="save-msg"
                class:save-error={samplesSaveMsg !== "Saved"}
                >{samplesSaveMsg}</span
              >
            {/if}
            <button
              class="btn btn-primary"
              onclick={saveSamples}
              disabled={samplesSaving || !samplesDirty}
            >
              {samplesSaving ? "Saving..." : "Save Samples"}
            </button>
          {/if}
        </div>
      </div>
      {#if samplesFile}
        <div class="info-banner">
          Samples loaded from <code>{samplesFile}</code>. Edit the file directly
          to make changes.
        </div>
      {/if}
      <SamplesSection
        bind:this={samplesRef}
        bind:samples={samplesMap}
        onchange={onSamplesChange}
        onbrowse={onSampleBrowse}
      />
    </div>
  </div>
{/if}

{#if sampleBrowseTarget}
  <div class="browser-overlay">
    <div class="browser-modal">
      <FileBrowser
        filter={["audio"]}
        onselect={onSampleBrowseSelect}
        oncancel={closeSampleBrowser}
      />
    </div>
  </div>
{/if}

<style>
  .error-actions {
    display: flex;
    gap: 8px;
    justify-content: center;
    margin-top: 8px;
  }
  .list-view {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .list-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .list-header h2 {
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
  }
  .profile-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .empty-state {
    text-align: center;
    padding: 60px 20px;
    color: var(--text-dim);
  }
  .empty-state p {
    margin-bottom: 4px;
    font-size: 14px;
  }
  .detail-view {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .detail-toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    position: sticky;
    top: 48px;
    z-index: 10;
  }
  .detail-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
    flex: 1;
  }
  .toolbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .save-msg {
    font-size: 13px;
    color: var(--green);
  }
  .save-error {
    color: var(--red);
  }
  .samples-top-section {
    margin-top: 16px;
    padding-top: 24px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .info-badge {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--bg-hover);
    color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .info-banner {
    font-size: 14px;
    padding: 10px 14px;
    border-radius: var(--radius);
    background: var(--bg-hover);
    color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .info-banner code {
    font-weight: 600;
    color: var(--text);
  }
  .profile-file-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    text-align: left;
    width: 100%;
    color: var(--text);
    font: inherit;
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .profile-file-row:hover {
    background: var(--bg-card-hover);
    border-color: var(--text-dim);
  }
  .pf-name {
    font-size: 15px;
    font-weight: 600;
    flex-shrink: 0;
  }
  .pf-hostname {
    font-size: 13px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .pf-badges {
    display: flex;
    gap: 4px;
    margin-left: auto;
  }
  .pf-badge {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.5px;
    padding: 2px 5px;
    border-radius: 3px;
  }
  .pf-audio {
    background: var(--accent);
    color: #fff;
  }
  .pf-midi {
    background: var(--green-dim);
    color: var(--green);
  }
  .pf-dmx {
    background: var(--yellow-dim);
    color: var(--yellow);
  }
  .pf-trigger {
    background: rgba(168, 85, 247, 0.15);
    color: #a855f7;
  }
  .pf-ctrl {
    background: var(--red-dim);
    color: var(--red);
  }
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
  @media (max-width: 600px) {
    .detail-toolbar {
      flex-wrap: wrap;
    }
  }
</style>
