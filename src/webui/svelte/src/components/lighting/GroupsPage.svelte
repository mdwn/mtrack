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
  import { registerDirtyGuard } from "../../lib/dirtyGuard";
  import { playbackStore } from "../../lib/ws/stores";
  import {
    fetchRunningProfile,
    loadProfileSet,
    pickRunningProfile,
    readProfile,
    writeProfile,
    parseProfileYaml,
    type ProfileEntry,
    type ProfileSet,
  } from "../../lib/profileStore";
  import ProfileLightingPanel from "./ProfileLightingPanel.svelte";

  interface Props {
    /** The `?profile=` value from the hash, if any. */
    profileParam: string | null;
  }

  let { profileParam }: Props = $props();

  let loading = $state(true);
  let error = $state("");
  let set = $state<ProfileSet | null>(null);
  let running = $state<string | null>(null);
  let selected = $state<ProfileEntry | null>(null);
  let profile = $state<any>(null);
  let dirty = $state(false);
  let saving = $state(false);
  let saveMsg = $state("");
  let saveOk = $state(false);
  let selectEl: HTMLSelectElement | undefined = $state();

  $effect(() => {
    return registerDirtyGuard(() => dirty, get(t)("config.discardUnsaved"));
  });

  $effect(() => {
    if (dirty) {
      const handler = (e: BeforeUnloadEvent) => {
        e.preventDefault();
      };
      window.addEventListener("beforeunload", handler);
      return () => window.removeEventListener("beforeunload", handler);
    }
  });

  function hashFor(name: string): string {
    return `#/lighting/groups?profile=${encodeURIComponent(name)}`;
  }

  /** The profile the URL asks for, else the one the player is running. */
  function target(): ProfileEntry | null {
    if (!set) return null;
    return (
      (profileParam
        ? set.entries.find((e) => e.name === profileParam)
        : undefined) ?? pickRunningProfile(set.entries, running)
    );
  }

  async function open(entry: ProfileEntry | null) {
    selected = entry;
    profile = null;
    dirty = false;
    saveMsg = "";
    saveOk = false;
    if (!entry || !set) return;
    try {
      const p = await readProfile(entry.ref, set.inline);
      // A profile can carry `dmx` without a `lighting` block yet.
      if (p?.dmx && !p.dmx.lighting) p.dmx.lighting = {};
      profile = p;
    } catch (e: any) {
      error = e.message;
    }
  }

  async function load() {
    try {
      loading = true;
      error = "";
      const [s, r] = await Promise.all([
        loadProfileSet(),
        fetchRunningProfile(),
      ]);
      set = s;
      running = r;
      await open(target());
    } catch (e: any) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    load();
  });

  // Follow the URL: the picker and Back both change `profileParam`. Unsaved
  // edits are confirmed away first, and a refusal puts the URL back.
  $effect(() => {
    void profileParam;
    const want = target();
    if (loading || !want || want.name === selected?.name) return;
    void (async () => {
      if (
        dirty &&
        !(await showConfirm(get(t)("config.discardUnsaved"), { danger: true }))
      ) {
        if (selectEl && selected) selectEl.value = selected.name;
        if (selected) window.location.hash = hashFor(selected.name);
        return;
      }
      await open(want);
    })();
  });

  function onPick(e: Event) {
    window.location.hash = hashFor((e.target as HTMLSelectElement).value);
  }

  function onchange() {
    dirty = true;
  }

  async function save() {
    if (!selected || !set || !profile) return;
    if ($playbackStore.locked) {
      saveMsg = get(t)("common.locked");
      return;
    }
    saving = true;
    saveMsg = "";
    saveOk = false;
    try {
      const snapshot = await writeProfile(selected.ref, profile, set.checksum);
      if (snapshot) {
        set.yaml = snapshot.yaml;
        set.checksum = snapshot.checksum;
        set.inline = parseProfileYaml(snapshot.yaml).inline;
      }
      dirty = false;
      saveOk = true;
      setTimeout(() => (saveOk = false), 2000);
    } catch (e: any) {
      saveMsg = e.message;
    } finally {
      saving = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === "s") {
      e.preventDefault();
      if (dirty && !$playbackStore.locked && !saving) save();
    }
  }

  $effect(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });
</script>

{#if loading}
  <div class="groups-placeholder">
    <p><span class="spinner"></span> {$t("config.loadingConfig")}</p>
  </div>
{:else if error}
  <div class="groups-placeholder">
    <h2>{$t("common.error")}</h2>
    <p>{error}</p>
    <button class="btn" onclick={load}>{$t("common.retry")}</button>
  </div>
{:else if !set || set.entries.length === 0}
  <div class="groups-placeholder" data-testid="groups-no-profiles">
    <p>{$t("lighting.groups.noProfiles")}</p>
    <a class="btn" href="#/config">{$t("nav.config")}</a>
  </div>
{:else}
  <div class="groups-page">
    <div class="groups-toolbar">
      <div class="groups-picker">
        <label for="groups-profile">{$t("lighting.groups.profile")}</label>
        <select
          id="groups-profile"
          class="input"
          bind:this={selectEl}
          value={selected?.name}
          onchange={onPick}
        >
          {#each set.entries as entry (entry.name)}
            <option value={entry.name}>
              {entry.name}{entry.hostname && entry.hostname === running
                ? ` (${$t("lighting.groups.running")})`
                : ""}
            </option>
          {/each}
        </select>
      </div>
      <div class="groups-actions">
        {#if saveOk}
          <span class="save-msg">{$t("common.saved")}</span>
        {:else if saveMsg}
          <span class="save-msg save-error">{saveMsg}</span>
        {/if}
        {#if dirty && !saving}
          <span class="dirty-flag">{$t("common.unsaved")}</span>
        {/if}
        <button
          class="btn"
          class:btn-primary={dirty && !$playbackStore.locked}
          onclick={save}
          disabled={saving || !dirty || $playbackStore.locked || !profile?.dmx}
          title={$playbackStore.locked ? $t("common.locked") : null}
        >
          {saving ? $t("common.saving") : $t("common.save")}
        </button>
      </div>
    </div>

    {#if profile && profile.dmx}
      <div class="groups-body">
        <ProfileLightingPanel bind:lighting={profile.dmx.lighting} {onchange} />
      </div>
    {:else if profile}
      <div class="groups-placeholder" data-testid="groups-no-dmx">
        <p>{$t("lighting.groups.noDmx")}</p>
        {#if selected}
          <a
            class="btn"
            href={`#/config/${encodeURIComponent(selected.name)}/lighting`}
            >{$t("lighting.groups.enableInConfig")}</a
          >
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .groups-page {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .groups-toolbar {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 14px 20px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    box-shadow: var(--nc-shadow-xs);
  }
  .groups-picker {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 200px;
  }
  .groups-picker label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .groups-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .groups-body {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 16px;
  }
  .groups-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 40px 20px;
    text-align: center;
    color: var(--text-dim);
  }
  .dirty-flag {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-cyan-600);
  }
  :global(.nc--dark) .dirty-flag {
    color: var(--nc-cyan-300);
  }
  .save-msg {
    font-size: 13px;
    color: var(--green);
  }
  .save-error {
    color: var(--red);
  }
</style>
