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
  import type {
    AudioDeviceInfo,
    ChannelCalibration,
    NoiseFloorResult,
  } from "../../lib/api/config";
  import {
    startCalibration,
    startCapture,
    stopCapture,
    cancelCalibration,
  } from "../../lib/api/config";

  interface Props {
    trigger: any;
    audioDevices: AudioDeviceInfo[];
    onrefresh: () => void;
    onchange: () => void;
  }

  let {
    trigger = $bindable(),
    audioDevices,
    onrefresh,
    onchange,
  }: Props = $props();

  // Only show input devices (those with input channels).
  let inputDevices = $derived(audioDevices.filter((d) => d.max_channels > 0));
  let inputDeviceNames = $derived(inputDevices.map((d) => d.name));

  let selectedDevice = $derived(
    inputDevices.find((d) => d.name === trigger.device),
  );
  let sampleRateOptions = $derived(
    selectedDevice?.supported_sample_rates ?? [],
  );

  function setOrDelete(key: string, value: any) {
    if (value === undefined || value === "") {
      delete trigger[key];
    } else {
      trigger[key] = value;
    }
    onchange();
  }

  // Input list helpers
  let inputs: any[] = $derived(trigger.inputs || []);

  function addAudioInput() {
    if (!trigger.inputs) trigger.inputs = [];
    trigger.inputs.push({
      kind: "audio",
      channel: trigger.inputs.filter((i: any) => i.kind === "audio").length + 1,
      sample: "",
      threshold: 0.1,
    });
    onchange();
  }

  function addMidiInput() {
    if (!trigger.inputs) trigger.inputs = [];
    trigger.inputs.push({
      kind: "midi",
      event: { type: "note_on", channel: 10, key: 60 },
      sample: "",
    });
    onchange();
  }

  function removeInput(i: number) {
    trigger.inputs.splice(i, 1);
    if (trigger.inputs.length === 0) delete trigger.inputs;
    onchange();
  }

  function updateInput(i: number, key: string, value: any) {
    trigger.inputs[i][key] = value;
    onchange();
  }

  function setOrDeleteInput(i: number, key: string, value: any) {
    if (value === undefined || value === "") {
      delete trigger.inputs[i][key];
    } else {
      trigger.inputs[i][key] = value;
    }
    onchange();
  }

  function updateMidiEvent(i: number, key: string, value: any) {
    if (!trigger.inputs[i].event) trigger.inputs[i].event = {};
    trigger.inputs[i].event[key] = value;
    onchange();
  }

  // Expanded input index for showing advanced settings
  let expandedInput: number | null = $state(null);

  function toggleExpanded(i: number) {
    expandedInput = expandedInput === i ? null : i;
  }

  // --- Calibration wizard state ---
  type CalStep = "setup" | "noise" | "capture" | "results";
  let calInputIndex: number | null = $state(null);
  let calStep: CalStep = $state("setup");
  let calDevice: string = $state("");
  let calChannel: number = $state(1);
  let calDuration: number = $state(3);
  let calError: string = $state("");
  let calNoiseFloor: NoiseFloorResult | null = $state(null);
  let calResult: ChannelCalibration | null = $state(null);
  let calCountdown: number = $state(0);
  let calCountdownTimer: ReturnType<typeof setInterval> | null = $state(null);

  function openCalibrate(i: number) {
    calInputIndex = i;
    calStep = "setup";
    calError = "";
    calNoiseFloor = null;
    calResult = null;
    // Pre-fill from trigger config and input
    calDevice = trigger.device || "";
    calChannel = trigger.inputs[i].channel || 1;
    calDuration = 3;
  }

  function closeCalibrate() {
    if (calCountdownTimer) clearInterval(calCountdownTimer);
    cancelCalibration().catch(() => {});
    calInputIndex = null;
    calCountdownTimer = null;
  }

  async function doNoiseFloor() {
    calStep = "noise";
    calError = "";
    calCountdown = calDuration;

    // Start countdown timer
    calCountdownTimer = setInterval(() => {
      calCountdown = Math.max(0, calCountdown - 1);
    }, 1000);

    try {
      calNoiseFloor = await startCalibration(
        calDevice,
        calChannel,
        calDuration,
      );
      calStep = "capture";
      // Auto-start capture
      await startCapture();
    } catch (e: any) {
      calError = e.message || "Calibration failed";
      calStep = "setup";
    } finally {
      if (calCountdownTimer) clearInterval(calCountdownTimer);
      calCountdownTimer = null;
    }
  }

  async function doStopCapture() {
    calError = "";
    try {
      calResult = await stopCapture();
      calStep = "results";
    } catch (e: any) {
      calError = e.message || "Stop capture failed";
    }
  }

  function applyCalibration() {
    if (calResult === null || calInputIndex === null) return;
    const input = trigger.inputs[calInputIndex];
    input.threshold = parseFloat(calResult.threshold.toFixed(4));
    input.gain = parseFloat(calResult.gain.toFixed(2));
    if (calResult.scan_time_ms !== 5) {
      input.scan_time_ms = calResult.scan_time_ms;
    }
    if (calResult.retrigger_time_ms !== 30) {
      input.retrigger_time_ms = calResult.retrigger_time_ms;
    }
    if (calResult.highpass_freq != null) {
      input.highpass_freq = calResult.highpass_freq;
    } else {
      delete input.highpass_freq;
    }
    if (calResult.dynamic_threshold_decay_ms != null) {
      input.dynamic_threshold_decay_ms = calResult.dynamic_threshold_decay_ms;
    } else {
      delete input.dynamic_threshold_decay_ms;
    }
    onchange();
    calInputIndex = null;
  }
