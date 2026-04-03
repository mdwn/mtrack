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
  import { get } from "svelte/store";
  import { showConfirm } from "../../lib/dialog.svelte";
  import type { AudioDeviceInfo, MidiDeviceInfo } from "../../lib/api/config";
  import Tooltip from "./Tooltip.svelte";
  import AudioSection from "./AudioSection.svelte";
  import MidiSection from "./MidiSection.svelte";
  import DmxSection from "./DmxSection.svelte";
  import TriggerSection from "./TriggerSection.svelte";
  import ControllersSection from "./ControllersSection.svelte";
  import LightingSection from "./LightingSection.svelte";
  import NotificationsSection from "./NotificationsSection.svelte";
  import type { NotifBrowseTarget } from "./NotificationsSection.svelte";
  import StatusEventsSection from "./StatusEventsSection.svelte";

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
    onnotifbrowse?: (target: NotifBrowseTarget) => void;
    onnotifupload?: (files: File[]) => void;
    notifUploadMsg?: string;
    notifUploading?: boolean;
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
    onnotifbrowse,
    onnotifupload,
    notifUploadMsg = "",
    notifUploading = false,
  }: Props = $props();

  let notificationsRef: NotificationsSection | undefined = $state();

  const tabs = [
    { key: "audio", labelKey: "profile.tabs.audio" },
    { key: "midi", labelKey: "profile.tabs.midi" },
    { key: "lighting", labelKey: "profile.tabs.lighting" },
    { key: "trigger", labelKey: "profile.tabs.trigger" },
    { key: "controllers", labelKey: "profile.tabs.controllers" },
    { key: "notifications", labelKey: "profile.tabs.notifications" },
    { key: "status_events", labelKey: "profile.tabs.statusEvents" },
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
      else if (section === "notifications") profile.notifications = {};
      else if (section === "status_events")
        profile.status_events = {
          off_events: [],
          idling_events: [],
          playing_events: [],
        };
    } else {
      if (section === "audio") delete profile.audio;
      else if (section === "midi") delete profile.midi;
      else if (section === "lighting") delete profile.dmx;
      else if (section === "trigger") delete profile.trigger;
      else if (section === "controllers") delete profile.controllers;
      else if (section === "notifications") delete profile.notifications;
      else if (section === "status_events") delete profile.status_events;
    }
    onchange();
  }

  export function applyNotifBrowseResult(
    target: NotifBrowseTarget,
    path: string,
  ) {
    notificationsRef?.applyBrowseResult(target, path);
  }

  function removeSectionDetail(): string {
    const label = get(t)(
      tabs.find((tab) => tab.key === activeTab)?.labelKey ?? "",
    );
    const base = get(t)("profile.confirmRemoveSection", { values: { section: label } });
    if (activeTab === "audio" && profile.audio) {
      const count = Object.keys(profile.audio.track_mappings || {}).length;
      if (count > 0) {
        return `${base} ${get(t)("profile.removeAudioDetail", { values: { count } })}`;
      }
    } else if (activeTab === "lighting" && profile.dmx) {
      const count = (profile.dmx.universes || []).length;
      if (count > 0) {
        return `${base} ${get(t)("profile.removeLightingDetail", { values: { count } })}`;
      }
    } else if (activeTab === "controllers" && profile.controllers) {
      const count = profile.controllers.length;
      if (count > 0) {
        return `${base} ${get(t)("profile.removeControllersDetail", { values: { count } })}`;
      }
    } else if (activeTab === "trigger" && profile.trigger) {
      const count = (profile.trigger.inputs || []).length;
      if (count > 0) {
        return `${base} ${get(t)("profile.removeTriggerDetail", { values: { count } })}`;
      }
    }
    return base;
  }

  async function handleRemoveSection() {
    if (await showConfirm(removeSectionDetail(), { danger: true })) {
      toggleSection(activeTab, false);
    }
  }
</script>

<div class="editor">
  <div class="field">
    <label for="profile-hostname"
      >{$t("profile.hostname")}<Tooltip
        text={$t("tooltips.profile.hostname")}
      /></label
    >
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
    {#if !isEnabled(activeTab)}
      <div class="panel-enable">
        <p class="panel-enable-text">
          {$t("profile.enableHint", {
            values: {
              section: $t(
                tabs.find((tab) => tab.key === activeTab)?.labelKey ?? "",
              ),
            },
          })}
        </p>
        <button
          class="btn btn-primary"
          onclick={() => {
            toggleSection(activeTab, true);
            onchange();
          }}
        >
          {$t("profile.enable", {
            values: {
              section: $t(
                tabs.find((tab) => tab.key === activeTab)?.labelKey ?? "",
              ),
            },
          })}
        </button>
      </div>
    {:else if activeTab === "audio" && profile.audio}
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
    {:else if activeTab === "notifications" && profile.notifications}
      <div class="panel-body">
        <NotificationsSection
          bind:this={notificationsRef}
          bind:notifications={profile.notifications}
          {onchange}
          onbrowse={onnotifbrowse}
          onupload={onnotifupload}
          uploadMsg={notifUploadMsg}
          uploading={notifUploading}
        />
      </div>
    {:else if activeTab === "status_events" && profile.status_events}
      <div class="panel-body">
        <StatusEventsSection
          bind:statusEvents={profile.status_events}
          {onchange}
        />
      </div>
    {/if}

    {#if isEnabled(activeTab)}
      <div class="panel-footer">
        <button class="btn btn-danger btn-sm" onclick={handleRemoveSection}>
          {$t("profile.removeSection", {
            values: {
              section: $t(
                tabs.find((tab) => tab.key === activeTab)?.labelKey ?? "",
              ),
            },
          })}
        </button>
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
  .panel-enable {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 40px 20px;
    text-align: center;
  }
  .panel-enable-text {
    font-size: 14px;
    color: var(--text-dim);
  }
  .panel-body {
    padding: 16px;
  }
  .lighting-subsection {
    margin-top: 20px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
  }
  .panel-footer {
    display: flex;
    justify-content: flex-end;
    padding: 12px 16px;
    border-top: 1px solid var(--border);
  }
  @media (max-width: 600px) {
    .tab {
      padding: 8px 12px;
      font-size: 13px;
    }
  }
</style>
