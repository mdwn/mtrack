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
  import { effectsStore } from "../../lib/ws/stores";
</script>

<div class="card card-full">
  <div class="card-header">
    <span class="card-title">Active Effects</span>
  </div>
  {#if $effectsStore.length === 0}
    <div class="effects-empty">No active effects</div>
  {:else}
    <ul class="effects-list">
      {#each $effectsStore as effect (effect)}
        <li>{effect}</li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .effects-list {
    list-style: none;
  }
  .effects-list li {
    padding: 4px 0;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .effects-empty {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
  }
</style>
