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
  import LightingSection from "./LightingSection.svelte";

  interface Props {
    profile: any;
    audioDevices: AudioDeviceInfo[];
    midiDevices: MidiDeviceInfo[];
    trackNames: string[];
    sampleNames: string[];
    onrefreshDevices: () => void;
    onchange: () => void;
  }

  let {
    profile = $bindable(),
    audioDevices,
    midiDevices,
    trackNames,
    sampleNames,
    onrefreshDevices,
    onchange,
  }: Props = $props();

  const tabs = [
    { key: "audio", label: "Audio" },
    { key: "midi", label: "MIDI" },
    { key: "dmx", label: "DMX" },
    { key: "lighting", label: "Lighting" },
    { key: "trigger", label: "Triggers" },
    { key: "controllers", label: "Controllers" },
  ] as const;

  type TabKey = (typeof tabs)[number]["key"];

  let activeTab = $state<TabKey>("audio");

  function isEnabled(key: string): boolean {
    if (key === "lighting") {
      return profile.dmx?.lighting != null;
    }
    return profile[key] != null;
  }

  function toggleSection(section: string, enabled: boolean) {
    if (section === "lighting") {
      if (enabled) {
        if (!profile.dmx) profile.dmx = { universes: [] };
        profile.dmx.lighting = {};
      } else {
        if (profile.dmx) delete profile.dmx.lighting;
      }
    } else if (enabled) {
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

  <div class="tab-bar" role="tablist">
    {#each tabs as tab (tab.key)}
      <button
        class="tab"
        class:active={activeTab === tab.key}
        role="tab"
        aria-selected={activeTab === tab.key}
        onclick={() => (activeTab = tab.key)}
      >
        {tab.label}
        {#if isEnabled(tab.key)}
          <span class="tab-dot"></span>
        {/if}
      </button>
    {/each}
  </div>

  <div class="tab-panel" role="tabpanel">
    <div class="panel-header">
      <label class="enable-toggle">
        <input
          type="checkbox"
          checked={isEnabled(activeTab)}
          onchange={(e) =>
            toggleSection(activeTab, (e.target as HTMLInputElement).checked)}
        />
        Enable {tabs.find((t) => t.key === activeTab)?.label}
      </label>
      {#if activeTab === "lighting" && !profile.dmx}
        <span class="panel-note">Enabling lighting will also enable DMX.</span>
      {/if}
    </div>

    {#if activeTab === "audio" && profile.audio}
      <div class="panel-body">
        <AudioSection
          bind:audio={profile.audio}
          devices={audioDevices}
          {trackNames}
          onrefresh={onrefreshDevices}
          {onchange}
        />
      </div>
    {:else if activeTab === "midi" && profile.midi}
      <div class="panel-body">
        <MidiSection
          bind:midi={profile.midi}
          devices={midiDevices}
          onrefresh={onrefreshDevices}
          {onchange}
        />
      </div>
    {:else if activeTab === "dmx" && profile.dmx}
      <div class="panel-body">
        <DmxSection bind:dmx={profile.dmx} {onchange} />
      </div>
    {:else if activeTab === "lighting" && profile.dmx?.lighting}
      <div class="panel-body">
        <LightingSection bind:lighting={profile.dmx.lighting} {onchange} />
      </div>
    {:else if activeTab === "trigger" && profile.trigger}
      <div class="panel-body">
        <TriggerSection
          bind:trigger={profile.trigger}
          {audioDevices}
          {sampleNames}
          onrefresh={onrefreshDevices}
          {onchange}
        />
      </div>
    {:else if activeTab === "controllers" && profile.controllers}
      <div class="panel-body">
        <ControllersSection bind:controllers={profile.controllers} {onchange} />
      </div>
    {:else if !isEnabled(activeTab)}
      <div class="panel-empty">
        <p>
          {tabs.find((t) => t.key === activeTab)?.label} is not enabled for this profile.
        </p>
        <p>Toggle the checkbox above to configure it.</p>
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
  .tab-bar {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .tab {
    position: relative;
    padding: 10px 16px;
    font-size: 13px;
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
  .tab-panel {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .panel-header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .enable-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .panel-note {
    font-size: 11px;
    color: var(--text-dim);
  }
  .panel-body {
    padding: 16px;
  }
  .panel-empty {
    padding: 40px 20px;
    text-align: center;
    color: var(--text-dim);
  }
  .panel-empty p {
    margin-bottom: 4px;
    font-size: 13px;
  }
  @media (max-width: 600px) {
    .tab {
      padding: 8px 12px;
      font-size: 12px;
    }
  }
</style>
