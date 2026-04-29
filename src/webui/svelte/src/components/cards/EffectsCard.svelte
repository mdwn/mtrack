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
  import { t } from "svelte-i18n";

  // Pick a badge kind based on the effect name's namespace.
  function badgeKindFor(effect: string): string {
    if (effect.includes("dmx")) return "dmx";
    if (effect.includes("midi")) return "midi";
    if (effect.includes("strobe") || effect.includes("flash")) return "trigger";
    return "ctrl";
  }
</script>

<section class="card effects-card">
  <header class="effects-card__head">
    <div>
      <div class="overline">{$t("effects.title")}</div>
      <div class="effects-card__title">
        {$effectsStore.length} active
      </div>
    </div>
  </header>
  <div class="effects-card__body">
    {#if $effectsStore.length === 0}
      <div class="effects-card__empty">{$t("effects.noEffects")}</div>
    {:else}
      <div class="effects-card__chips">
        {#each $effectsStore as effect, i (`${i}:${effect}`)}
          <span class="badge badge--pill badge--{badgeKindFor(effect)}"
            >{effect}</span
          >
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .effects-card {
    margin-top: 24px;
    padding: 0;
  }
  .effects-card__head {
    padding: 16px 20px;
    border-bottom: 1px solid var(--card-border);
  }
  .effects-card__title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 16px;
    margin-top: 4px;
    color: var(--nc-fg-1);
  }
  .effects-card__body {
    padding: 16px 20px;
  }
  .effects-card__empty {
    color: var(--nc-fg-3);
    font-style: italic;
    font-size: 13px;
  }
  .effects-card__chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
</style>
