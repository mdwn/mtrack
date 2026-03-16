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
  import type { MidiDeviceInfo } from "../../lib/api/config";

  interface Props {
    midi: any;
    devices: MidiDeviceInfo[];
    onrefresh: () => void;
    onchange: () => void;
  }

  let { midi = $bindable(), devices, onrefresh, onchange }: Props = $props();

  let outputDeviceNames = $derived(
    devices.filter((d) => d.has_output).map((d) => d.name),
  );

  function set(key: string, value: any) {
    midi[key] = value;
    onchange();
  }

  function setOrDelete(key: string, value: any) {
    if (value === undefined || value === "") {
      delete midi[key];
    } else {
      midi[key] = value;
    }
    onchange();
  }
</script>

<div class="section-fields">
  <div class="field">
    <label for="midi-device">Device</label>
    <div class="field-row">
      <input
        id="midi-device"
        class="input"
        list="midi-device-list"
        placeholder="Type or select a device"
        value={midi.device || ""}
        onchange={(e) => set("device", (e.target as HTMLInputElement).value)}
      />
      <datalist id="midi-device-list">
        {#each outputDeviceNames as name (name)}
          <option value={name}></option>
        {/each}
      </datalist>
      <button class="btn" onclick={onrefresh}>Refresh</button>
    </div>
  </div>

  <div class="field">
    <label for="midi-delay">Playback Delay</label>
    <input
      id="midi-delay"
      type="text"
      class="input"
      placeholder="e.g. 500ms"
      value={midi.playback_delay ?? ""}
      onchange={(e) =>
        setOrDelete(
          "playback_delay",
          (e.target as HTMLInputElement).value.trim() || undefined,
        )}
    />
  </div>

  <div class="field">
    <label for="midi-beat-clock">
      <input
        id="midi-beat-clock"
        type="checkbox"
        checked={midi.beat_clock ?? false}
        onchange={(e) => {
          const checked = (e.target as HTMLInputElement).checked;
          setOrDelete("beat_clock", checked || undefined);
        }}
      />
      Enable Beat Clock
    </label>
  </div>

  {#if midi.midi_to_dmx}
    <div class="note">
      MIDI-to-DMX mappings configured. Edit in raw YAML for advanced
      configuration.
    </div>
  {/if}
</div>

<style>
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field label:has(input[type="checkbox"]) {
    display: flex;
    align-items: center;
    gap: 8px;
    text-transform: none;
    font-size: 14px;
    cursor: pointer;
  }
  .field-row {
    display: flex;
    gap: 8px;
  }
  .field-row .input {
    flex: 1;
  }
  .note {
    font-size: 13px;
    color: var(--text-dim);
    font-style: italic;
  }
</style>
