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
  import { logStore } from "../../lib/ws/stores";
  import { tick } from "svelte";
  import { t } from "svelte-i18n";
  import { SvelteSet } from "svelte/reactivity";

  const ALL_LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] as const;
  let enabledLevels = new SvelteSet(["INFO", "WARN", "ERROR"]);

  function toggleLevel(level: string) {
    if (enabledLevels.has(level)) {
      enabledLevels.delete(level);
    } else {
      enabledLevels.add(level);
    }
  }

  let filteredLogs = $derived(
    $logStore.filter((line) => enabledLevels.has(line.level)),
  );

  let container: HTMLDivElement | undefined = $state();
  let autoScroll = $state(true);

  function handleScroll() {
    if (!container) return;
    const atBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight <
      30;
    autoScroll = atBottom;
  }

  $effect(() => {
    void filteredLogs;
    if (autoScroll && container) {
      tick().then(() => {
        container!.scrollTop = container!.scrollHeight;
      });
    }
  });
</script>

<section class="card logs-card">
  <header class="logs-card__head">
    <div>
      <div class="overline">{$t("logs.title")}</div>
      <div class="logs-card__title">Streaming events</div>
    </div>
    <div class="logs-card__filters">
      {#each ALL_LEVELS as level (level)}
        <button
          class="logs-card__pill logs-card__pill--{level}"
          class:logs-card__pill--active={enabledLevels.has(level)}
          onclick={() => toggleLevel(level)}
          aria-pressed={enabledLevels.has(level)}
        >
          {level}
        </button>
      {/each}
    </div>
  </header>
  <div
    class="logs-card__feed"
    bind:this={container}
    onscroll={handleScroll}
    role="log"
    aria-label={$t("logs.title")}
  >
    {#each filteredLogs as line, i (i)}
      <div class="logs-card__line logs-card__line--{line.level}">
        <span class="logs-card__lvl">{line.level}</span>
        <span class="logs-card__target">{line.target}</span>
        <span class="logs-card__msg">{line.message}</span>
      </div>
    {/each}
  </div>
</section>

<style>
  .logs-card {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .logs-card__head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    padding: 16px 20px;
    gap: 12px;
    border-bottom: 1px solid var(--card-border);
    flex-wrap: wrap;
  }
  .logs-card__title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 16px;
    margin-top: 4px;
    color: var(--nc-fg-1);
  }
  .logs-card__filters {
    display: flex;
    gap: 4px;
  }
  .logs-card__pill {
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.08em;
    padding: 5px 9px;
    border-radius: 999px;
    border: 1px solid var(--card-border);
    background: var(--nc-bg-2);
    color: var(--nc-fg-3);
    cursor: pointer;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease),
      border-color var(--nc-dur-fast) var(--nc-ease);
  }
  .logs-card__pill:hover {
    border-color: var(--nc-fg-3);
    color: var(--nc-fg-2);
  }
  .logs-card__pill--active.logs-card__pill--ERROR {
    background: rgba(232, 75, 75, 0.15);
    color: var(--nc-pink-600);
    border-color: rgba(232, 75, 75, 0.45);
  }
  :global(.nc--dark) .logs-card__pill--active.logs-card__pill--ERROR {
    color: var(--nc-pink-300);
  }
  .logs-card__pill--active.logs-card__pill--WARN {
    background: rgba(242, 181, 68, 0.18);
    color: #b47a1a;
    border-color: rgba(242, 181, 68, 0.45);
  }
  :global(.nc--dark) .logs-card__pill--active.logs-card__pill--WARN {
    color: var(--nc-warn);
  }
  .logs-card__pill--active.logs-card__pill--INFO {
    background: rgba(94, 202, 234, 0.18);
    color: var(--nc-cyan-600);
    border-color: rgba(94, 202, 234, 0.45);
  }
  :global(.nc--dark) .logs-card__pill--active.logs-card__pill--INFO {
    color: var(--nc-cyan-300);
  }
  .logs-card__pill--active.logs-card__pill--DEBUG {
    background: var(--nc-bg-3);
    color: var(--nc-fg-2);
    border-color: var(--nc-fg-4);
  }
  .logs-card__pill--active.logs-card__pill--TRACE {
    background: var(--nc-bg-3);
    color: var(--nc-fg-3);
    border-color: var(--nc-fg-4);
    opacity: 0.85;
  }

  .logs-card__feed {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    line-height: 1.55;
    flex: 1;
    overflow-y: auto;
    max-height: 320px;
    padding: 12px 20px;
    background: var(--inset-bg);
  }
  .logs-card__line {
    display: grid;
    grid-template-columns: 56px max-content 1fr;
    gap: 10px;
    padding: 2px 8px;
    margin: 0 -8px;
    border-left: 3px solid transparent;
    word-break: break-word;
  }
  .logs-card__lvl {
    font-weight: 700;
  }
  .logs-card__line--ERROR {
    background: rgba(232, 75, 75, 0.1);
    border-left-color: var(--nc-error);
    font-weight: 600;
  }
  .logs-card__line--ERROR .logs-card__lvl {
    color: var(--nc-pink-600);
  }
  :global(.nc--dark) .logs-card__line--ERROR .logs-card__lvl {
    color: var(--nc-pink-300);
  }
  .logs-card__line--WARN {
    background: rgba(242, 181, 68, 0.1);
    border-left-color: var(--nc-warn);
  }
  .logs-card__line--WARN .logs-card__lvl {
    color: #b47a1a;
  }
  :global(.nc--dark) .logs-card__line--WARN .logs-card__lvl {
    color: var(--nc-warn);
  }
  .logs-card__line--INFO .logs-card__lvl {
    color: var(--nc-cyan-600);
  }
  :global(.nc--dark) .logs-card__line--INFO .logs-card__lvl {
    color: var(--nc-cyan-300);
  }
  .logs-card__line--DEBUG .logs-card__lvl {
    color: var(--nc-fg-3);
  }
  .logs-card__line--TRACE .logs-card__lvl {
    color: var(--nc-fg-4);
  }
  .logs-card__line--TRACE {
    opacity: 0.75;
  }
  .logs-card__target {
    color: var(--nc-fg-4);
  }
  .logs-card__msg {
    color: var(--nc-fg-2);
  }
</style>
