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
  import type { AudioDeviceInfo, MidiDeviceInfo } from "../../lib/api/config";
  import AudioSection from "./AudioSection.svelte";
  import MidiSection from "./MidiSection.svelte";
  import DmxSection from "./DmxSection.svelte";
  import TriggerSection from "./TriggerSection.svelte";
  import ControllersSection from "./ControllersSection.svelte";

  interface Props {
    profile: any;
    audioDevices: AudioDeviceInfo[];
    midiDevices: MidiDeviceInfo[];
    trackNames: string[];
    onrefreshDevices: () => void;
    onchange: () => void;
  }

  let {
    profile = $bindable(),
    audioDevices,
    midiDevices,
    trackNames,
    onrefreshDevices,
    onchange,
  }: Props = $props();

  let collapsed: Record<string, boolean> = $state({});

  function toggleCollapse(section: string) {
    collapsed[section] = !collapsed[section];
  }

  function toggleSection(section: string, enabled: boolean) {
    if (enabled) {
      if (section === "audio") profile.audio = { device: "" };
      else if (section === "midi") profile.midi = { device: "" };
      else if (section === "dmx") profile.dmx = { universes: [] };
      else if (section === "trigger") profile.trigger = { inputs: [] };
      else if (section === "controllers") profile.controllers = [];
    } else {
      if (section === "audio") delete profile.audio;
      else if (section === "midi") delete profile.midi;
      else if (section === "dmx") delete profile.dmx;
      else if (section === "trigger") delete profile.trigger;
      else if (section === "controllers") delete profile.controllers;
    }
    onchange();
  }

  let hasAudio = $derived(profile.audio != null);
  let hasMidi = $derived(profile.midi != null);
  let hasDmx = $derived(profile.dmx != null);
  let hasTrigger = $derived(profile.trigger != null);
  let hasControllers = $derived(profile.controllers != null);
</script>

