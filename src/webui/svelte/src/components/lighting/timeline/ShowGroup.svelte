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
  import type {
    Cue,
    Sequence,
    TempoSection,
    SubLaneType,
  } from "../../../lib/lighting/types";
  import { LAYERS } from "../../../lib/lighting/types";
  import type { OffsetMarker } from "../../../lib/lighting/timeline-state";
  import { expandSequencesIntoCues } from "../../../lib/lighting/timeline-state";
  import ShowLane from "./ShowLane.svelte";

  interface Props {
    name: string;
    cues: Cue[];
    sequences?: Sequence[];
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    tempo?: TempoSection;
    selectedCueIndex: number | null;
    selectedSubLane: SubLaneType | null;
    snapEnabled: boolean;
    snapResolution: import("../../../lib/lighting/timeline-state").SnapResolution;
    laneType?: "show" | "sequence";
    offsets?: OffsetMarker[];
    playheadMs?: number | null;
    onselect: (index: number, subLane: SubLaneType) => void;
    oncuechange: (index: number, cue: Cue) => void;
    oncuedelete: (index: number) => void;
    oncueadd: (cue: Cue) => void;
    ondelete: () => void;
    oneffectresize?: (cueIndex: number, newDurationStr: string) => void;
    onloopchange?: (cueIndex: number, newLoopCount: number) => void;
    onsequenceedit?: (sequenceName: string) => void;
  }

  let {
    name,
    cues,
    sequences = [],
    pixelsPerMs,
    scrollLeft,
    viewportWidth,
    tempo,
    selectedCueIndex,
    selectedSubLane,
    snapEnabled,
    snapResolution,
    laneType = "show",
    offsets = [],
    playheadMs = null,
    onselect,
    oncuechange,
    oncuedelete,
    oncueadd,
    ondelete,
    oneffectresize,
    onloopchange,
    onsequenceedit,
  }: Props = $props();

  // Expand sequence references into concrete cues for layer lane display
  let expandedCues = $derived(
    sequences.length > 0
      ? expandSequencesIntoCues(cues, sequences, tempo)
      : cues,
  );

  const LAYER_LABELS: Record<string, string> = {
    foreground: "FG",
    midground: "MG",
    background: "BG",
  };

  let subLanes = $derived([
    // Effect lanes derived from LAYERS (reversed so foreground is on top)
    ...[...LAYERS].reverse().map((layer) => ({
      type: `effects:${layer}` as SubLaneType,
      label: LAYER_LABELS[layer] ?? layer,
      height: 40,
    })),
    {
      type: "commands" as SubLaneType,
      label: $t("timeline.showGroup.cmds"),
      height: 32,
    },
    {
      type: "sequences" as SubLaneType,
      label: $t("timeline.showGroup.seqs"),
      height: 40,
    },
  ]);
</script>

<div class="show-group">
  <div class="group-label">
    <div class="group-header">
      <span class="group-name" title={name}>{name}</span>
      <button
        class="btn-icon group-delete"
        title={$t("timeline.showGroup.deleteShow")}
        onclick={ondelete}
      >
        &#10005;
      </button>
    </div>
    <div class="sublane-labels">
      {#each subLanes as sl (sl.type)}
        <div class="sublane-row" style:height="{sl.height}px">
          <span class="sublane-type-label">{sl.label}</span>
        </div>
      {/each}
    </div>
  </div>
  <div class="group-lanes">
    <div class="lane-header-spacer"></div>
    {#each subLanes as sl (sl.type)}
      <ShowLane
        {name}
        cues={sl.type.startsWith("effects") ? expandedCues : cues}
        {laneType}
        {pixelsPerMs}
        {scrollLeft}
        {viewportWidth}
        {tempo}
        selectedCueIndex={selectedSubLane === sl.type ? selectedCueIndex : null}
        {snapEnabled}
        {snapResolution}
        subLaneType={sl.type}
        laneHeight={sl.height}
        hideLabel={true}
        {offsets}
        {playheadMs}
        onselect={(ci) => onselect(ci, sl.type)}
        {oncuechange}
        {oncuedelete}
        {oncueadd}
        ondelete={() => {}}
        oneffectresize={sl.type.startsWith("effects")
          ? oneffectresize
          : undefined}
        onloopchange={sl.type === "sequences" ? onloopchange : undefined}
        onsequenceedit={sl.type === "sequences" ? onsequenceedit : undefined}
        sequenceDefs={sl.type === "sequences" ? sequences : []}
      />
    {/each}
  </div>
</div>

<style>
  .show-group {
    display: flex;
    border-bottom: 1px solid var(--border);
  }
  .group-label {
    width: 80px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--bg-card);
  }
  .group-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 8px;
    border-bottom: 1px solid var(--border);
    min-height: 22px;
  }
  .group-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .sublane-labels {
    display: flex;
    flex-direction: column;
    width: 100%;
  }
  .sublane-row {
    display: flex;
    align-items: center;
    min-width: 0;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    box-sizing: content-box;
  }
  .sublane-type-label {
    font-size: 11px;
    color: var(--text-dim);
  }
  .group-delete {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 11px;
    padding: 1px 3px;
    border-radius: 3px;
    opacity: 0;
    transition: opacity 0.15s;
    flex-shrink: 0;
  }
  .show-group:hover .group-delete {
    opacity: 1;
  }
  .group-delete:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .lane-header-spacer {
    height: 22px;
    min-height: 22px;
    border-bottom: 1px solid var(--border);
  }
  .group-lanes {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
</style>
