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
<!--
  Play up to the spot, pause, take it. The row reads the playhead out in
  both units, because which one a dialog writes depends on what it stores.
-->
<script lang="ts">
  import { t } from "svelte-i18n";
  import type { Snippet } from "svelte";
  import { formatSeconds } from "../../lib/util/beatGrid";

  interface Props {
    /** Playhead position in seconds. */
    time: number;
    /** Its musical position, e.g. "m14 · b2½". */
    position?: string;
    /** The capture buttons — one per boundary the dialog can take. */
    actions: Snippet;
  }

  let { time, position, actions }: Props = $props();
</script>

<div class="playhead-capture">
  <span class="playhead-at mono">
    {$t("sections.dialog.playheadAt", {
      values: { pos: formatSeconds(time) },
    })}{#if position}<span class="playhead-pos"> · {position}</span>{/if}
  </span>
  {@render actions()}
</div>

<style>
  .playhead-capture {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 10px;
  }
  .playhead-at {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .playhead-pos {
    color: var(--text-muted);
  }
</style>
