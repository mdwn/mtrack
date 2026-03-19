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
  import type { Timestamp } from "../../lib/lighting/types";

  interface Props {
    value: Timestamp;
    onchange: (ts: Timestamp) => void;
  }

  let { value, onchange }: Props = $props();

  let mode = $derived(value.type);

  // Absolute fields derived from prop
  let mins = $derived(
    value.type === "absolute" ? Math.floor((value.ms ?? 0) / 60000) : 0,
  );
  let secs = $derived(
    value.type === "absolute"
      ? Math.floor(((value.ms ?? 0) % 60000) / 1000)
      : 0,
  );
  let msVal = $derived(value.type === "absolute" ? (value.ms ?? 0) % 1000 : 0);

  // Measure/beat fields derived from prop
  let measureVal = $derived(
    value.type === "measure_beat" ? (value.measure ?? 1) : 1,
  );
  let beatVal = $derived(value.type === "measure_beat" ? (value.beat ?? 1) : 1);

  function emitAbsolute(newMins: number, newSecs: number, newMs: number) {
    onchange({
      type: "absolute",
      ms: newMins * 60000 + newSecs * 1000 + newMs,
    });
  }

  function emitMeasureBeat(newMeasure: number, newBeat: number) {
    onchange({
      type: "measure_beat",
      measure: newMeasure,
      beat: newBeat,
    });
  }

  function switchMode(newMode: "absolute" | "measure_beat") {
    if (newMode === "absolute") {
      emitAbsolute(mins, secs, msVal);
    } else {
      emitMeasureBeat(measureVal, beatVal);
    }
  }
</script>

<div class="timestamp-input">
  <select
    class="mode-select"
    value={mode}
    onchange={(e) =>
      switchMode(
        (e.target as HTMLSelectElement).value as "absolute" | "measure_beat",
      )}
  >
    <option value="absolute">{$t("timestamp.time")}</option>
    <option value="measure_beat">{$t("timestamp.measure")}</option>
  </select>

  {#if mode === "absolute"}
    <div class="abs-fields">
      <input
        type="number"
        class="input ts-num"
        min="0"
        max="99"
        value={mins}
        onchange={(e) =>
          emitAbsolute(
            parseInt((e.target as HTMLInputElement).value) || 0,
            secs,
            msVal,
          )}
        title={$t("timestamp.minutes")}
      />
      <span class="ts-sep">:</span>
      <input
        type="number"
        class="input ts-num"
        min="0"
        max="59"
        value={secs}
        onchange={(e) =>
          emitAbsolute(
            mins,
            parseInt((e.target as HTMLInputElement).value) || 0,
            msVal,
          )}
        title={$t("timestamp.seconds")}
      />
      <span class="ts-sep">.</span>
      <input
        type="number"
        class="input ts-ms"
        min="0"
        max="999"
        value={msVal}
        onchange={(e) =>
          emitAbsolute(
            mins,
            secs,
            parseInt((e.target as HTMLInputElement).value) || 0,
          )}
        title={$t("timestamp.milliseconds")}
      />
    </div>
  {:else}
    <div class="mb-fields">
      <input
        type="number"
        class="input ts-num"
        min="1"
        value={measureVal}
        onchange={(e) =>
          emitMeasureBeat(
            parseInt((e.target as HTMLInputElement).value) || 1,
            beatVal,
          )}
        title={$t("timestamp.measure")}
      />
      <span class="ts-sep">/</span>
      <input
        type="number"
        class="input ts-beat"
        min="1"
        step="0.5"
        value={beatVal}
        onchange={(e) =>
          emitMeasureBeat(
            measureVal,
            parseFloat((e.target as HTMLInputElement).value) || 1,
          )}
        title={$t("timestamp.beat")}
      />
    </div>
  {/if}
</div>

<style>
  .timestamp-input {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .mode-select {
    background: var(--bg-input);
    border: 1px solid var(--border);
    color: var(--text);
    font-size: 12px;
    padding: 4px 6px;
    border-radius: var(--radius);
    width: 80px;
  }
  .abs-fields,
  .mb-fields {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .ts-num {
    width: 44px;
    text-align: center;
    padding: 4px !important;
    font-size: 13px !important;
  }
  .ts-ms {
    width: 50px;
    text-align: center;
    padding: 4px !important;
    font-size: 13px !important;
  }
  .ts-beat {
    width: 50px;
    text-align: center;
    padding: 4px !important;
    font-size: 13px !important;
  }
  .ts-sep {
    color: var(--text-muted);
    font-size: 14px;
    font-weight: 600;
  }
</style>
