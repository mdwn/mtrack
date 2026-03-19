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
  import type { AudioDeviceInfo } from "../../lib/api/config";

  interface Props {
    audio: any;
    devices: AudioDeviceInfo[];
    trackNames: string[];
    onrefresh: () => void;
    onchange: () => void;
  }

  let {
    audio = $bindable(),
    devices,
    trackNames,
    onrefresh,
    onchange,
  }: Props = $props();

  let selectedDevice = $derived(devices.find((d) => d.name === audio.device));

  let sampleRateOptions = $derived(
    selectedDevice?.supported_sample_rates ?? [],
  );

  let sampleFormatOptions = $derived.by(() => {
    if (!selectedDevice) return [] as string[];
    const formats = new Set(
      selectedDevice.supported_formats.map((f) => f.sample_format),
    );
    return [...formats].sort();
  });

  let bitsPerSampleOptions = $derived.by(() => {
    if (!selectedDevice) return [] as number[];
    const currentFormat = audio.sample_format || "int";
    const bits = new Set(
      selectedDevice.supported_formats
        .filter((f) => f.sample_format === currentFormat)
        .map((f) => f.bits_per_sample),
    );
    return [...bits].sort((a, b) => a - b);
  });

  function set(key: string, value: any) {
    audio[key] = value;
    onchange();
  }

  function setOrDelete(key: string, value: any, defaultVal: any) {
    if (value === defaultVal || value === "" || value === undefined) {
      delete audio[key];
    } else {
      audio[key] = value;
    }
    onchange();
  }

  // Track mappings helpers
  let mappingEntries = $derived(
    Object.entries((audio.track_mappings || {}) as Record<string, number[]>),
  );

  function addMapping() {
    if (!audio.track_mappings) audio.track_mappings = {};
    audio.track_mappings[""] = [1];
    onchange();
  }

  function removeMapping(name: string) {
    delete audio.track_mappings[name];
    if (Object.keys(audio.track_mappings).length === 0) {
      delete audio.track_mappings;
    }
    onchange();
  }

  function updateMappingName(oldName: string, newName: string) {
    if (oldName === newName || !newName) return;
    const channels = audio.track_mappings[oldName];
    delete audio.track_mappings[oldName];
    audio.track_mappings[newName] = channels;
    onchange();
  }

  function updateMappingChannels(name: string, value: string) {
    audio.track_mappings[name] = value
      .split(",")
      .map((s: string) => parseInt(s.trim()))
      .filter((n: number) => !isNaN(n));
    onchange();
  }
</script>

