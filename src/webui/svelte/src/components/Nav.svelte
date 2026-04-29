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
  import { wsConnected, playbackStore } from "../lib/ws/stores";
  import { setLocked } from "../lib/api/config";
  import { t } from "svelte-i18n";
  import NavDrawer from "./NavDrawer.svelte";
  import BrandMark from "./BrandMark.svelte";

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();
  let drawerOpen = $state(false);
  let toggling = $state(false);

  async function toggleLock() {
    toggling = true;
    try {
      await setLocked(!$playbackStore.locked);
    } catch (e) {
      console.error("Failed to toggle lock:", e);
    } finally {
      toggling = false;
    }
  }

  const links = [
    { hash: "#/", labelKey: "nav.dashboard" },
    { hash: "#/songs", labelKey: "nav.songs" },
    { hash: "#/playlists", labelKey: "nav.playlists" },
    { hash: "#/config", labelKey: "nav.config" },
    { hash: "#/status", labelKey: "nav.status" },
  ];

  function isActive(hash: string): boolean {
    if (hash === "#/") return currentHash === "#/" || currentHash === "";
    return currentHash.startsWith(hash);
  }

  function closeDrawer() {
    drawerOpen = false;
  }
</script>

<nav class="topnav">
  <button
    class="topnav__hamburger"
    onclick={() => (drawerOpen = true)}
    aria-label={$t("nav.menu")}
    aria-expanded={drawerOpen}
  >
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.7"
      stroke-linecap="round"
      stroke-linejoin="round"
      ><path d="M4 7h16M4 12h16M4 17h16" /></svg
    >
  </button>
  <a class="topnav__brand" href="#/">
    <BrandMark />
    <span>mtrack</span>
  </a>
  <div class="topnav__tabs" role="tablist">
    {#each links as link (link.hash)}
      <a
        href={link.hash}
        class="topnav__tab"
        class:topnav__tab--active={isActive(link.hash)}
        role="tab"
        aria-selected={isActive(link.hash)}
      >
        {$t(link.labelKey)}
      </a>
    {/each}
  </div>
  <div class="topnav__right">
    {#if $playbackStore.song_name}
      <div
        class="topnav__transport"
        title={$playbackStore.is_playing
          ? $t("playback.playing")
          : $t("playback.stopped")}
      >
        <span
          class="topnav__transport-state"
          class:topnav__transport-state--playing={$playbackStore.is_playing}
          class:topnav__transport-state--stopped={!$playbackStore.is_playing}
          aria-hidden="true"
        >
          {#if $playbackStore.is_playing}
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"
              ><rect x="6" y="5" width="4" height="14" rx="1" /><rect
                x="14"
                y="5"
                width="4"
                height="14"
                rx="1"
              /></svg
            >
          {:else}
            <svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"
              ><path d="M8 5v14l11-7z" /></svg
            >
          {/if}
        </span>
        <span class="topnav__transport-song">{$playbackStore.song_name}</span>
      </div>
    {/if}
    <button
      class="topnav__lock"
      class:topnav__lock--locked={$playbackStore.locked}
      onclick={toggleLock}
      disabled={toggling}
      title={$playbackStore.locked
        ? $t("nav.lock.lockedHint")
        : $t("nav.lock.unlockedHint")}
      aria-label={$playbackStore.locked
        ? $t("nav.lock.lockedHint")
        : $t("nav.lock.unlockedHint")}
    >
      {#if $playbackStore.locked}
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><rect x="5" y="11" width="14" height="10" rx="2" /><path
            d="M8 11V8a4 4 0 0 1 8 0v3"
          /></svg
        >
      {:else}
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><rect x="5" y="11" width="14" height="10" rx="2" /><path
            d="M8 11V8a4 4 0 0 1 7.8-1"
          /></svg
        >
      {/if}
    </button>
    <span
      class="topnav__conn"
      class:topnav__conn--off={!$wsConnected}
      title={$wsConnected
        ? $t("nav.connection.connected")
        : $t("nav.connection.disconnected")}
      role="status"
      aria-label={$wsConnected
        ? $t("nav.connection.serverConnected")
        : $t("nav.connection.serverDisconnected")}
    >
      <span class="topnav__conn-dot" aria-hidden="true"></span>
      <span class="sr-only"
        >{$wsConnected
          ? $t("nav.connection.connected")
          : $t("nav.connection.disconnected")}</span
      >
    </span>
  </div>
</nav>

<NavDrawer
  open={drawerOpen}
  {links}
  {currentHash}
  locked={$playbackStore.locked}
  {toggling}
  onClose={closeDrawer}
  onToggleLock={toggleLock}
/>

{#if !$wsConnected}
  <div class="disconnect-banner" role="alert">
    {$t("nav.connection.banner")}
  </div>
{/if}

<style>
  .topnav {
    display: flex;
    align-items: center;
    height: 56px;
    padding: 0 24px;
    border-bottom: 1px solid var(--nc-border-1);
    background: var(--nc-bg-1);
    position: sticky;
    top: 0;
    z-index: var(--z-nav);
    gap: 4px;
  }
  .topnav__hamburger {
    display: none;
    width: 36px;
    height: 36px;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    border: 1px solid var(--nc-border-1);
    background: var(--nc-bg-2);
    color: var(--nc-fg-1);
    cursor: pointer;
    margin-right: 12px;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .topnav__hamburger:hover {
    background: var(--nc-bg-3);
    border-color: var(--nc-fg-3);
  }
  .topnav__brand {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-family: var(--nc-font-display);
    font-weight: 800;
    font-size: 18px;
    line-height: 1;
    letter-spacing: -0.02em;
    color: var(--nc-cyan-500);
    text-decoration: none;
    margin-right: 24px;
    border: none;
  }
  :global(.nc--dark) .topnav__brand {
    color: var(--nc-cyan-300);
  }
  .topnav__tabs {
    display: flex;
    gap: 2px;
    flex: 1;
  }
  .topnav__tab {
    font-family: var(--nc-font-display);
    font-weight: 600;
    font-size: 14px;
    line-height: 1;
    padding: 8px 14px;
    border-radius: 8px;
    color: var(--nc-fg-2);
    cursor: pointer;
    border: none;
    background: transparent;
    text-decoration: none;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease);
  }
  .topnav__tab:hover {
    color: var(--nc-fg-1);
    background: var(--nc-bg-2);
  }
  .topnav__tab--active {
    color: var(--nc-fg-1);
    background: var(--nc-bg-2);
    box-shadow: inset 0 -2px 0 var(--nc-cyan-400);
    border-radius: 8px 8px 0 0;
  }
  .topnav__right {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .topnav__transport {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px 6px 8px;
    border-radius: 999px;
    background: var(--nc-bg-2);
    border: 1px solid var(--nc-border-1);
    font-family: var(--nc-font-sans);
    font-weight: 500;
    font-size: 13px;
    line-height: 1;
    max-width: 240px;
  }
  .topnav__transport-state {
    width: 22px;
    height: 22px;
    border-radius: 999px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--nc-pink-400);
    color: var(--nc-ink);
    flex: 0 0 auto;
  }
  .topnav__transport-state--playing {
    background: var(--nc-pink-400);
    animation: ncPulsePink 1.6s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  .topnav__transport-state--stopped {
    background: var(--nc-gray-300);
  }
  @keyframes ncPulsePink {
    0%,
    100% {
      box-shadow: 0 0 0 0 rgba(239, 96, 163, 0.5);
    }
    50% {
      box-shadow: 0 0 0 6px rgba(239, 96, 163, 0);
    }
  }
  .topnav__transport-song {
    color: var(--nc-fg-1);
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .topnav__lock,
  .topnav__conn {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--nc-border-1);
    background: var(--nc-bg-2);
    color: var(--nc-fg-2);
    cursor: pointer;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .topnav__lock {
    padding: 0;
  }
  .topnav__lock:hover {
    background: var(--nc-bg-3);
    color: var(--nc-fg-1);
  }
  .topnav__lock--locked {
    color: var(--nc-warn);
    border-color: rgba(242, 181, 68, 0.45);
    background: rgba(242, 181, 68, 0.12);
  }
  .topnav__lock:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .topnav__conn {
    cursor: default;
  }
  .topnav__conn-dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--nc-success);
    box-shadow: 0 0 8px rgba(77, 192, 138, 0.6);
  }
  .topnav__conn--off .topnav__conn-dot {
    background: var(--nc-error);
    box-shadow: 0 0 8px rgba(232, 75, 75, 0.6);
    animation: ncPulseDisconnect 2s ease-in-out infinite;
  }
  @keyframes ncPulseDisconnect {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  .disconnect-banner {
    background: rgba(232, 75, 75, 0.12);
    color: var(--nc-error);
    text-align: center;
    padding: 6px 12px;
    font-size: 13px;
    font-weight: 500;
    border-bottom: 1px solid rgba(232, 75, 75, 0.4);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border-width: 0;
  }

  @media (max-width: 720px) {
    .topnav {
      padding: 0 14px;
      height: 52px;
      gap: 0;
    }
    .topnav__brand {
      margin-right: 0;
    }
    .topnav__hamburger {
      display: inline-flex;
    }
    .topnav__tabs {
      display: none;
    }
    .topnav__transport {
      display: none;
    }
    .topnav__lock {
      display: none;
    }
    .topnav__right {
      margin-left: auto;
    }
  }
</style>
