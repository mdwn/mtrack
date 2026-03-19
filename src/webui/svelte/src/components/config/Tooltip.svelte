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
  interface Props {
    text: string;
  }

  let { text }: Props = $props();

  let open = $state(false);
  let usedTouch = $state(false);
  let wrapEl: HTMLSpanElement | undefined = $state();
  let iconEl: HTMLButtonElement | undefined = $state();
  let portalEl: HTMLDivElement | undefined = $state();

  function onPointerDown(e: PointerEvent) {
    // Track whether this interaction is touch so hover handlers can defer.
    usedTouch = e.pointerType === "touch";
  }

  function toggle() {
    open = !open;
  }

  function onWindowPointerDown(e: PointerEvent) {
    if (open && wrapEl && !wrapEl.contains(e.target as Node)) {
      open = false;
    }
  }

  function position() {
    if (!iconEl || !portalEl) return;
    const rect = iconEl.getBoundingClientRect();
    const popover = portalEl;
    const centerX = rect.left + rect.width / 2;
    let left = centerX - popover.offsetWidth / 2;

    // Keep within viewport horizontally
    const margin = 8;
    if (left < margin) left = margin;
    if (left + popover.offsetWidth > window.innerWidth - margin) {
      left = window.innerWidth - margin - popover.offsetWidth;
    }

    popover.style.left = `${left}px`;
    popover.style.top = `${rect.top - popover.offsetHeight - 8 + window.scrollY}px`;

    // Position arrow to point at icon center
    const arrowLeft = centerX - left;
    popover.style.setProperty("--arrow-left", `${arrowLeft}px`);
  }

  $effect(() => {
    if (open && portalEl) {
      position();
    }
  });
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<span class="tooltip-wrap" bind:this={wrapEl}>
  <button
    class="tooltip-icon"
    type="button"
    bind:this={iconEl}
    onpointerdown={onPointerDown}
    onclick={toggle}
    onmouseenter={() => {
      if (!usedTouch) open = true;
    }}
    onmouseleave={() => {
      if (!usedTouch) open = false;
    }}
    aria-label="More info"
  >
    i
  </button>
</span>

{#if open}
  <div class="tooltip-portal" bind:this={portalEl} role="tooltip">
    {text}
  </div>
{/if}

<style>
  .tooltip-wrap {
    display: inline-flex;
    align-items: center;
    margin-left: 4px;
    vertical-align: middle;
  }
  .tooltip-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid var(--text-dim);
    background: none;
    color: var(--text-dim);
    font-size: 10px;
    font-weight: 700;
    font-style: italic;
    font-family: serif;
    line-height: 1;
    cursor: help;
    padding: 0;
    transition:
      color 0.15s,
      border-color 0.15s;
  }
  .tooltip-icon:hover {
    color: var(--accent);
    border-color: var(--accent);
  }
  .tooltip-portal {
    position: absolute;
    top: 0;
    left: 0;
    width: max-content;
    max-width: 280px;
    padding: 8px 12px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 400;
    font-style: normal;
    line-height: 1.45;
    text-transform: none;
    letter-spacing: normal;
    white-space: normal;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    z-index: 9999;
    pointer-events: none;
  }
  .tooltip-portal::after {
    content: "";
    position: absolute;
    top: 100%;
    left: var(--arrow-left, 50%);
    transform: translateX(-50%);
    border: 5px solid transparent;
    border-top-color: var(--border);
  }
</style>
