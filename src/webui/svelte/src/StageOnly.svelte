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
  import { untrack } from "svelte";

  type View = "plot" | "3d";

  interface Props {
    /** Offers the 3D view. It fetches the rig's models from the asset
     *  store, so it is for a host that answers those. */
    threeD?: boolean;
    /** The view to start in. */
    view?: View;
    /** Told when the person switches between the plot and 3D. */
    onViewChange?: (view: View) => void;
  }
  let { threeD = false, view = "plot", onViewChange }: Props = $props();

  // No layout yet, a layout with no fixtures, or a stream that ended.
  let empty = $derived(Object.keys($metadataStore).length === 0);

  /** The view the person chose last; a card built again starts in it. */
  let current = $state<View>(untrack(() => view));
  /** Moves to build the card again. */
  let builds = $state(0);

  function changed(next: View) {
    current = next;
    onViewChange?.(next);
  }

  /** Builds the 3D view again from nothing, so it asks again for every
   *  file it could not have. Nothing happens in the plot. */
  export function retry3d(): void {
    if (current === "3d") builds++;
  }
</script>

<main class="stage-only">
  {#if empty}
    <p class="stage-only__empty" data-testid="stage-empty">
      {$t("stage.noLights")}
    </p>
  {:else}
    {#key builds}
      <StageView embedded {threeD} view={current} onViewChange={changed} />
    {/key}
  {/if}
</main>

<style>
  .stage-only {
    position: fixed;
    inset: 0;
    overflow: auto;
    background: var(--card-bg);
    /* A view, not a document: a long press selects nothing and calls up no
       menu, and a tap does not flash. */
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
    -webkit-tap-highlight-color: transparent;
  }
  /* As the stage card says it has no venue. */
  .stage-only__empty {
    margin: 16px 20px;
    color: var(--nc-fg-3);
    font-size: 14px;
  }
</style>
