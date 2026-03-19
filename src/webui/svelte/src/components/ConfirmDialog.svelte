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
  import { t } from "svelte-i18n";
  import { dialogState } from "../lib/dialog.svelte";

  let inputValue = $state("");
  let inputEl: HTMLInputElement | undefined = $state();
  let previousFocus: Element | null = null;

  function confirm() {
    if (dialogState.mode === "prompt") {
      dialogState.resolve?.(inputValue);
    } else if (dialogState.mode === "alert") {
      dialogState.resolve?.(null);
    } else {
      dialogState.resolve?.(true);
    }
    close();
  }

  function cancel() {
    if (dialogState.mode === "prompt") {
      dialogState.resolve?.(null);
    } else {
      dialogState.resolve?.(false);
    }
    close();
  }

  function close() {
    dialogState.open = false;
    dialogState.resolve = null;
    inputValue = "";
    if (previousFocus instanceof HTMLElement) {
      previousFocus.focus();
    }
    previousFocus = null;
  }

  function onKeydown(e: KeyboardEvent) {
    if (!dialogState.open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      cancel();
    } else if (e.key === "Enter") {
      e.preventDefault();
      confirm();
    }
  }

  $effect(() => {
    if (dialogState.open) {
      previousFocus = document.activeElement;
      if (dialogState.mode === "prompt") {
        inputValue = dialogState.options.defaultValue ?? "";
        // Focus input after render
        requestAnimationFrame(() => inputEl?.focus());
      }
    }
  });

  let confirmLabel = $derived(
    dialogState.options.confirmLabel ??
      (dialogState.mode === "alert" ? $t("common.ok") : $t("common.confirm")),
  );

  let cancelLabel = $derived(
    dialogState.options.cancelLabel ?? $t("common.cancel"),
  );
</script>

<svelte:window onkeydown={onKeydown} />

{#if dialogState.open}
  <div
    class="dialog-overlay"
    onclick={cancel}
    onkeydown={() => {}}
    role="dialog"
    aria-modal="true"
    tabindex="-1"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="dialog-card"
      onclick={(e) => e.stopPropagation()}
      onkeydown={() => {}}
    >
      <div class="dialog-message">{dialogState.message}</div>

      {#if dialogState.mode === "prompt"}
        <input
          bind:this={inputEl}
          class="input dialog-input"
          type="text"
          placeholder={dialogState.options.placeholder ?? ""}
          bind:value={inputValue}
        />
      {/if}

      <div class="dialog-actions">
        {#if dialogState.mode !== "alert"}
          <button class="btn" onclick={cancel}>{cancelLabel}</button>
        {/if}
        <button
          class="btn {dialogState.options.danger
            ? 'btn-danger'
            : 'btn-primary'}"
          onclick={confirm}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 300;
    padding: 24px;
  }
  .dialog-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg, 8px);
    padding: 20px 24px;
    max-width: 420px;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 16px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .dialog-message {
    font-size: 14px;
    line-height: 1.5;
    color: var(--text);
    white-space: pre-line;
  }
  .dialog-input {
    width: 100%;
  }
  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
