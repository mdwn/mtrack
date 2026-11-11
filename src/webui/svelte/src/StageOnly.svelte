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
  // The stage-only page (stage-main.ts): the live stage plot filling the
  // window, or a line saying there is nothing to draw. It reads the stores
  // and nothing else; whoever mounts it fills them.
  import StageView from "./components/StageView.svelte";
  import { metadataStore } from "./lib/ws/stores";
  import { t } from "svelte-i18n";

  interface Props {
    /** Offers the 3D view. It fetches the rig's models from
     *  /api/lighting/assets/, so it is for a host that answers those. */
    threeD?: boolean;
  }
  let { threeD = false }: Props = $props();

  // No layout yet, a layout with no fixtures, or a stream that ended.
  let empty = $derived(Object.keys($metadataStore).length === 0);
</script>

<main class="stage-only">
  {#if empty}
    <p class="stage-only__empty" data-testid="stage-empty">
      {$t("stage.noLights")}
    </p>
  {:else}
    <StageView embedded {threeD} />
  {/if}
</main>

<style>
  .stage-only {
    position: fixed;
    inset: 0;
    overflow: auto;
    background: var(--card-bg);
  }
  /* As the stage card says it has no venue. */
  .stage-only__empty {
    margin: 16px 20px;
    color: var(--nc-fg-3);
    font-size: 14px;
  }
</style>
