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

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();
  let menuOpen = $state(false);
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
    { hash: "#/config", labelKey: "nav.config" },
    { hash: "#/songs", labelKey: "nav.songs" },
    { hash: "#/playlists", labelKey: "nav.playlists" },
    { hash: "#/status", labelKey: "nav.status" },
  ];

  function isActive(hash: string): boolean {
    if (hash === "#/") return currentHash === "#/" || currentHash === "";
    return currentHash.startsWith(hash);
  }

  let activePageName = $derived(links.find((l) => isActive(l.hash))?.labelKey);

  function closeMenu() {
    menuOpen = false;
  }
</script>

<nav class="nav">
  <span class="nav-brand"
    >mtrack{#if activePageName}<span class="nav-page-name">
        / {$t(activePageName)}</span
      >{/if}</span
  >
  <button
    class="hamburger"
    onclick={() => (menuOpen = !menuOpen)}
    aria-label={$t("nav.menu")}
    aria-expanded={menuOpen}
    aria-controls="nav-links"
  >
    <span class="hamburger-line"></span>
    <span class="hamburger-line"></span>
    <span class="hamburger-line"></span>
  </button>
  <div class="nav-links" class:open={menuOpen} id="nav-links">
    {#each links as link (link.hash)}
      <a
        href={link.hash}
        class="nav-link"
        class:active={isActive(link.hash)}
        onclick={closeMenu}
      >
        {$t(link.labelKey)}
      </a>
    {/each}
  </div>
  {#if $playbackStore.song_name}
    <div class="now-playing">
      <span class="now-playing-icon" aria-hidden="true">
        {#if $playbackStore.is_playing}
          <svg width="10" height="12" viewBox="0 0 10 12" fill="currentColor"
            ><path d="M0 0l10 6-10 6z" /></svg
          >
        {:else}
          <svg width="10" height="12" viewBox="0 0 10 12" fill="currentColor"
            ><rect x="0" y="0" width="3" height="12" /><rect
              x="7"
              y="0"
              width="3"
              height="12"
            /></svg
          >
        {/if}
      </span>
      <span class="now-playing-song">{$playbackStore.song_name}</span>
    </div>
  {/if}
  <div class="nav-status">
    <button
      class="lock-toggle"
      class:locked={$playbackStore.locked}
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
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><rect x="3" y="11" width="18" height="11" rx="2" ry="2" /><path
            d="M7 11V7a5 5 0 0 1 10 0v4"
          /></svg
        >
      {:else}
        <svg
          width="14"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          ><rect x="3" y="11" width="18" height="11" rx="2" ry="2" /><path
            d="M7 11V7a5 5 0 0 1 9.9-1"
          /></svg
        >
      {/if}
    </button>
    <div
      class="status-indicator"
      class:connected={$wsConnected}
      class:disconnected={!$wsConnected}
      title={$wsConnected
        ? $t("nav.connection.connected")
        : $t("nav.connection.disconnected")}
      role="status"
      aria-label={$wsConnected
        ? $t("nav.connection.serverConnected")
        : $t("nav.connection.serverDisconnected")}
    >
      <span class="sr-only"
        >{$wsConnected
          ? $t("nav.connection.connected")
          : $t("nav.connection.disconnected")}</span
      >
    </div>
  </div>
</nav>
{#if !$wsConnected}
  <div class="disconnect-banner" role="alert">
    {$t("nav.connection.banner")}
  </div>
{/if}

<style>
  .nav {
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 0 20px;
    height: 48px;
    background: var(--bg-card);
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    z-index: var(--z-nav);
  }
  .nav-brand {
    font-family: var(--sans);
    font-weight: 600;
    font-size: 16px;
    color: var(--accent);
    letter-spacing: -0.5px;
  }
  .nav-page-name {
    display: none;
    color: var(--text-muted);
    font-weight: 400;
    font-size: 14px;
  }
  .hamburger {
    display: none;
    flex-direction: column;
    justify-content: center;
    gap: 4px;
    background: none;
    border: none;
    padding: 4px;
    cursor: pointer;
    margin-left: auto;
  }
  .hamburger-line {
    display: block;
    width: 18px;
    height: 2px;
    background: var(--text-muted);
    border-radius: 1px;
  }
  .nav-links {
    display: flex;
    gap: 4px;
    flex: 1;
  }
  .nav-link {
    color: var(--text-muted);
    text-decoration: none;
    padding: 6px 12px;
    border-radius: var(--radius);
    font-size: 14px;
    font-weight: 500;
    transition:
      color 0.15s,
      background 0.15s;
  }
  .nav-link:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.05);
  }
  .nav-link.active {
    color: var(--text);
    background: var(--accent-subtle);
  }
  .now-playing {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    color: var(--text-muted);
    min-width: 0;
  }
  .now-playing-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    color: var(--pink);
  }
  .now-playing-song {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 300px;
  }
  .nav-status {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .lock-toggle {
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius);
    padding: 2px 6px;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .lock-toggle:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: var(--border);
  }
  .lock-toggle.locked {
    color: var(--yellow, #eab308);
  }
  .status-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
    transition: background 0.3s;
    border: 1.5px solid transparent;
  }
  .status-indicator.connected {
    background: var(--green);
  }
  .status-indicator.disconnected {
    background: var(--red);
    animation: pulse-disconnect 2s ease-in-out infinite;
  }
  @keyframes pulse-disconnect {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  .disconnect-banner {
    background: var(--red-dim);
    color: var(--red);
    text-align: center;
    padding: 6px 12px;
    font-size: 13px;
    font-weight: 500;
    border-bottom: 1px solid var(--red);
  }

  @media (max-width: 600px) {
    .nav-page-name {
      display: inline;
    }
    .hamburger {
      display: flex;
    }
    .now-playing {
      max-width: 100px;
      font-size: 12px;
    }
    .now-playing-song {
      max-width: 150px;
    }
    .nav-status {
      margin-left: 12px;
    }
    .nav-links {
      display: none;
      position: absolute;
      top: 48px;
      left: 0;
      right: 0;
      flex-direction: column;
      background: var(--bg-card);
      border-bottom: 1px solid var(--border);
      padding: 8px 12px;
      gap: 2px;
    }
    .nav-links.open {
      display: flex;
    }
  }
</style>