<div class="section-fields">
  <div class="field">
    <label for="audio-device">{$t("audio.device")}</label>
    <div class="field-row">
      <input
        id="audio-device"
        class="input"
        list="audio-device-list"
        placeholder={$t("audio.devicePlaceholder")}
        value={audio.device || ""}
        onchange={(e) => set("device", (e.target as HTMLInputElement).value)}
      />
      <datalist id="audio-device-list">
        {#each devices as d (d.name)}
          <option value={d.name}
            >{d.name} ({d.max_channels}ch, {d.host_name})</option
          >
        {/each}
      </datalist>
      <button class="btn" onclick={onrefresh}>{$t("common.refresh")}</button>
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="audio-sample-rate">{$t("audio.sampleRate")}</label>
      <select
        id="audio-sample-rate"
        class="input"
        value={String(audio.sample_rate ?? "")}
        onchange={(e) => {
          const v = (e.target as HTMLSelectElement).value;
          setOrDelete("sample_rate", v ? parseInt(v) : undefined, undefined);
        }}
      >
        <option value="">{$t("common.default")}</option>
        {#if sampleRateOptions.length > 0}
          {#each sampleRateOptions as rate (rate)}
            <option value={String(rate)}>{rate}</option>
          {/each}
        {:else}
          <option value="44100">44100</option>
          <option value="48000">48000</option>
          <option value="96000">96000</option>
        {/if}
        {#if audio.sample_rate && !sampleRateOptions.includes(audio.sample_rate) && sampleRateOptions.length > 0}
          <option value={String(audio.sample_rate)}>{audio.sample_rate}</option>
        {/if}
      </select>
    </div>

    <div class="field">
      <label for="audio-sample-format">{$t("audio.sampleFormat")}</label>
      <select
        id="audio-sample-format"
        class="input"
        value={audio.sample_format ?? ""}
        onchange={(e) =>
          setOrDelete(
            "sample_format",
            (e.target as HTMLSelectElement).value || undefined,
            undefined,
          )}
      >
        <option value="">{$t("common.default")}</option>
        {#if sampleFormatOptions.length > 0}
          {#each sampleFormatOptions as fmt (fmt)}
            <option value={fmt}>{fmt}</option>
          {/each}
        {:else}
          <option value="int">int</option>
          <option value="float">float</option>
        {/if}
        {#if audio.sample_format && !sampleFormatOptions.includes(audio.sample_format) && sampleFormatOptions.length > 0}
          <option value={audio.sample_format}>{audio.sample_format}</option>
        {/if}
      </select>
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="audio-bits">{$t("audio.bitsPerSample")}</label>
      <select
        id="audio-bits"
        class="input"
        value={String(audio.bits_per_sample ?? "")}
        onchange={(e) => {
          const v = (e.target as HTMLSelectElement).value;
          setOrDelete(
            "bits_per_sample",
            v ? parseInt(v) : undefined,
            undefined,
          );
        }}
      >
        <option value="">{$t("common.default")}</option>
        {#if bitsPerSampleOptions.length > 0}
          {#each bitsPerSampleOptions as bits (bits)}
            <option value={String(bits)}>{bits}</option>
          {/each}
        {:else}
          <option value="16">16</option>
          <option value="24">24</option>
          <option value="32">32</option>
        {/if}
        {#if audio.bits_per_sample && !bitsPerSampleOptions.includes(audio.bits_per_sample) && bitsPerSampleOptions.length > 0}
          <option value={String(audio.bits_per_sample)}
            >{audio.bits_per_sample}</option
          >
        {/if}
      </select>
    </div>

    <div class="field">
      <label for="audio-buffer-size">{$t("audio.bufferSize")}</label>
      <input
        id="audio-buffer-size"
        type="number"
        class="input"
        placeholder="1024"
        value={audio.buffer_size ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("buffer_size", v ? parseInt(v) : undefined, undefined);
        }}
      />
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="audio-stream-buffer">{$t("audio.streamBufferSize")}</label>
      <input
        id="audio-stream-buffer"
        type="text"
        class="input"
        placeholder="default"
        value={typeof audio.stream_buffer_size === "number"
          ? audio.stream_buffer_size
          : (audio.stream_buffer_size ?? "")}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value.trim();
          if (!v || v === "default") {
            setOrDelete("stream_buffer_size", undefined, undefined);
          } else if (v === "min") {
            set("stream_buffer_size", "min");
          } else {
            const n = parseInt(v);
            setOrDelete(
              "stream_buffer_size",
              isNaN(n) ? undefined : n,
              undefined,
            );
          }
        }}
      />
    </div>

    <div class="field">
      <label for="audio-buffer-threads">{$t("audio.bufferThreads")}</label>
      <input
        id="audio-buffer-threads"
        type="number"
        class="input"
        placeholder="2"
        value={audio.buffer_threads ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("buffer_threads", v ? parseInt(v) : undefined, undefined);
        }}
      />
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="audio-resampler">{$t("audio.resampler")}</label>
      <select
        id="audio-resampler"
        class="input"
        value={audio.resampler ?? ""}
        onchange={(e) =>
          setOrDelete(
            "resampler",
            (e.target as HTMLSelectElement).value || undefined,
            undefined,
          )}
      >
        <option value="">{$t("audio.resamplerDefault")}</option>
        <option value="sinc">sinc</option>
        <option value="fft">fft</option>
      </select>
    </div>

    <div class="field">
      <label for="audio-delay">{$t("audio.playbackDelay")}</label>
      <input
        id="audio-delay"
        type="text"
        class="input"
        placeholder={$t("audio.playbackDelayPlaceholder")}
        value={audio.playback_delay ?? ""}
        onchange={(e) =>
          setOrDelete(
            "playback_delay",
            (e.target as HTMLInputElement).value.trim() || undefined,
            undefined,
          )}
      />
    </div>
  </div>

  <div class="field">
    <div class="field-header">
      <span class="field-label">{$t("audio.trackMappings")}</span>
      <button class="btn" onclick={addMapping}>{$t("common.add")}</button>
    </div>
    {#each mappingEntries as [name, channels], i (name)}
      <div class="mapping-row" data-index={i}>
        <input
          class="input mapping-name"
          list="track-name-list"
          value={name}
          placeholder={$t("audio.trackNamePlaceholder")}
          onchange={(e) =>
            updateMappingName(
              name,
              (e.target as HTMLInputElement).value.trim(),
            )}
        />
        <input
          class="input mapping-channels"
          value={channels.join(", ")}
          placeholder="1, 2"
          onchange={(e) =>
            updateMappingChannels(name, (e.target as HTMLInputElement).value)}
        />
        <button class="btn btn-danger" onclick={() => removeMapping(name)}
          >X</button
        >
      </div>
    {/each}
  </div>
</div>

<datalist id="track-name-list">
  {#each trackNames as name (name)}
    <option value={name}></option>
  {/each}
</datalist>

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
    font-size: 12px;
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
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .mapping-row {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .mapping-name {
    width: 140px;
    flex-shrink: 0;
  }
  .mapping-channels {
    flex: 1;
  }
  @media (max-width: 600px) {
    .field-row-2 {
      grid-template-columns: 1fr;
    }
  }
</style>
