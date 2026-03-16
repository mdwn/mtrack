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
  import type {
    Cue,
    TempoSection,
    SubLaneType,
  } from "../../../lib/lighting/types";
  import type { OffsetMarker } from "../../../lib/lighting/timeline-state";
  import ShowLane from "./ShowLane.svelte";

  interface Props {
    name: string;
    cues: Cue[];
    pixelsPerMs: number;
    scrollLeft: number;
    viewportWidth: number;
    tempo?: TempoSection;
    selectedCueIndex: number | null;
    selectedSubLane: SubLaneType | null;
    snapEnabled: boolean;
    snapResolution: "beat" | "measure";
    laneType?: "show" | "sequence";
    offsets?: OffsetMarker[];
    onselect: (index: number, subLane: SubLaneType) => void;
    oncuechange: (index: number, cue: Cue) => void;
    oncuedelete: (index: number) => void;
    oncueadd: (cue: Cue) => void;
    ondelete: () => void;
  }

  let {
    name,
    cues,
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
    onselect,
    oncuechange,
    oncuedelete,
    oncueadd,
    ondelete,
  }: Props = $props();

  const subLanes: { type: SubLaneType; label: string; height: number }[] = [
    { type: "effects", label: "Effects", height: 32 },
    { type: "commands", label: "Cmds", height: 24 },
    { type: "sequences", label: "Seqs", height: 32 },
  ];
</script>

<div class="show-group">
  <div class="group-label">
    <div class="sublane-labels">
      {#each subLanes as sl, i (sl.type)}
        <div class="sublane-row" style:height="{sl.height}px">
          {#if i === 0}
            <span class="group-name" title={name}>{name}</span>
          {:else}
            <span class="sublane-type-label">{sl.label}</span>
          {/if}
        </div>
      {/each}
    </div>
    <button
      class="btn-icon group-delete"
      title="Delete show"
      onclick={ondelete}
    >
      &#10005;
    </button>
  </div>
  <div class="group-lanes">
    {#each subLanes as sl (sl.type)}
      <ShowLane
        {name}
        {cues}
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
        onselect={(ci) => onselect(ci, sl.type)}
        {oncuechange}
        {oncuedelete}
        {oncueadd}
        ondelete={() => {}}
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
    align-items: flex-start;
    padding: 0 8px;
    border-right: 1px solid var(--border);
    background: var(--bg-card);
    position: relative;
  }
  .group-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
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
    border-bottom: 1px solid var(--border);
    box-sizing: content-box;
  }
  .sublane-type-label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .group-delete {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 13px;
    padding: 1px 3px;
    border-radius: 3px;
    opacity: 0;
    transition: opacity 0.15s;
    position: absolute;
    top: 4px;
    right: 4px;
  }
  .show-group:hover .group-delete {
    opacity: 1;
  }
  .group-delete:hover {
    background: rgba(239, 68, 68, 0.15);
    color: var(--red);
  }
  .group-lanes {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
</style>
