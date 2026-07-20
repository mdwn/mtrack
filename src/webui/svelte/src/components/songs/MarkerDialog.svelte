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
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    /** Footer buttons (rendered left of the Done button). */
    actions?: Snippet;
  }

  let { title, onclose, children, actions }: Props = $props();

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div
  class="marker-dialog-overlay"
  onclick={onclose}
  onkeydown={() => {}}
  role="presentation"
>
  <div
    class="marker-dialog"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={() => {}}
  >
    <div class="marker-dialog-header">
      <span class="marker-dialog-title">{title}</span>
    </div>
    <div class="marker-dialog-body">
      {@render children()}
    </div>
    <div class="marker-dialog-footer">
      {#if actions}
        {@render actions()}
      {/if}
      <span class="footer-spacer"></span>
      <button type="button" class="btn btn-primary done-btn" onclick={onclose}
        >Done</button
      >
    </div>
  </div>
</div>

<style>
  .marker-dialog-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: flex-end;
    justify-content: center;
    z-index: 200;
  }
  /* Bottom sheet on phones, centered card on wider screens. */
  .marker-dialog {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 14px 14px 0 0;
    width: 100%;
    max-width: 480px;
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: dialog-in 0.15s ease-out;
  }
  @media (min-width: 640px) {
    .marker-dialog-overlay {
      align-items: center;
      padding: 32px;
    }
    .marker-dialog {
      border-radius: 14px;
    }
  }
  @keyframes dialog-in {
    from {
      transform: translateY(24px);
      opacity: 0;
    }
    to {
      transform: translateY(0);
      opacity: 1;
    }
  }
  .marker-dialog-header {
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .marker-dialog-title {
    font-size: 14px;
    font-weight: 600;
  }
  .marker-dialog-body {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
  }
  .marker-dialog-footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid var(--border);
  }
  .footer-spacer {
    flex: 1;
  }
  .done-btn {
    min-height: 40px;
    min-width: 88px;
  }
</style>
