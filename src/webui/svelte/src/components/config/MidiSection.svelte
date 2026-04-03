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
  import type { MidiDeviceInfo } from "../../lib/api/config";

  interface Props {
    midi: any;
    devices: MidiDeviceInfo[];
    onrefresh: () => void | Promise<void>;
    onchange: () => void;
  }

  let { midi = $bindable(), devices, onrefresh, onchange }: Props = $props();

  let refreshing = $state(false);

  async function handleRefresh() {
    refreshing = true;
    try {
      await onrefresh();
    } finally {
      refreshing = false;
    }
  }

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

  // MIDI-to-DMX helpers
  function getMappings(): any[] {
    return midi.midi_to_dmx ?? [];
  }

  function addMapping() {
    if (!midi.midi_to_dmx) midi.midi_to_dmx = [];
    midi.midi_to_dmx = [...midi.midi_to_dmx, { midi_channel: 1, universe: "" }];
    onchange();
  }

  function removeMapping(index: number) {
    const list = [...getMappings()];
    list.splice(index, 1);
    midi.midi_to_dmx = list.length > 0 ? list : undefined;
    onchange();
  }

  function updateMapping(index: number, key: string, value: any) {
    midi.midi_to_dmx[index][key] = value;
    onchange();
  }

  function addTransformer(mi: number) {
    if (!midi.midi_to_dmx[mi].transformers)
      midi.midi_to_dmx[mi].transformers = [];
    midi.midi_to_dmx[mi].transformers = [
      ...midi.midi_to_dmx[mi].transformers,
      { type: "note_mapper", input_note: 0, convert_to_notes: [] },
    ];
    onchange();
  }

  function removeTransformer(mi: number, ti: number) {
    const list = [...(midi.midi_to_dmx[mi].transformers ?? [])];
    list.splice(ti, 1);
    midi.midi_to_dmx[mi].transformers = list.length > 0 ? list : undefined;
    onchange();
  }

  function setTransformerType(mi: number, ti: number, type: string) {
    if (type === "note_mapper") {
      midi.midi_to_dmx[mi].transformers[ti] = {
        type: "note_mapper",
        input_note: 0,
        convert_to_notes: [],
      };
    } else {
      midi.midi_to_dmx[mi].transformers[ti] = {
        type: "control_change_mapper",
        input_controller: 0,
        convert_to_controllers: [],
      };
    }
    onchange();
  }

  function updateTransformerField(
    mi: number,
    ti: number,
    key: string,
    value: any,
  ) {
    midi.midi_to_dmx[mi].transformers[ti][key] = value;
    onchange();
  }

  function parseNumberList(str: string): number[] {
    return str
      .split(",")
      .map((s) => parseInt(s.trim()))
      .filter((n) => !isNaN(n));
  }

  function formatNumberList(nums: number[]): string {
    return nums.join(", ");
  }
</script>

