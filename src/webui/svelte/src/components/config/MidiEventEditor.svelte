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
<script lang="ts" module>
  import type { MidiEvent } from "../../gen/rules/mtrack_rules";

  /** The event a new MIDI binding starts at: middle C, full velocity, on
   *  channel 1. */
  export function newMidiEvent(): MidiEvent {
    return { type: "note_on", channel: 1, key: 60, velocity: 127 };
  }
</script>

<script lang="ts">
  import { t } from "svelte-i18n";
  import { rules } from "../../lib/rules";

  interface Props {
    event: MidiEvent;
    onchange: () => void;
    idPrefix?: string;
  }

  let { event = $bindable(), onchange, idPrefix = "midi" }: Props = $props();

  /** The MIDI events a config can name, and the range each field takes,
   *  as the player reads them: the editor offers every kind. */
  const midi = rules.midi_event_shapes();

  /** An event's fields other than its type, with their values. */
  function fieldsOf(event: MidiEvent): [string, number][] {
    return Object.entries(event).filter(([key]) => key !== "type");
  }

  /** The fields the event's kind has, each with the event's value (or the
   *  kind's lowest, when the event leaves it out). Keys the kind does not
   *  have -- a misspelling the player ignores -- are not drawn. */
  let fields = $derived.by((): [string, number][] => {
    const kind = midi.kinds.find((k) => k.type === event.type);
    if (!kind) return [];
    const values = new Map(fieldsOf(event));
    return fieldsOf(kind).map(([field, lowest]) => [
      field,
      values.get(field) ?? lowest,
    ]);
  });

  /** The event as another kind, with that kind's fields: the ones the two
   *  share keep their values, and keys neither kind has stay as written. */
  function setType(type: string) {
    const kind = midi.kinds.find((k) => k.type === type);
    if (!kind) return;
    const drawn = new Set(fields.map(([field]) => field));
    const next = { ...kind };
    for (const [field, value] of fieldsOf(event)) {
      if (field in kind || !drawn.has(field))
        Object.assign(next, { [field]: value });
    }
    event = next;
    onchange();
  }

  /** A field of the event, held to the range the player reads. */
  function setField(field: string, raw: string) {
    const [lowest, highest] = midi.ranges[field];
    const value = Math.round(Number(raw)) || lowest;
    Object.assign(event, {
      [field]: Math.min(highest, Math.max(lowest, value)),
    });
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
      {#each midi.kinds as kind (kind.type)}
        <option value={kind.type}>{$t(`midiEvent.types.${kind.type}`)}</option>
      {/each}
    </select>
  </div>

  {#each fields as [field, value] (field)}
    {@const [lowest, highest] = midi.ranges[field]}
    <div class="field field-num">
      <label for="{idPrefix}-{field}">{$t(`midiEvent.fields.${field}`)}</label>
      <input
        id="{idPrefix}-{field}"
        class="input"
        type="number"
        min={lowest}
        max={highest}
        {value}
        onchange={(e) => setField(field, (e.target as HTMLInputElement).value)}
      />
    </div>
  {/each}
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
