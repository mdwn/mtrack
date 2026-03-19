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
  import { t } from "svelte-i18n";
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
    initialSection?: string;
    onrefreshDevices: () => void;
    onchange: () => void;
    onsectionchange?: (section: string) => void;
  }

  let {
    profile = $bindable(),
    audioDevices,
    midiDevices,
    trackNames,
    sampleNames,
    initialSection,
    onrefreshDevices,
    onchange,
    onsectionchange,
  }: Props = $props();

  const tabs = [
    { key: "audio", labelKey: "profile.tabs.audio" },
    { key: "midi", labelKey: "profile.tabs.midi" },
    { key: "lighting", labelKey: "profile.tabs.lighting" },
    { key: "trigger", labelKey: "profile.tabs.trigger" },
    { key: "controllers", labelKey: "profile.tabs.controllers" },
  ] as const;

  type TabKey = (typeof tabs)[number]["key"];

  const validKeys = tabs.map((tab) => tab.key as string);
  function getInitialTab(): TabKey {
    const s = initialSection;
    return s && validKeys.includes(s) ? (s as TabKey) : "audio";
  }
  let activeTab = $state<TabKey>(getInitialTab());

  function isEnabled(key: string): boolean {
    if (key === "lighting") return profile.dmx != null;
    return profile[key] != null;
  }

  function toggleSection(section: string, enabled: boolean) {
    if (enabled) {
      if (section === "audio") profile.audio = { device: "" };
      else if (section === "midi") profile.midi = { device: "" };
      else if (section === "lighting")
        profile.dmx = { universes: [], lighting: {} };
      else if (section === "trigger") profile.trigger = { inputs: [] };
      else if (section === "controllers") profile.controllers = [];
    } else {
      if (section === "audio") delete profile.audio;
      else if (section === "midi") delete profile.midi;
      else if (section === "lighting") delete profile.dmx;
      else if (section === "trigger") delete profile.trigger;
      else if (section === "controllers") delete profile.controllers;
    }
    onchange();
  }
</script>

<div class="editor">
  <div class="field">
    <label for="profile-hostname">{$t("profile.hostname")}</label>
    <input
      id="profile-hostname"
      class="input"
      type="text"
      placeholder={$t("profile.hostnamePlaceholder")}
      value={profile.hostname ?? ""}
      oninput={(e) => {
        const v = (e.target as HTMLInputElement).value.trim();
        if (v) {
          profile.hostname = v;
        } else {
          delete profile.hostname;
        }
        onchange();
      }}
    />
    <span class="field-hint">{$t("profile.hostnameHint")}</span>
  </div>

  <div class="tab-bar" role="tablist">
    {#each tabs as tab (tab.key)}
      <button
        class="tab"
        class:active={activeTab === tab.key}
        role="tab"
        aria-selected={activeTab === tab.key}
        onclick={() => {
          activeTab = tab.key;
          onsectionchange?.(tab.key);
        }}
      >
        {$t(tab.labelKey)}
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
        {$t("profile.enable", {
          values: {
            section: $t(
              tabs.find((tab) => tab.key === activeTab)?.labelKey ?? "",
            ),
          },
        })}
      </label>
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
    {:else if activeTab === "lighting" && profile.dmx}
      <div class="panel-body">
        <DmxSection bind:dmx={profile.dmx} {onchange} />

        <div class="lighting-subsection">
          <LightingSection bind:lighting={profile.dmx.lighting} {onchange} />
        </div>
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
          {$t("profile.notEnabled", {
            values: {
              section: $t(
                tabs.find((tab) => tab.key === activeTab)?.labelKey ?? "",
              ),
            },
          })}
        </p>
        <p>{$t("profile.toggleHint")}</p>
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
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field-hint {
    font-size: 12px;
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
    font-size: 14px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .panel-body {
    padding: 16px;
  }
  .lighting-subsection {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
  }
  .panel-empty {
    padding: 40px 20px;
    text-align: center;
    color: var(--text-dim);
  }
  .panel-empty p {
    margin-bottom: 4px;
    font-size: 14px;
  }
  @media (max-width: 600px) {
    .tab {
      padding: 8px 12px;
      font-size: 13px;
    }
  }
</style>
