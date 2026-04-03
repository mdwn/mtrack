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

  const ALL_LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] as const;
  let enabledLevels = $state(new Set(["INFO", "WARN", "ERROR"]));

  function toggleLevel(level: string) {
    const next = new Set(enabledLevels);
    if (next.has(level)) {
      next.delete(level);
    } else {
      next.add(level);
    }
    enabledLevels = next;
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
    // Re-run whenever filteredLogs changes
    void filteredLogs;
    if (autoScroll && container) {
      tick().then(() => {
        container!.scrollTop = container!.scrollHeight;
      });
    }
  });
</script>

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">{$t("logs.title")}</span>
    <div class="log-level-filters">
      {#each ALL_LEVELS as level}
        <button
          class="log-level-pill level-{level}"
          class:active={enabledLevels.has(level)}
          onclick={() => toggleLevel(level)}
        >
          {level}
        </button>
      {/each}
    </div>
  </div>
  <div class="log-container" bind:this={container} onscroll={handleScroll}>
    {#each filteredLogs as line, i (i)}
      <div class="log-line level-{line.level}">
        <span class="log-level">{line.level}</span>
        <span class="log-target">{line.target}</span>:
        <span class="log-message">{line.message}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .log-level-filters {
    display: flex;
    gap: 4px;
  }
  .log-level-pill {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.3px;
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-dim);
    cursor: pointer;
    transition: background 0.15s, color 0.15s, border-color 0.15s;
  }
  .log-level-pill:hover {
    border-color: var(--text-dim);
  }
  .log-level-pill.active.level-ERROR {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
    border-color: var(--red);
  }
  .log-level-pill.active.level-WARN {
    background: rgba(234, 179, 8, 0.12);
    color: var(--yellow);
    border-color: var(--yellow);
  }
  .log-level-pill.active.level-INFO {
    background: rgba(59, 130, 246, 0.12);
    color: var(--blue);
    border-color: var(--blue);
  }
  .log-level-pill.active.level-DEBUG {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text-muted);
    border-color: var(--text-muted);
  }
  .log-level-pill.active.level-TRACE {
    background: rgba(255, 255, 255, 0.04);
    color: var(--text-dim);
    border-color: var(--text-dim);
  }
  .log-container {
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.6;
    max-height: 300px;
    overflow-y: auto;
    background: var(--bg-input);
    border-radius: var(--radius);
    padding: 8px 12px;
  }
  .log-line {
    white-space: pre-wrap;
    word-break: break-all;
    padding-left: 8px;
    border-left: 3px solid transparent;
  }
  .log-line.level-ERROR {
    background: rgba(239, 68, 68, 0.1);
    border-left: 3px solid var(--red);
  }
  .log-line.level-ERROR .log-level {
    color: var(--red);
    font-weight: bold;
  }
  .log-line.level-WARN {
    background: rgba(234, 179, 8, 0.08);
    border-left: 3px solid var(--yellow);
  }
  .log-line.level-WARN .log-level {
    color: var(--yellow);
  }
  .log-line.level-INFO {
    border-left: 3px solid var(--blue);
  }
  .log-line.level-INFO .log-level {
    color: var(--blue);
  }
  .log-line.level-DEBUG .log-level {
    color: var(--text-dim);
  }
  .log-line.level-TRACE .log-level {
    color: var(--text-dim);
    opacity: 0.6;
  }
  .log-line.level-TRACE {
    opacity: 0.7;
  }
  .log-target {
    color: var(--text-dim);
  }
  .log-message {
    color: var(--text-muted);
  }
</style>
