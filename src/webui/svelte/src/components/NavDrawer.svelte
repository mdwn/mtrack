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
  import { wsConnected } from "../lib/ws/stores";
  import { t } from "svelte-i18n";
  import BrandMark from "./BrandMark.svelte";

  interface NavLink {
    hash: string;
    labelKey: string;
  }

  interface Props {
    open: boolean;
    links: NavLink[];
    currentHash: string;
    locked: boolean;
    toggling: boolean;
    onClose: () => void;
    onToggleLock: () => void;
  }

  let {
    open,
    links,
    currentHash,
    locked,
    toggling,
    onClose,
    onToggleLock,
  }: Props = $props();

  function isActive(hash: string): boolean {
    if (hash === "#/") return currentHash === "#/" || currentHash === "";
    return currentHash.startsWith(hash);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }

  // Trap focus within the drawer when open. Best-effort: focus first item.
  let drawerEl: HTMLElement | null = $state(null);
  $effect(() => {
    if (open && drawerEl) {
      const firstLink = drawerEl.querySelector<HTMLElement>(
        "a.drawer__item, button.drawer__item",
      );
      firstLink?.focus();
    }
  });
</script>

<svelte:window on:keydown={onKeyDown} />

<div
  class="drawer-backdrop"
  class:drawer-backdrop--open={open}
  onclick={onClose}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") onClose();
  }}
  role="presentation"
  aria-hidden="true"
></div>
<aside
  bind:this={drawerEl}
  class="drawer"
  class:drawer--open={open}
  aria-hidden={!open}
  aria-label={$t("nav.menu")}
>
  <div class="drawer__head">
    <span class="drawer__brand">
      <BrandMark />
      <span>mtrack</span>
    </span>
    <button class="drawer__close" onclick={onClose} aria-label={$t("common.close")}>
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
        ><path d="M6 6l12 12M18 6L6 18" /></svg
      >
    </button>
  </div>
  <nav class="drawer__nav">
    {#each links as link (link.hash)}
      <a
        class="drawer__item"
        class:drawer__item--active={isActive(link.hash)}
        href={link.hash}
        onclick={onClose}
      >
        <span>{$t(link.labelKey)}</span>
        {#if isActive(link.hash)}
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><path d="M9 6l6 6-6 6" /></svg
          >
        {/if}
      </a>
    {/each}
  </nav>
  <div class="drawer__foot">
    <button
      class="drawer__lock"
      class:drawer__lock--locked={locked}
      onclick={onToggleLock}
      disabled={toggling}
      aria-label={locked
        ? $t("nav.lock.lockedHint")
        : $t("nav.lock.unlockedHint")}
    >
      {#if locked}
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
    <span class="drawer__foot-label">
      {locked ? $t("nav.lock.locked") : $t("nav.lock.unlocked")}
    </span>
    <span
      class="drawer__conn"
      class:drawer__conn--off={!$wsConnected}
      title={$wsConnected
        ? $t("nav.connection.connected")
        : $t("nav.connection.disconnected")}
      aria-hidden="true"
    >
      <span class="drawer__conn-dot"></span>
    </span>
  </div>
</aside>

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 100;
    opacity: 0;
    pointer-events: none;
    transition: opacity 200ms var(--nc-ease);
    border: none;
  }
  .drawer-backdrop--open {
    opacity: 1;
    pointer-events: auto;
  }
  .drawer {
    position: fixed;
    left: 0;
    top: 0;
    bottom: 0;
    width: 280px;
    max-width: 84vw;
    background: var(--card-bg);
    border-right: 1px solid var(--card-border);
    z-index: 101;
    transform: translateX(-100%);
    transition: transform 240ms var(--nc-ease);
    display: flex;
    flex-direction: column;
    box-shadow: 8px 0 32px rgba(0, 0, 0, 0.3);
  }
  .drawer--open {
    transform: translateX(0);
  }
  .drawer__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--card-border);
  }
  .drawer__brand {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-family: var(--nc-font-display);
    font-weight: 800;
    font-size: 18px;
    line-height: 1;
    color: var(--nc-cyan-500);
    letter-spacing: -0.02em;
  }
  :global(.nc--dark) .drawer__brand {
    color: var(--nc-cyan-300);
  }
  .drawer__close {
    width: 36px;
    height: 36px;
    border-radius: 8px;
    border: 1px solid var(--card-border);
    background: transparent;
    color: var(--nc-fg-2);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .drawer__close:hover {
    background: var(--nc-bg-2);
    color: var(--nc-fg-1);
  }
  .drawer__nav {
    display: flex;
    flex-direction: column;
    padding: 12px 8px;
    gap: 2px;
    flex: 1;
  }
  .drawer__item {
    text-align: left;
    font-family: var(--nc-font-display);
    font-weight: 600;
    font-size: 16px;
    line-height: 1;
    padding: 14px 16px;
    border-radius: 10px;
    border: none;
    background: transparent;
    color: var(--nc-fg-1);
    text-decoration: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 48px;
  }
  .drawer__item:hover {
    background: var(--nc-bg-2);
  }
  .drawer__item--active {
    background: rgba(94, 202, 234, 0.14);
    color: var(--nc-cyan-600);
    box-shadow: inset 3px 0 0 var(--nc-cyan-400);
  }
  :global(.nc--dark) .drawer__item--active {
    color: var(--nc-cyan-300);
  }
  .drawer__foot {
    border-top: 1px solid var(--card-border);
    padding: 14px 20px;
    display: flex;
    align-items: center;
    gap: 12px;
    font-family: var(--nc-font-sans);
    font-weight: 500;
    font-size: 13px;
    color: var(--nc-fg-2);
  }
  .drawer__lock {
    width: 36px;
    height: 36px;
    border-radius: 8px;
    border: 1px solid var(--nc-border-1);
    background: var(--nc-bg-2);
    color: var(--nc-fg-2);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .drawer__lock:hover {
    background: var(--nc-bg-3);
    color: var(--nc-fg-1);
  }
  .drawer__lock--locked {
    color: var(--nc-warn);
    border-color: rgba(242, 181, 68, 0.45);
    background: rgba(242, 181, 68, 0.12);
  }
  .drawer__lock:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .drawer__foot-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .drawer__conn {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--nc-border-1);
    background: var(--nc-bg-2);
  }
  .drawer__conn-dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--nc-success);
    box-shadow: 0 0 8px rgba(77, 192, 138, 0.6);
  }
  .drawer__conn--off .drawer__conn-dot {
    background: var(--nc-error);
    box-shadow: 0 0 8px rgba(232, 75, 75, 0.6);
  }
</style>
