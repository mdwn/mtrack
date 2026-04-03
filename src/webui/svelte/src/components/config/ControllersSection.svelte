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
    controllers: any[];
    onchange: () => void;
  }

  let { controllers = $bindable(), onchange }: Props = $props();

  let showOscAdvanced: Record<number, boolean> = $state({});
  let nextId = $state(controllers.length);

  // Assign stable keys to controllers for Svelte's keyed each block.
  // Initialize synchronously to avoid undefined keys on first render.
  let controllerKeys = $state<number[]>(
    Array.from({ length: controllers.length }, (_, i) => i),
  );
  $effect(() => {
    while (controllerKeys.length < controllers.length) {
      controllerKeys.push(nextId++);
    }
    if (controllerKeys.length > controllers.length) {
      controllerKeys.length = controllers.length;
    }
  });

  function addController(kind: string) {
    const newCtrl: any = { kind };
    controllers = [...controllers, newCtrl];
    controllerKeys = [...controllerKeys, nextId++];
    onchange();
  }

  function removeController(i: number) {
    controllers = [...controllers.slice(0, i), ...controllers.slice(i + 1)];
    controllerKeys = [
      ...controllerKeys.slice(0, i),
      ...controllerKeys.slice(i + 1),
    ];
    onchange();
  }

  function updateField(i: number, key: string, value: any) {
    if (value === undefined || value === "") {
      delete controllers[i][key];
    } else {
      controllers[i][key] = value;
    }
    onchange();
  }

  // OSC broadcast address helpers
  function addBroadcastAddr(i: number) {
    if (!controllers[i].broadcast_addresses)
      controllers[i].broadcast_addresses = [];
    controllers[i].broadcast_addresses.push("");
    onchange();
  }

  function removeBroadcastAddr(ci: number, ai: number) {
    controllers[ci].broadcast_addresses.splice(ai, 1);
    if (controllers[ci].broadcast_addresses.length === 0) {
      delete controllers[ci].broadcast_addresses;
    }
    onchange();
  }

  function updateBroadcastAddr(ci: number, ai: number, value: string) {
    controllers[ci].broadcast_addresses[ai] = value;
    onchange();
  }

  const oscPaths = [
    ["play", "/mtrack/play"],
    ["prev", "/mtrack/prev"],
    ["next", "/mtrack/next"],
    ["stop", "/mtrack/stop"],
    ["all_songs", "/mtrack/all_songs"],
    ["playlist", "/mtrack/playlist"],
    ["stop_samples", "/mtrack/samples/stop"],
    ["section_ack", "/mtrack/section_ack"],
    ["stop_section_loop", "/mtrack/stop_section_loop"],
    ["loop_section", "/mtrack/loop_section"],
    ["status", "/mtrack/status"],
    ["playlist_current", "/mtrack/playlist/current"],
    ["playlist_current_song", "/mtrack/playlist/current_song"],
    ["playlist_current_song_elapsed", "/mtrack/playlist/current_song/elapsed"],
  ];

  function toggleOscAdvanced(i: number) {
    showOscAdvanced[i] = !showOscAdvanced[i];
  }

  // Morningstar helpers
  const morningstarModels = [
    { value: "mc3", label: "MC3" },
    { value: "mc6", label: "MC6" },
    { value: "mc8", label: "MC8" },
    { value: "mc6pro", label: "MC6 Pro" },
    { value: "mc8pro", label: "MC8 Pro" },
    { value: "mc4pro", label: "MC4 Pro" },
    { value: "custom", label: "Custom" },
  ];

  function toggleMorningstar(i: number) {
    if (controllers[i].morningstar) {
      delete controllers[i].morningstar;
    } else {
      controllers[i].morningstar = { model: "mc4pro" };
    }
    onchange();
  }

  function updateMorningstarField(i: number, key: string, value: any) {
    if (!controllers[i].morningstar) return;
    if (value === undefined || value === "") {
      delete controllers[i].morningstar[key];
    } else {
      controllers[i].morningstar[key] = value;
    }
    onchange();
  }

  function getMorningstarModel(ctrl: any): string {
    const ms = ctrl.morningstar;
    if (!ms?.model) return "mc4pro";
    if (typeof ms.model === "string") return ms.model;
    if (typeof ms.model === "object" && ms.model.custom) return "custom";
    return "mc4pro";
  }

  function setMorningstarModel(i: number, value: string) {
    if (!controllers[i].morningstar) return;
    if (value === "custom") {
      const existingId =
        typeof controllers[i].morningstar.model === "object"
          ? (controllers[i].morningstar.model.custom?.model_id ?? 0)
          : 0;
      controllers[i].morningstar.model = { custom: { model_id: existingId } };
    } else {
      controllers[i].morningstar.model = value;
    }
    onchange();
  }

  function getCustomModelId(ctrl: any): number {
    const model = ctrl.morningstar?.model;
    if (typeof model === "object" && model.custom) {
      return model.custom.model_id ?? 0;
    }
    return 0;
  }

  function setCustomModelId(i: number, value: number) {
    if (!controllers[i].morningstar) return;
    controllers[i].morningstar.model = {
      custom: { model_id: value },
    };
    onchange();
  }

  // MIDI controller event definitions: [field, i18nKey, required]
  const midiActions: [string, string, boolean][] = [
    ["play", "controllers.midiPlay", true],
    ["prev", "controllers.midiPrev", true],
    ["next", "controllers.midiNext", true],
    ["stop", "controllers.midiStop", true],
    ["all_songs", "controllers.midiAllSongs", true],
    ["playlist", "controllers.midiPlaylist", true],
    ["section_ack", "controllers.midiSectionAck", false],
    ["stop_section_loop", "controllers.midiStopSectionLoop", false],
  ];

  function defaultMidiEvent(): MidiEvent {
    return { type: "note_on", channel: 1, key: 60, velocity: 127 };
  }

  function addMidiController() {
    const newCtrl: any = {
      kind: "midi",
      play: defaultMidiEvent(),
      prev: { ...defaultMidiEvent(), key: 61 },
      next: { ...defaultMidiEvent(), key: 62 },
      stop: { ...defaultMidiEvent(), key: 63 },
      all_songs: { ...defaultMidiEvent(), key: 64 },
      playlist: { ...defaultMidiEvent(), key: 65 },
    };
    controllers = [...controllers, newCtrl];
    controllerKeys = [...controllerKeys, nextId++];
    onchange();
  }

  function updateMidiAction(ci: number) {
    controllers[ci] = { ...controllers[ci] };
    onchange();
  }

  function toggleOptionalMidiAction(ci: number, field: string) {
    if (controllers[ci][field]) {
      delete controllers[ci][field];
    } else {
      controllers[ci][field] = defaultMidiEvent();
    }
    onchange();
  }