</script>

<div class="section-fields">
  <div class="field">
    <label for="trigger-device">Audio Input Device</label>
    <div class="field-row">
      <input
        id="trigger-device"
        class="input"
        list="trigger-device-list"
        placeholder="Only needed for audio triggers"
        value={trigger.device || ""}
        onchange={(e) =>
          setOrDelete(
            "device",
            (e.target as HTMLInputElement).value.trim() || undefined,
          )}
      />
      <datalist id="trigger-device-list">
        {#each inputDeviceNames as name (name)}
          <option value={name}></option>
        {/each}
      </datalist>
      <button class="btn" onclick={onrefresh}>Refresh</button>
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="trigger-sample-rate">Sample Rate</label>
      <select
        id="trigger-sample-rate"
        class="input"
        value={String(trigger.sample_rate ?? "")}
        onchange={(e) => {
          const v = (e.target as HTMLSelectElement).value;
          setOrDelete("sample_rate", v ? parseInt(v) : undefined);
        }}
      >
        <option value="">Default</option>
        {#if sampleRateOptions.length > 0}
          {#each sampleRateOptions as rate (rate)}
            <option value={String(rate)}>{rate}</option>
          {/each}
        {:else}
          <option value="44100">44100</option>
          <option value="48000">48000</option>
          <option value="96000">96000</option>
        {/if}
        {#if trigger.sample_rate && !sampleRateOptions.includes(trigger.sample_rate) && sampleRateOptions.length > 0}
          <option value={String(trigger.sample_rate)}
            >{trigger.sample_rate}</option
          >
        {/if}
      </select>
    </div>

    <div class="field">
      <label for="trigger-buffer-size">Buffer Size</label>
      <input
        id="trigger-buffer-size"
        type="number"
        class="input"
        placeholder="Device default"
        value={trigger.buffer_size ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("buffer_size", v ? parseInt(v) : undefined);
        }}
      />
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="trigger-crosstalk-window">Crosstalk Window (ms)</label>
      <input
        id="trigger-crosstalk-window"
        type="number"
        class="input"
        placeholder="Disabled"
        value={trigger.crosstalk_window_ms ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("crosstalk_window_ms", v ? parseInt(v) : undefined);
        }}
      />
    </div>

    <div class="field">
      <label for="trigger-crosstalk-threshold">Crosstalk Threshold</label>
      <input
        id="trigger-crosstalk-threshold"
        type="number"
        step="0.1"
        class="input"
        placeholder="Disabled"
        value={trigger.crosstalk_threshold ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("crosstalk_threshold", v ? parseFloat(v) : undefined);
        }}
      />
    </div>
  </div>

  <div class="field">
    <div class="field-header">
      <span class="field-label">Inputs</span>
      <div class="btn-group">
        <button class="btn" onclick={addAudioInput}>+ Audio</button>
        <button class="btn" onclick={addMidiInput}>+ MIDI</button>
      </div>
    </div>

    {#each inputs as input, i (i)}
      <div class="input-card" class:midi={input.kind === "midi"}>
        <div class="input-header">
          <span class="input-kind"
            >{input.kind === "audio" ? "Audio" : "MIDI"}</span
          >
          <div class="input-header-controls">
            {#if input.kind === "audio"}
              <button
                class="btn btn-small btn-accent"
                onclick={() => openCalibrate(i)}>Calibrate</button
              >
              <button class="btn btn-small" onclick={() => toggleExpanded(i)}
                >{expandedInput === i ? "Less" : "More"}</button
              >
            {/if}
            <button
              class="btn btn-danger btn-small"
              onclick={() => removeInput(i)}>X</button
            >
          </div>
        </div>

        {#if input.kind === "audio"}
          <!-- Calibration wizard (inline) -->
          {#if calInputIndex === i}
            <div class="cal-wizard">
              {#if calStep === "setup"}
                <div class="cal-step">
                  <div class="cal-title">Calibrate Channel {calChannel}</div>
                  <div class="field-row-3">
                    <div class="field">
                      <label for="cal-device">Device</label>
                      <input
                        id="cal-device"
                        class="input"
                        list="trigger-device-list"
                        bind:value={calDevice}
                      />
                    </div>
                    <div class="field">
                      <label for="cal-channel">Channel</label>
                      <input
                        id="cal-channel"
                        type="number"
                        min="1"
                        class="input"
                        bind:value={calChannel}
                      />
                    </div>
                    <div class="field">
                      <label for="cal-duration">Duration (s)</label>
                      <input
                        id="cal-duration"
                        type="number"
                        min="1"
                        max="30"
                        class="input"
                        bind:value={calDuration}
                      />
                    </div>
                  </div>
                  {#if calError}
                    <div class="cal-error">{calError}</div>
                  {/if}
                  <div class="cal-actions">
                    <button
                      class="btn btn-accent"
                      onclick={doNoiseFloor}
                      disabled={!calDevice}>Start</button
                    >
                    <button class="btn" onclick={closeCalibrate}>Cancel</button>
                  </div>
                </div>
              {:else if calStep === "noise"}
                <div class="cal-step">
                  <div class="cal-title">Measuring noise floor...</div>
                  <div class="cal-status">
                    Keep all pads silent. {calCountdown}s remaining
                  </div>
                  <div class="cal-progress">
                    <div
                      class="cal-progress-bar"
                      style="width: {Math.max(
                        0,
                        (1 - calCountdown / calDuration) * 100,
                      )}%"
                    ></div>
                  </div>
                </div>
              {:else if calStep === "capture"}
                <div class="cal-step">
                  <div class="cal-title">Hit your trigger now!</div>
                  {#if calNoiseFloor}
                    <div class="cal-stats">
                      Noise floor: peak {calNoiseFloor.peak.toFixed(6)}, RMS {calNoiseFloor.rms.toFixed(
                        6,
                      )}
                    </div>
                  {/if}
                  {#if calError}
                    <div class="cal-error">{calError}</div>
                  {/if}
                  <div class="cal-actions">
                    <button class="btn btn-accent" onclick={doStopCapture}
                      >Stop</button
                    >
                    <button class="btn" onclick={closeCalibrate}>Cancel</button>
                  </div>
                </div>
              {:else if calStep === "results"}
                <div class="cal-step">
                  {#if calResult}
                    <div class="cal-title">Calibration Results</div>
                    <div class="cal-results-grid">
                      <span class="cal-label">Hits detected:</span>
                      <span>{calResult.num_hits_detected}</span>
                      <span class="cal-label">Threshold:</span>
                      <span>{calResult.threshold.toFixed(4)}</span>
                      <span class="cal-label">Gain:</span>
                      <span>{calResult.gain.toFixed(2)}</span>
                      <span class="cal-label">Scan time:</span>
                      <span>{calResult.scan_time_ms} ms</span>
                      <span class="cal-label">Retrigger time:</span>
                      <span>{calResult.retrigger_time_ms} ms</span>
                      {#if calResult.highpass_freq != null}
                        <span class="cal-label">Highpass:</span>
                        <span>{calResult.highpass_freq} Hz</span>
                      {/if}
                      {#if calResult.dynamic_threshold_decay_ms != null}
                        <span class="cal-label">Dyn. threshold:</span>
                        <span>{calResult.dynamic_threshold_decay_ms} ms</span>
                      {/if}
                      <span class="cal-label">Max hit amplitude:</span>
                      <span>{calResult.max_hit_amplitude.toFixed(4)}</span>
                    </div>
                    {#if calResult.num_hits_detected === 0}
                      <div class="cal-error">
                        No hits detected. Try again with stronger hits or lower
                        noise.
                      </div>
                    {/if}
                    <div class="cal-actions">
                      <button
                        class="btn btn-accent"
                        onclick={applyCalibration}
                        disabled={calResult.num_hits_detected === 0}
                        >Apply</button
                      >
                      <button class="btn" onclick={closeCalibrate}
                        >Discard</button
                      >
                    </div>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}

          <div class="input-fields">
            <div class="field-row-3">
              <div class="field">
                <label for="trigger-ch-{i}">Channel</label>
                <input
                  id="trigger-ch-{i}"
                  type="number"
                  min="1"
                  class="input"
                  value={input.channel}
                  onchange={(e) =>
                    updateInput(
                      i,
                      "channel",
                      parseInt((e.target as HTMLInputElement).value) || 1,
                    )}
                />
              </div>
              <div class="field">
                <label for="trigger-sample-{i}">Sample</label>
                <input
                  id="trigger-sample-{i}"
                  class="input"
                  placeholder="Sample name"
                  value={input.sample ?? ""}
                  onchange={(e) =>
                    setOrDeleteInput(
                      i,
                      "sample",
                      (e.target as HTMLInputElement).value.trim() || undefined,
                    )}
                />
              </div>
              <div class="field">
                <label for="trigger-action-{i}">Action</label>
                <select
                  id="trigger-action-{i}"
                  class="input"
                  value={input.action ?? "trigger"}
                  onchange={(e) => {
                    const v = (e.target as HTMLSelectElement).value;
                    setOrDeleteInput(
                      i,
                      "action",
                      v === "trigger" ? undefined : v,
                    );
                  }}
                >
                  <option value="trigger">Trigger</option>
                  <option value="release">Release</option>
                </select>
              </div>
            </div>

            <div class="field-row-3">
              <div class="field">
                <label for="trigger-thresh-{i}">Threshold</label>
                <input
                  id="trigger-thresh-{i}"
                  type="number"
                  step="0.01"
                  min="0"
                  max="1"
                  class="input"
                  value={input.threshold ?? 0.1}
                  onchange={(e) =>
                    updateInput(
                      i,
                      "threshold",
                      parseFloat((e.target as HTMLInputElement).value),
                    )}
                />
              </div>
              <div class="field">
                <label for="trigger-gain-{i}">Gain</label>
                <input
                  id="trigger-gain-{i}"
                  type="number"
                  step="0.1"
                  min="0"
                  class="input"
                  value={input.gain ?? 1.0}
                  onchange={(e) => {
                    const v = parseFloat((e.target as HTMLInputElement).value);
                    setOrDeleteInput(i, "gain", v === 1.0 ? undefined : v);
                  }}
                />
              </div>
              <div class="field">
                <label for="trigger-rg-{i}">Release Group</label>
                <input
                  id="trigger-rg-{i}"
                  class="input"
                  placeholder="Optional"
                  value={input.release_group ?? ""}
                  onchange={(e) =>
                    setOrDeleteInput(
                      i,
                      "release_group",
                      (e.target as HTMLInputElement).value.trim() || undefined,
                    )}
                />
              </div>
            </div>

            {#if expandedInput === i}
              <div class="field-row-3">
                <div class="field">
                  <label for="trigger-retrig-{i}">Retrigger (ms)</label>
                  <input
                    id="trigger-retrig-{i}"
                    type="number"
                    min="0"
                    class="input"
                    value={input.retrigger_time_ms ?? 30}
                    onchange={(e) => {
                      const v = parseInt((e.target as HTMLInputElement).value);
                      setOrDeleteInput(
                        i,
                        "retrigger_time_ms",
                        v === 30 ? undefined : v,
                      );
                    }}
                  />
                </div>
                <div class="field">
                  <label for="trigger-scan-{i}">Scan (ms)</label>
                  <input
                    id="trigger-scan-{i}"
                    type="number"
                    min="0"
                    class="input"
                    value={input.scan_time_ms ?? 5}
                    onchange={(e) => {
                      const v = parseInt((e.target as HTMLInputElement).value);
                      setOrDeleteInput(
                        i,
                        "scan_time_ms",
                        v === 5 ? undefined : v,
                      );
                    }}
                  />
                </div>
                <div class="field">
                  <label for="trigger-vel-{i}">Velocity Curve</label>
                  <select
                    id="trigger-vel-{i}"
                    class="input"
                    value={input.velocity_curve ?? "linear"}
                    onchange={(e) => {
                      const v = (e.target as HTMLSelectElement).value;
                      setOrDeleteInput(
                        i,
                        "velocity_curve",
                        v === "linear" ? undefined : v,
                      );
                    }}
                  >
                    <option value="linear">Linear</option>
                    <option value="logarithmic">Logarithmic</option>
                    <option value="fixed">Fixed</option>
                  </select>
                </div>
              </div>

              <div class="field-row-3">
                <div class="field">
                  <label for="trigger-hp-{i}">Highpass (Hz)</label>
                  <input
                    id="trigger-hp-{i}"
                    type="number"
                    step="1"
                    class="input"
                    placeholder="Off"
                    value={input.highpass_freq ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLInputElement).value;
                      setOrDeleteInput(
                        i,
                        "highpass_freq",
                        v ? parseFloat(v) : undefined,
                      );
                    }}
                  />
                </div>
                <div class="field">
                  <label for="trigger-dynth-{i}">Dyn. Threshold (ms)</label>
                  <input
                    id="trigger-dynth-{i}"
                    type="number"
                    class="input"
                    placeholder="Off"
                    value={input.dynamic_threshold_decay_ms ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLInputElement).value;
                      setOrDeleteInput(
                        i,
                        "dynamic_threshold_decay_ms",
                        v ? parseInt(v) : undefined,
                      );
                    }}
                  />
                </div>
                <div class="field">
                  <label for="trigger-nf-{i}">Noise Floor Sens.</label>
                  <input
                    id="trigger-nf-{i}"
                    type="number"
                    step="0.1"
                    class="input"
                    placeholder="Off"
                    value={input.noise_floor_sensitivity ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLInputElement).value;
                      setOrDeleteInput(
                        i,
                        "noise_floor_sensitivity",
                        v ? parseFloat(v) : undefined,
                      );
                    }}
                  />
                </div>
              </div>

              {#if input.velocity_curve === "fixed"}
                <div class="field-row-3">
                  <div class="field">
                    <label for="trigger-fixvel-{i}">Fixed Velocity</label>
                    <input
                      id="trigger-fixvel-{i}"
                      type="number"
                      min="0"
                      max="127"
                      class="input"
                      value={input.fixed_velocity ?? 127}
                      onchange={(e) => {
                        const v = parseInt(
                          (e.target as HTMLInputElement).value,
                        );
                        setOrDeleteInput(
                          i,
                          "fixed_velocity",
                          v === 127 ? undefined : v,
                        );
                      }}
                    />
                  </div>
                </div>
              {/if}
            {/if}
          </div>
        {:else}
          <!-- MIDI trigger input -->
          <div class="input-fields">
            <div class="field-row-3">
              <div class="field">
                <label for="trigger-midi-type-{i}">Event Type</label>
                <select
                  id="trigger-midi-type-{i}"
                  class="input"
                  value={input.event?.type ?? "note_on"}
                  onchange={(e) =>
                    updateMidiEvent(
                      i,
                      "type",
                      (e.target as HTMLSelectElement).value,
                    )}
                >
                  <option value="note_on">Note On</option>
                  <option value="note_off">Note Off</option>
                  <option value="control_change">Control Change</option>
                </select>
              </div>
              <div class="field">
                <label for="trigger-midi-ch-{i}">Channel</label>
                <input
                  id="trigger-midi-ch-{i}"
                  type="number"
                  min="1"
                  max="16"
                  class="input"
                  value={input.event?.channel ?? 10}
                  onchange={(e) =>
                    updateMidiEvent(
                      i,
                      "channel",
                      parseInt((e.target as HTMLInputElement).value) || 1,
                    )}
                />
              </div>
              <div class="field">
                <label for="trigger-midi-key-{i}">Key / CC#</label>
                <input
                  id="trigger-midi-key-{i}"
                  type="number"
                  min="0"
                  max="127"
                  class="input"
                  value={input.event?.key ?? input.event?.control ?? 60}
                  onchange={(e) => {
                    const v =
                      parseInt((e.target as HTMLInputElement).value) || 0;
                    const type = input.event?.type ?? "note_on";
                    if (type === "control_change") {
                      updateMidiEvent(i, "control", v);
                    } else {
                      updateMidiEvent(i, "key", v);
                    }
                  }}
                />
              </div>
            </div>
            <div class="field">
              <label for="trigger-midi-sample-{i}">Sample</label>
              <input
                id="trigger-midi-sample-{i}"
                class="input"
                placeholder="Sample name"
                value={input.sample ?? ""}
                onchange={(e) =>
                  updateInput(
                    i,
                    "sample",
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
            </div>
          </div>
        {/if}
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
  .field label,
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field-row {
    display: flex;
    gap: 8px;
  }
  .field-row .input {
    flex: 1;
  }
  .field-row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .field-row-3 {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 12px;
  }
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .btn-group {
    display: flex;
    gap: 4px;
  }
  .input-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px;
    margin-top: 4px;
  }
  .input-card.midi {
    border-left: 3px solid var(--accent, #5c8ade);
  }
  .input-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }
  .input-kind {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .input-header-controls {
    display: flex;
    gap: 4px;
  }
  .input-fields {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .btn-small {
    padding: 2px 8px;
    font-size: 11px;
  }
  .btn-accent {
    background: var(--accent, #5c8ade);
    color: #fff;
    border-color: var(--accent, #5c8ade);
  }

  /* Calibration wizard styles */
  .cal-wizard {
    border: 1px solid var(--accent, #5c8ade);
    border-radius: var(--radius);
    padding: 12px;
    margin-bottom: 10px;
    background: var(--bg-base, #1a1a2e);
  }
  .cal-step {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .cal-title {
    font-weight: 600;
    font-size: 13px;
  }
  .cal-status {
    font-size: 12px;
    color: var(--text-muted);
  }
  .cal-stats {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
  }
  .cal-error {
    font-size: 12px;
    color: var(--danger, #e74c3c);
  }
  .cal-actions {
    display: flex;
    gap: 8px;
  }
  .cal-progress {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
  }
  .cal-progress-bar {
    height: 100%;
    background: var(--accent, #5c8ade);
    transition: width 1s linear;
  }
  .cal-results-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    font-size: 12px;
    font-family: monospace;
  }
  .cal-label {
    color: var(--text-muted);
  }

  @media (max-width: 600px) {
    .field-row-2,
    .field-row-3 {
      grid-template-columns: 1fr;
    }
  }
</style>