<div class="section-fields">
  <div class="field">
    <label for="midi-device">{$t("midi.device")}</label>
    <div class="field-row">
      <input
        id="midi-device"
        class="input"
        list="midi-device-list"
        placeholder={$t("midi.devicePlaceholder")}
        value={midi.device || ""}
        onchange={(e) => set("device", (e.target as HTMLInputElement).value)}
      />
      <datalist id="midi-device-list">
        {#each outputDeviceNames as name (name)}
          <option value={name}></option>
        {/each}
      </datalist>
      <button class="btn" onclick={handleRefresh} disabled={refreshing}>{refreshing ? $t("common.refreshing") : $t("common.refresh")}</button>
    </div>
  </div>

  <div class="field">
    <label for="midi-delay"
      >{$t("midi.playbackDelay")}
      <Tooltip text={$t("tooltips.midi.playbackDelay")} /></label
    >
    <input
      id="midi-delay"
      type="text"
      class="input"
      placeholder={$t("midi.playbackDelayPlaceholder")}
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
      {$t("midi.enableBeatClock")}
      <Tooltip text={$t("tooltips.midi.beatClock")} />
    </label>
  </div>

  <div class="midi-to-dmx-section">
    <div class="field-header">
      <span class="field-label"
        >{$t("midi.midiToDmx")}
        <Tooltip text={$t("tooltips.midi.midiToDmx")} /></span
      >
      <button class="btn" onclick={addMapping}>{$t("common.add")}</button>
    </div>
    <p class="hint-text">{$t("midi.midiToDmxHint")}</p>

    {#each getMappings() as mapping, mi (mi)}
      <div class="mapping-card">
        <div class="mapping-header">
          <span class="mapping-label">{$t("midi.mapping")} {mi + 1}</span>
          <button
            class="btn btn-danger btn-sm"
            onclick={() => removeMapping(mi)}>{$t("common.remove")}</button
          >
        </div>

        <div class="mapping-fields">
          <div class="field field-narrow">
            <label for="mtd-ch-{mi}">{$t("midi.midiChannel")}</label>
            <input
              id="mtd-ch-{mi}"
              class="input"
              type="number"
              min="1"
              max="16"
              value={mapping.midi_channel ?? 1}
              onchange={(e) =>
                updateMapping(
                  mi,
                  "midi_channel",
                  parseInt((e.target as HTMLInputElement).value) || 1,
                )}
            />
          </div>
          <div class="field">
            <label for="mtd-univ-{mi}">{$t("midi.universe")}</label>
            <input
              id="mtd-univ-{mi}"
              class="input"
              type="text"
              placeholder={$t("midi.universePlaceholder")}
              value={mapping.universe ?? ""}
              onchange={(e) =>
                updateMapping(
                  mi,
                  "universe",
                  (e.target as HTMLInputElement).value.trim(),
                )}
            />
          </div>
        </div>

        <div class="transformers-area">
          <div class="field-header">
            <span class="transformer-label">{$t("midi.transformers")}</span>
            <button class="btn btn-sm" onclick={() => addTransformer(mi)}
              >{$t("common.add")}</button
            >
          </div>
          {#each mapping.transformers ?? [] as transformer, ti (ti)}
            <div class="transformer-row">
              <select
                class="input transformer-type"
                value={transformer.type}
                onchange={(e) =>
                  setTransformerType(
                    mi,
                    ti,
                    (e.target as HTMLSelectElement).value,
                  )}
              >
                <option value="note_mapper">{$t("midi.noteMapper")}</option>
                <option value="control_change_mapper"
                  >{$t("midi.ccMapper")}</option
                >
              </select>

              {#if transformer.type === "note_mapper"}
                <div class="field field-narrow">
                  <label for="mtd-{mi}-t{ti}-note">{$t("midi.inputNote")}</label
                  >
                  <input
                    id="mtd-{mi}-t{ti}-note"
                    class="input"
                    type="number"
                    min="0"
                    max="127"
                    value={transformer.input_note ?? 0}
                    onchange={(e) =>
                      updateTransformerField(
                        mi,
                        ti,
                        "input_note",
                        parseInt((e.target as HTMLInputElement).value) || 0,
                      )}
                  />
                </div>
                <div class="field">
                  <label for="mtd-{mi}-t{ti}-notes"
                    >{$t("midi.convertToNotes")}</label
                  >
                  <input
                    id="mtd-{mi}-t{ti}-notes"
                    class="input"
                    type="text"
                    placeholder="61, 62"
                    value={formatNumberList(transformer.convert_to_notes ?? [])}
                    onchange={(e) =>
                      updateTransformerField(
                        mi,
                        ti,
                        "convert_to_notes",
                        parseNumberList((e.target as HTMLInputElement).value),
                      )}
                  />
                </div>
              {:else}
                <div class="field field-narrow">
                  <label for="mtd-{mi}-t{ti}-cc"
                    >{$t("midi.inputController")}</label
                  >
                  <input
                    id="mtd-{mi}-t{ti}-cc"
                    class="input"
                    type="number"
                    min="0"
                    max="127"
                    value={transformer.input_controller ?? 0}
                    onchange={(e) =>
                      updateTransformerField(
                        mi,
                        ti,
                        "input_controller",
                        parseInt((e.target as HTMLInputElement).value) || 0,
                      )}
                  />
                </div>
                <div class="field">
                  <label for="mtd-{mi}-t{ti}-ccs"
                    >{$t("midi.convertToControllers")}</label
                  >
                  <input
                    id="mtd-{mi}-t{ti}-ccs"
                    class="input"
                    type="text"
                    placeholder="8, 9"
                    value={formatNumberList(
                      transformer.convert_to_controllers ?? [],
                    )}
                    onchange={(e) =>
                      updateTransformerField(
                        mi,
                        ti,
                        "convert_to_controllers",
                        parseNumberList((e.target as HTMLInputElement).value),
                      )}
                  />
                </div>
              {/if}

              <button
                class="btn btn-danger btn-sm"
                onclick={() => removeTransformer(mi, ti)}>&times;</button
              >
            </div>
          {/each}
        </div>
      </div>
    {/each}
  </div>
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
  .midi-to-dmx-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .field-label {
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .hint-text {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
  }
  .mapping-card {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mapping-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .mapping-label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .mapping-fields {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }
  .field-narrow {
    width: 80px;
    flex-shrink: 0;
  }
  .transformers-area {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border);
  }
  .transformer-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim);
  }
  .transformer-row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .transformer-type {
    width: 160px;
    flex-shrink: 0;
  }
</style>
