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
    type AudioDeviceInfo,
    type MidiDeviceInfo,
  } from "../lib/api/config";
  import { fetchSongs } from "../lib/api/songs";
  import ProfileCard from "../components/config/ProfileCard.svelte";
  import ProfileEditor from "../components/config/ProfileEditor.svelte";

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

  async function loadConfig() {
    try {
      loading = true;
      error = "";
      const snapshot = await fetchConfigStore();
      configYaml = snapshot.yaml;
      checksum = snapshot.checksum;
      parseProfiles();
    } catch (e: any) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  function parseProfiles() {
    try {
      const parsed = YAML.parse(configYaml);
      profiles = parsed?.profiles || [];
    } catch {
      profiles = [];
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
      const songs = await fetchSongs();
      const names = new SvelteSet<string>();
      for (const song of songs) {
        for (const track of song.tracks) {
          names.add(track);
        }
      }
      trackNames = [...names].sort();
    } catch (e: any) {
      console.error("Failed to load track names:", e);
    }
  }

  function selectProfile(index: number) {
    selectedIndex = index;
    isNew = false;
    savedSnapshot = JSON.stringify(profiles[index]);
    dirty = false;
    saveMsg = "";
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

  $effect(() => {
    loadConfig();
    loadDevices();
    loadTrackNames();
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
    <button class="btn" onclick={loadConfig}>Retry</button>
  </div>
{:else if selectedIndex !== null && profiles[selectedIndex]}
  <!-- Detail View -->
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
      onrefreshDevices={loadDevices}
      onchange={onProfileChange}
    />
  </div>
{:else}
  <!-- List View -->
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
      <div class="profile-grid">
        {#each profiles as profile, i (i)}
          <ProfileCard {profile} index={i} onclick={() => selectProfile(i)} />
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
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
  .profile-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
  }
  .empty-state {
    text-align: center;
    padding: 60px 20px;
    color: var(--text-dim);
  }
  .empty-state p {
    margin-bottom: 4px;
    font-size: 13px;
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
    top: 0;
    z-index: 10;
  }
  .detail-title {
    font-size: 15px;
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
    font-size: 12px;
    color: var(--green);
  }
  .save-error {
    color: var(--red);
  }
  @media (max-width: 600px) {
    .profile-grid {
      grid-template-columns: 1fr;
    }
    .detail-toolbar {
      flex-wrap: wrap;
    }
  }
</style>
