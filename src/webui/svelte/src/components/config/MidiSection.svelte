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
    BeatClockSeams,
    Midi,
    MidiDeviceInfo,
  } from "../../gen/rules/mtrack_rules";
  import { t } from "svelte-i18n";
  import Tooltip from "./Tooltip.svelte";
  import { rules } from "../../lib/rules";

  interface Props {
    midi: Midi;
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

  function set<K extends keyof Midi>(key: K, value: Midi[K]) {
    midi[key] = value;
    onchange();
  }

  function setOrDelete<K extends keyof Midi>(key: K, value: Midi[K] | "") {
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
      <button class="btn" onclick={handleRefresh} disabled={refreshing}
        >{refreshing ? $t("common.refreshing") : $t("common.refresh")}</button
      >
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

  <div class="field">
    <label for="midi-persist-tempo">
      <input
        id="midi-persist-tempo"
        type="checkbox"
        checked={midi.persist_tempo ?? false}
        disabled={!(midi.beat_clock ?? false)}
        onchange={(e) => {
          const checked = (e.target as HTMLInputElement).checked;
          setOrDelete("persist_tempo", checked || undefined);
        }}
      />
      {$t("midi.persistTempo")}
      <Tooltip text={$t("tooltips.midi.persistTempo")} />
    </label>
  </div>

  <div class="field">
    <label for="midi-beat-clock-seams"
      >{$t("midi.beatClockSeams")}
      <Tooltip text={$t("tooltips.midi.beatClockSeams")} /></label
    >
    <select
      id="midi-beat-clock-seams"
      class="input"
      value={midi.beat_clock_seams ?? "run_through"}
      disabled={!(midi.beat_clock ?? false)}
      onchange={(e) => {
        const value = (e.target as HTMLSelectElement).value as BeatClockSeams;
        setOrDelete(
          "beat_clock_seams",
          value === "run_through" ? undefined : value,
        );
      }}
    >
      {#each rules.config_choices().beat_clock_seams as s (s)}
        <option value={s}>{$t(`midi.seams.${s}`)}</option>
      {/each}
    </select>
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
</style>
