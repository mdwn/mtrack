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

  export interface MidiEvent {
    type: string;
    channel?: number;
    key?: number;
    velocity?: number;
    program?: number;
    controller?: number;
    value?: number;
    bend?: number;
  }

  interface Props {
    event: MidiEvent;
    onchange: () => void;
    idPrefix?: string;
  }

  let { event = $bindable(), onchange, idPrefix = "midi" }: Props = $props();

  function setType(type: string) {
    if (type === "note_on" || type === "note_off") {
      event = { type, channel: event.channel ?? 1, key: 60, velocity: 100 };
    } else if (type === "aftertouch") {
      event = { type, channel: event.channel ?? 1, key: 60, velocity: 100 };
    } else if (type === "control_change") {
      event = { type, channel: event.channel ?? 1, controller: 0, value: 0 };
    } else if (type === "program_change") {
      event = { type, channel: event.channel ?? 1, program: 0 };
    } else if (type === "channel_aftertouch") {
      event = { type, channel: event.channel ?? 1, velocity: 64 };
    } else if (type === "pitch_bend") {
      event = { type, channel: event.channel ?? 1, bend: 8192 };
    }
    onchange();
  }

  function updateNum(field: keyof MidiEvent, value: string, fallback: number) {
    (event as unknown as Record<string, unknown>)[field] =
      parseInt(value) || fallback;
    onchange();
  }
</script>

<div class="midi-event-row">
  <div class="field field-type">
    <label for="{idPrefix}-type">{$t("midiEvent.type")}</label>
    <select
      id="{idPrefix}-type"
      class="input"
      value={event.type}
      onchange={(e) => setType((e.target as HTMLSelectElement).value)}
    >
      <option value="note_on">{$t("midiEvent.noteOn")}</option>
      <option value="note_off">{$t("midiEvent.noteOff")}</option>
      <option value="aftertouch">{$t("midiEvent.aftertouch")}</option>
      <option value="control_change">{$t("midiEvent.controlChange")}</option>
      <option value="program_change">{$t("midiEvent.programChange")}</option>
      <option value="channel_aftertouch"
        >{$t("midiEvent.channelAftertouch")}</option
      >
      <option value="pitch_bend">{$t("midiEvent.pitchBend")}</option>
    </select>
  </div>

  <div class="field field-num">
    <label for="{idPrefix}-channel">{$t("midiEvent.channel")}</label>
    <input
      id="{idPrefix}-channel"
      class="input"
      type="number"
      min="1"
      max="16"
      value={event.channel ?? 1}
      onchange={(e) =>
        updateNum("channel", (e.target as HTMLInputElement).value, 1)}
    />
  </div>

  {#if event.type === "note_on" || event.type === "note_off" || event.type === "aftertouch"}
    <div class="field field-num">
      <label for="{idPrefix}-key">{$t("midiEvent.key")}</label>
      <input
        id="{idPrefix}-key"
        class="input"
        type="number"
        min="0"
        max="127"
        value={event.key ?? 60}
        onchange={(e) =>
          updateNum("key", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
    <div class="field field-num">
      <label for="{idPrefix}-velocity">{$t("midiEvent.velocity")}</label>
      <input
        id="{idPrefix}-velocity"
        class="input"
        type="number"
        min="0"
        max="127"
        value={event.velocity ?? 0}
        onchange={(e) =>
          updateNum("velocity", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
  {:else if event.type === "control_change"}
    <div class="field field-num">
      <label for="{idPrefix}-controller">{$t("midiEvent.controller")}</label>
      <input
        id="{idPrefix}-controller"
        class="input"
        type="number"
        min="0"
        max="127"
        value={event.controller ?? 0}
        onchange={(e) =>
          updateNum("controller", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
    <div class="field field-num">
      <label for="{idPrefix}-value">{$t("midiEvent.value")}</label>
      <input
        id="{idPrefix}-value"
        class="input"
        type="number"
        min="0"
        max="127"
        value={event.value ?? 0}
        onchange={(e) =>
          updateNum("value", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
  {:else if event.type === "program_change"}
    <div class="field field-num">
      <label for="{idPrefix}-program">{$t("midiEvent.program")}</label>
      <input
        id="{idPrefix}-program"
        class="input"
        type="number"
        min="0"
        max="127"
        value={event.program ?? 0}
        onchange={(e) =>
          updateNum("program", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
  {:else if event.type === "channel_aftertouch"}
    <div class="field field-num">
      <label for="{idPrefix}-velocity">{$t("midiEvent.velocity")}</label>
      <input
        id="{idPrefix}-velocity"
        class="input"
        type="number"
        min="0"
        max="127"
        value={event.velocity ?? 0}
        onchange={(e) =>
          updateNum("velocity", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
  {:else if event.type === "pitch_bend"}
    <div class="field field-num">
      <label for="{idPrefix}-bend">{$t("midiEvent.bend")}</label>
      <input
        id="{idPrefix}-bend"
        class="input"
        type="number"
        min="0"
        max="16383"
        value={event.bend ?? 8192}
        onchange={(e) =>
          updateNum("bend", (e.target as HTMLInputElement).value, 0)}
      />
    </div>
  {/if}
</div>

<style>
  .midi-event-row {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field-type {
    min-width: 160px;
  }
  .field-num {
    width: 80px;
  }
  .field label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  @media (max-width: 480px) {
    .midi-event-row {
      flex-wrap: wrap;
    }
    .field-type {
      width: 100%;
    }
  }
</style>
