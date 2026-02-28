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
    // Re-run whenever logStore changes
    void $logStore;
    if (autoScroll && container) {
      tick().then(() => {
        container!.scrollTop = container!.scrollHeight;
      });
    }
  });
</script>

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">Logs</span>
  </div>
  <div class="log-container" bind:this={container} onscroll={handleScroll}>
    {#each $logStore as line, i (i)}
      <div class="log-line">
        <span class="level-{line.level}">{line.level}</span>
        <span class="log-target">{line.target}</span>:
        <span class="log-message">{line.message}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .log-container {
    font-family: var(--mono);
    font-size: 11px;
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
  }
  .log-line :global(.level-ERROR) {
    color: var(--red);
  }
  .log-line :global(.level-WARN) {
    color: var(--yellow);
  }
  .log-line :global(.level-INFO) {
    color: var(--blue);
  }
  .log-line :global(.level-DEBUG) {
    color: var(--text-dim);
  }
  .log-target {
    color: var(--text-dim);
  }
  .log-message {
    color: var(--text-muted);
  }
</style>