<div class="editor">
  <div class="field">
    <label for="profile-hostname">Hostname</label>
    <input
      id="profile-hostname"
      class="input"
      type="text"
      placeholder="Leave empty for default profile"
      value={profile.hostname ?? ""}
      onchange={(e) => {
        const v = (e.target as HTMLInputElement).value.trim();
        if (v) {
          profile.hostname = v;
        } else {
          delete profile.hostname;
        }
        onchange();
      }}
    />
    <span class="field-hint"
      >Matches against system hostname. Empty = matches any host.</span
    >
  </div>

  <!-- Audio Section -->
  <div class="section-card">
    <button class="section-header" onclick={() => toggleCollapse("audio")}>
      <span class="section-title">Audio</span>
      <div class="section-controls">
        <span class="toggle-wrap">
          <input
            id="toggle-audio"
            type="checkbox"
            checked={hasAudio}
            onclick={(e: MouseEvent) => e.stopPropagation()}
            onchange={(e) =>
              toggleSection("audio", (e.target as HTMLInputElement).checked)}
          />
          <label for="toggle-audio">Enable</label>
        </span>
        <span class="collapse-icon">{collapsed.audio ? "+" : "-"}</span>
      </div>
    </button>
    {#if hasAudio && !collapsed.audio}
      <div class="section-body">
        <AudioSection
          bind:audio={profile.audio}
          devices={audioDevices}
          {trackNames}
          onrefresh={onrefreshDevices}
          {onchange}
        />
      </div>
    {/if}
  </div>

  <!-- MIDI Section -->
  <div class="section-card">
    <button class="section-header" onclick={() => toggleCollapse("midi")}>
      <span class="section-title">MIDI</span>
      <div class="section-controls">
        <span class="toggle-wrap">
          <input
            id="toggle-midi"
            type="checkbox"
            checked={hasMidi}
            onclick={(e: MouseEvent) => e.stopPropagation()}
            onchange={(e) =>
              toggleSection("midi", (e.target as HTMLInputElement).checked)}
          />
          <label for="toggle-midi">Enable</label>
        </span>
        <span class="collapse-icon">{collapsed.midi ? "+" : "-"}</span>
      </div>
    </button>
    {#if hasMidi && !collapsed.midi}
      <div class="section-body">
        <MidiSection
          bind:midi={profile.midi}
          devices={midiDevices}
          onrefresh={onrefreshDevices}
          {onchange}
        />
      </div>
    {/if}
  </div>

  <!-- DMX Section -->
  <div class="section-card">
    <button class="section-header" onclick={() => toggleCollapse("dmx")}>
      <span class="section-title">DMX</span>
      <div class="section-controls">
        <span class="toggle-wrap">
          <input
            id="toggle-dmx"
            type="checkbox"
            checked={hasDmx}
            onclick={(e: MouseEvent) => e.stopPropagation()}
            onchange={(e) =>
              toggleSection("dmx", (e.target as HTMLInputElement).checked)}
          />
          <label for="toggle-dmx">Enable</label>
        </span>
        <span class="collapse-icon">{collapsed.dmx ? "+" : "-"}</span>
      </div>
    </button>
    {#if hasDmx && !collapsed.dmx}
      <div class="section-body">
        <DmxSection bind:dmx={profile.dmx} {onchange} />
      </div>
    {/if}
  </div>

  <!-- Trigger Section -->
  <div class="section-card">
    <button class="section-header" onclick={() => toggleCollapse("trigger")}>
      <span class="section-title">Triggers</span>
      <div class="section-controls">
        <span class="toggle-wrap">
          <input
            id="toggle-trigger"
            type="checkbox"
            checked={hasTrigger}
            onclick={(e: MouseEvent) => e.stopPropagation()}
            onchange={(e) =>
              toggleSection("trigger", (e.target as HTMLInputElement).checked)}
          />
          <label for="toggle-trigger">Enable</label>
        </span>
        <span class="collapse-icon">{collapsed.trigger ? "+" : "-"}</span>
      </div>
    </button>
    {#if hasTrigger && !collapsed.trigger}
      <div class="section-body">
        <TriggerSection
          bind:trigger={profile.trigger}
          {audioDevices}
          onrefresh={onrefreshDevices}
          {onchange}
        />
      </div>
    {/if}
  </div>

  <!-- Controllers Section -->
  <div class="section-card">
    <button
      class="section-header"
      onclick={() => toggleCollapse("controllers")}
    >
      <span class="section-title">Controllers</span>
      <div class="section-controls">
        <span class="toggle-wrap">
          <input
            id="toggle-controllers"
            type="checkbox"
            checked={hasControllers}
            onclick={(e: MouseEvent) => e.stopPropagation()}
            onchange={(e) =>
              toggleSection(
                "controllers",
                (e.target as HTMLInputElement).checked,
              )}
          />
          <label for="toggle-controllers">Enable</label>
        </span>
        <span class="collapse-icon">{collapsed.controllers ? "+" : "-"}</span>
      </div>
    </button>
    {#if hasControllers && !collapsed.controllers}
      <div class="section-body">
        <ControllersSection bind:controllers={profile.controllers} {onchange} />
      </div>
    {/if}
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field-hint {
    font-size: 11px;
    color: var(--text-dim);
  }
  .section-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .section-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    cursor: pointer;
    background: none;
    border: none;
    width: 100%;
    text-align: left;
    font-family: var(--sans);
    transition: background 0.15s;
  }
  .section-header:hover {
    background: var(--bg-card-hover);
  }
  .section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .section-controls {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .toggle-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .toggle-wrap label {
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .collapse-icon {
    font-family: var(--mono);
    font-size: 14px;
    color: var(--text-dim);
    width: 16px;
    text-align: center;
  }
  .section-body {
    padding: 0 16px 16px;
    border-top: 1px solid var(--border);
    padding-top: 12px;
  }
</style>