</script>

<div class="section-fields">
  {#each controllers as ctrl, i (controllerKeys[i])}
    <div class="controller-card">
      <div class="controller-header">
        <span class="controller-kind">{ctrl.kind?.toUpperCase()}</span>
        <button
          class="btn btn-danger btn-sm"
          onclick={() => removeController(i)}>{$t("common.remove")}</button
        >
      </div>

      {#if ctrl.kind === "grpc"}
        <div class="field">
          <label for="ctrl-grpc-port-{i}"
            >{$t("controllers.port")}
            <Tooltip text={$t("tooltips.controllers.grpcPort")} /></label
          >
          <input
            id="ctrl-grpc-port-{i}"
            type="number"
            class="input"
            placeholder="43234"
            value={ctrl.port ?? ""}
            onchange={(e) => {
              const v = (e.target as HTMLInputElement).value;
              updateField(i, "port", v ? parseInt(v) : undefined);
            }}
          />
        </div>
      {:else if ctrl.kind === "osc"}
        <div class="field">
          <label for="ctrl-osc-port-{i}"
            >{$t("controllers.port")}
            <Tooltip text={$t("tooltips.controllers.oscPort")} /></label
          >
          <input
            id="ctrl-osc-port-{i}"
            type="number"
            class="input"
            placeholder="43235"
            value={ctrl.port ?? ""}
            onchange={(e) => {
              const v = (e.target as HTMLInputElement).value;
              updateField(i, "port", v ? parseInt(v) : undefined);
            }}
          />
        </div>

        <div class="field">
          <div class="field-header">
            <span class="field-label"
              >{$t("controllers.broadcastAddresses")}
              <Tooltip
                text={$t("tooltips.controllers.broadcastAddresses")}
              /></span
            >
            <button class="btn" onclick={() => addBroadcastAddr(i)}
              >{$t("common.add")}</button
            >
          </div>
          {#each ctrl.broadcast_addresses || [] as addr, ai (ai)}
            <div class="addr-row">
              <input
                class="input"
                value={addr}
                placeholder={$t("controllers.broadcastPlaceholder")}
                onchange={(e) =>
                  updateBroadcastAddr(
                    i,
                    ai,
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
              <button
                class="btn btn-danger"
                onclick={() => removeBroadcastAddr(i, ai)}>X</button
              >
            </div>
          {/each}
        </div>

        <button class="btn btn-expand" onclick={() => toggleOscAdvanced(i)}>
          {showOscAdvanced[i]
            ? $t("controllers.hideOscPaths")
            : $t("controllers.showOscPaths")}
        </button>

        {#if showOscAdvanced[i]}
          <div class="osc-paths">
            {#each oscPaths as [key, defaultPath] (key)}
              <div class="field">
                <label for="osc-{key}-{i}">{key}</label>
                <input
                  id="osc-{key}-{i}"
                  class="input"
                  placeholder={defaultPath}
                  value={ctrl[key] ?? ""}
                  onchange={(e) =>
                    updateField(
                      i,
                      key,
                      (e.target as HTMLInputElement).value.trim() || undefined,
                    )}
                />
              </div>
            {/each}
          </div>
        {/if}
      {:else if ctrl.kind === "midi"}
        <p class="muted hint-text">{$t("controllers.midiHint")}</p>

        {#each midiActions as [field, labelKey, required] (field)}
          <div class="midi-action">
            {#if required}
              <div class="midi-action-header">
                <span class="midi-action-label">{$t(labelKey)}</span>
              </div>
              {#if ctrl[field]}
                <MidiEventEditor
                  bind:event={ctrl[field]}
                  onchange={() => updateMidiAction(i)}
                  idPrefix="midi-{i}-{field}"
                />
              {/if}
            {:else}
              <div class="midi-action-header">
                <label class="checkbox-label">
                  <input
                    type="checkbox"
                    checked={!!ctrl[field]}
                    onchange={() => toggleOptionalMidiAction(i, field)}
                  />
                  {$t(labelKey)}
                </label>
              </div>
              {#if ctrl[field]}
                <MidiEventEditor
                  bind:event={ctrl[field]}
                  onchange={() => updateMidiAction(i)}
                  idPrefix="midi-{i}-{field}"
                />
              {/if}
            {/if}
          </div>
        {/each}

        <div class="morningstar-section">
          <label class="checkbox-label">
            <input
              type="checkbox"
              checked={!!ctrl.morningstar}
              onchange={() => toggleMorningstar(i)}
            />
            {$t("controllers.morningstarEnable")}
            <Tooltip text={$t("tooltips.controllers.morningstar")} />
          </label>

          {#if ctrl.morningstar}
            <div class="morningstar-fields">
              <div class="field">
                <label for="ms-model-{i}"
                  >{$t("controllers.morningstarModel")}
                  <Tooltip
                    text={$t("tooltips.controllers.morningstarModel")}
                  /></label
                >
                <select
                  id="ms-model-{i}"
                  class="input"
                  value={getMorningstarModel(ctrl)}
                  onchange={(e) =>
                    setMorningstarModel(
                      i,
                      (e.target as HTMLSelectElement).value,
                    )}
                >
                  {#each morningstarModels as m (m.value)}
                    <option value={m.value}>{m.label}</option>
                  {/each}
                </select>
              </div>

              {#if getMorningstarModel(ctrl) === "custom"}
                <div class="field">
                  <label for="ms-custom-id-{i}"
                    >{$t("controllers.morningstarCustomModelId")}
                    <Tooltip
                      text={$t("tooltips.controllers.morningstarCustomModelId")}
                    /></label
                  >
                  <input
                    id="ms-custom-id-{i}"
                    type="number"
                    class="input"
                    min="0"
                    max="127"
                    value={getCustomModelId(ctrl)}
                    onchange={(e) =>
                      setCustomModelId(
                        i,
                        parseInt((e.target as HTMLInputElement).value) || 0,
                      )}
                  />
                </div>
              {/if}

              <label class="checkbox-label">
                <input
                  type="checkbox"
                  checked={ctrl.morningstar.save ?? false}
                  onchange={(e) =>
                    updateMorningstarField(
                      i,
                      "save",
                      (e.target as HTMLInputElement).checked || undefined,
                    )}
                />
                {$t("controllers.morningstarSave")}
                <Tooltip text={$t("tooltips.controllers.morningstarSave")} />
              </label>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/each}

  <div class="add-buttons">
    <button
      class="btn"
      title={$t("tooltips.controllers.addGrpc")}
      onclick={() => addController("grpc")}>{$t("controllers.addGrpc")}</button
    >
    <button
      class="btn"
      title={$t("tooltips.controllers.addOsc")}
      onclick={() => addController("osc")}>{$t("controllers.addOsc")}</button
    >
    <button
      class="btn"
      title={$t("tooltips.controllers.addMidi")}
      onclick={addMidiController}>{$t("controllers.addMidi")}</button
    >
  </div>
</div>

<style>
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .controller-card {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .controller-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .controller-kind {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: var(--text-muted);
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
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .addr-row {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .addr-row .input {
    flex: 1;
  }
  .btn-sm {
    padding: 3px 8px;
    font-size: 12px;
  }
  .btn-expand {
    font-size: 13px;
    align-self: flex-start;
  }
  .osc-paths {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .add-buttons {
    display: flex;
    gap: 8px;
  }
  .hint-text {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
  }
  .midi-action {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .midi-action-header {
    display: flex;
    align-items: center;
  }
  .midi-action-label {
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .morningstar-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .morningstar-fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    padding: 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
  }
  @media (max-width: 600px) {
    .osc-paths {
      grid-template-columns: 1fr;
    }
  }
</style>
