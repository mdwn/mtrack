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
  /* eslint-disable @typescript-eslint/no-explicit-any */
  import { t } from "svelte-i18n";
  import Tooltip from "./Tooltip.svelte";
  import MidiEventEditor from "./MidiEventEditor.svelte";
  import type { MidiEvent } from "./MidiEventEditor.svelte";

  interface Props {
    statusEvents: Record<string, any>;
    onchange: () => void;
  }

  let { statusEvents = $bindable(), onchange }: Props = $props();

  const eventGroups: [string, string, string][] = [
    ["off_events", "statusEvents.offEvents", "tooltips.statusEvents.offEvents"],
    [
      "idling_events",
      "statusEvents.idlingEvents",
      "tooltips.statusEvents.idlingEvents",
    ],
    [
      "playing_events",
      "statusEvents.playingEvents",
      "tooltips.statusEvents.playingEvents",
    ],
  ];

  function defaultEvent(): MidiEvent {
    return { type: "note_on", channel: 1, key: 60, velocity: 127 };
  }

  function getEvents(key: string): MidiEvent[] {
    return (statusEvents[key] as MidiEvent[]) ?? [];
  }

  function addEvent(key: string) {
    if (!statusEvents[key]) {
      statusEvents[key] = [];
    }
    statusEvents[key] = [...statusEvents[key], defaultEvent()];
    onchange();
  }

  function removeEvent(key: string, index: number) {
    const list = [...getEvents(key)];
    list.splice(index, 1);
    statusEvents[key] = list;
    onchange();
  }

  function onEventChange() {
    onchange();
  }
</script>

<div class="section-fields">
  <p class="muted hint-text">{$t("statusEvents.hint")}</p>

  {#each eventGroups as [key, labelKey, tooltipKey] (key)}
    <div class="event-group">
      <div class="group-header">
        <span class="group-label"
          >{$t(labelKey)} <Tooltip text={$t(tooltipKey)} /></span
        >
        <button class="btn btn-sm" onclick={() => addEvent(key)}
          >{$t("common.add")}</button
        >
      </div>
      {#if getEvents(key).length === 0}
        <p class="muted empty-text">{$t("statusEvents.noEvents")}</p>
      {/if}
      <!-- eslint-disable-next-line @typescript-eslint/no-unused-vars -->
      {#each getEvents(key) as _evt, i (i)}
        <div class="event-row">
          <MidiEventEditor
            bind:event={statusEvents[key][i]}
            onchange={onEventChange}
            idPrefix="status-{key}-{i}"
          />
          <button
            class="btn btn-danger btn-sm remove-btn"
            onclick={() => removeEvent(key, i)}>&times;</button
          >
        </div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .hint-text {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
  }
  .event-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .group-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .group-label {
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .empty-text {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
    font-style: italic;
  }
  .event-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  .event-row :global(.midi-event-row) {
    flex: 1;
  }
  .remove-btn {
    flex: 0 0 auto;
    padding: 4px 8px;
    font-size: 16px;
    line-height: 1;
  }
</style>
