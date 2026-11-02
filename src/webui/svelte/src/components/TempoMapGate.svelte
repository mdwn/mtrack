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
  // Shows its contents once the player's tempo map (WebAssembly) has
  // loaded, and says why when it cannot: the views that lay bars and beats
  // out need it, the rest of the UI does not.
  import type { Snippet } from "svelte";
  import { t } from "svelte-i18n";
  import { loadTempoMap, tempoMapLoaded } from "../lib/tempo-wasm";

  let { children }: { children: Snippet } = $props();

  let attempt = $state(0);
  let ready = $derived.by(() => {
    void attempt;
    return loadTempoMap();
  });
</script>

{#if tempoMapLoaded()}
  {@render children()}
{:else}
  {#await ready}
    <p class="tempo-map-gate" data-testid="tempo-map-loading">
      {$t("tempoMap.loading")}
    </p>
  {:then}
    {@render children()}
  {:catch error}
    <div
      class="tempo-map-gate tempo-map-gate--error"
      role="alert"
      data-testid="tempo-map-unavailable"
    >
      <p>{$t("tempoMap.unavailable", { values: { error: error.message } })}</p>
      <button type="button" onclick={() => attempt++}>
        {$t("tempoMap.retry")}
      </button>
    </div>
  {/await}
{/if}

<style>
  .tempo-map-gate {
    padding: 16px;
    color: var(--text-muted);
  }
  .tempo-map-gate--error {
    color: var(--text);
    border: 1px solid var(--red);
    border-radius: var(--radius);
  }
</style>
