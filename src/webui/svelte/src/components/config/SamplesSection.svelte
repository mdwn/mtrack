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
  import FileUpload from "../songs/FileUpload.svelte";
  import { uploadSampleFile } from "../../lib/api/config";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { showConfirm } from "../../lib/dialog.svelte";
  import Tooltip from "./Tooltip.svelte";

  export type SampleBrowseTarget = {
    sampleName: string;
    field: string;
    layerIndex?: number;
  };

  interface Props {
    samples: Record<string, any>;
    onchange: () => void;
    onbrowse: (target: SampleBrowseTarget) => void;
  }

  let { samples = $bindable(), onchange, onbrowse }: Props = $props();

  let editingName: string | null = $state(null);
  let collapsed: Record<string, boolean> = $state({});
  let advancedOpen: Record<string, boolean> = $state({});
  let uploading = $state(false);
  let uploadMsg = $state("");

  function hasAdvancedSettings(def: any): boolean {
    return (
      (def.release_behavior && def.release_behavior !== "play_to_completion") ||
      (def.retrigger && def.retrigger !== "cut") ||
      def.max_voices != null ||
      def.fade_time_ms != null ||
      def.velocity != null
    );
  }

  let sampleEntries = $derived(
    Object.entries(samples).sort(([a], [b]) => a.localeCompare(b)),
  );

  function addSample() {
    let name = "new_sample";
    let i = 1;
    while (samples[name]) {
      name = `new_sample_${i++}`;
    }
    samples[name] = { file: "" };
    editingName = name;
    onchange();
  }

  async function removeSample(name: string) {
    if (!(await showConfirm(get(t)("samples.confirmRemove", { values: { name } }), { danger: true }))) return;
    delete samples[name];
    samples = samples;
    onchange();
  }

  function renameSample(oldName: string, newName: string) {
    newName = newName.trim();
    if (!newName || newName === oldName || samples[newName]) return;
    const def = samples[oldName];
    delete samples[oldName];
    samples[newName] = def;
    editingName = null;
    onchange();
  }

  function toggleCollapse(name: string) {
    collapsed[name] = !collapsed[name];
  }

  function setOrDelete(name: string, key: string, value: any) {
    if (value === undefined || value === "" || value === null) {
      delete samples[name][key];
    } else {
      samples[name][key] = value;
    }
    onchange();
  }

  function addLayer(name: string) {
    if (!samples[name].velocity) samples[name].velocity = {};
    if (!samples[name].velocity.layers) samples[name].velocity.layers = [];
    samples[name].velocity.layers.push({ range: [1, 127], file: "" });
    onchange();
  }

  function removeLayer(name: string, index: number) {
    samples[name].velocity.layers.splice(index, 1);
    if (samples[name].velocity.layers.length === 0) {
      delete samples[name].velocity.layers;
    }
    onchange();
  }

  // Called by parent after browse completes
  export function applyBrowseResult(target: SampleBrowseTarget, path: string) {
    const { sampleName, field, layerIndex } = target;
    if (field === "file") {
      samples[sampleName].file = path;
    } else if (field === "layer" && layerIndex !== undefined) {
      samples[sampleName].velocity.layers[layerIndex].file = path;
    }
    onchange();
  }

  async function handleSampleUpload(
    sampleName: string,
    field: string,
    layerIndex: number | undefined,
    files: File[],
  ) {
    if (files.length === 0) return;
    uploading = true;
    uploadMsg = "";
    try {
      const result = await uploadSampleFile(files[0]);
      if (field === "file") {
        samples[sampleName].file = result.path;
      } else if (field === "layer" && layerIndex !== undefined) {
        samples[sampleName].velocity.layers[layerIndex].file = result.path;
      }
      uploadMsg = get(t)("samples.uploadedFile", {
        values: { name: files[0].name },
      });
      setTimeout(() => (uploadMsg = ""), 3000);
      onchange();
    } catch (e: any) {
      uploadMsg = e.message;
    } finally {
      uploading = false;
    }
  }
</script>

<div class="samples-section">
  {#if sampleEntries.length === 0}
    <div class="empty-state">
      <p>{$t("samples.noSamples")}</p>
      <p>{$t("samples.addSampleHint")}</p>
    </div>
  {/if}

  {#each sampleEntries as [name, def] (name)}
    <div class="sample-card">
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sample-header" onclick={() => toggleCollapse(name)}>
        <span class="sample-title">
          {#if editingName === name}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="input name-input"
              value={name}
              autofocus
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => {
                if (e.key === "Enter")
                  renameSample(name, (e.target as HTMLInputElement).value);
                if (e.key === "Escape") editingName = null;
              }}
              onblur={(e) =>
                renameSample(name, (e.target as HTMLInputElement).value)}
            />
          {:else}
            <span
              class="name-text"
              ondblclick={(e) => {
                e.stopPropagation();
                editingName = name;
              }}>{name}</span
            >
          {/if}
        </span>
        <div class="sample-controls">
          <button
            class="btn btn-danger btn-sm"
            onclick={(e) => {
              e.stopPropagation();
              removeSample(name);
            }}>{$t("common.remove")}</button
          >
          <span class="collapse-icon">{collapsed[name] ? "+" : "-"}</span>
        </div>
      </div>

      {#if !collapsed[name]}
        <div class="sample-body">
          <div class="field">
            <label for="sample-file-{name}">{$t("samples.file")}</label>
            <input
              id="sample-file-{name}"
              class="input"
              type="text"
              placeholder={$t("samples.filePlaceholder")}
              value={def.file ?? ""}
              onchange={(e) =>
                setOrDelete(
                  name,
                  "file",
                  (e.target as HTMLInputElement).value.trim() || undefined,
                )}
            />
            <div class="browse-row">
              <button
                class="btn"
                onclick={() => {
                  onbrowse({ sampleName: name, field: "file" });
                }}>{$t("samples.browseFilesystem")}</button
              >
            </div>
            <div class="upload-area">
              <FileUpload
                accept=".wav,.flac,.mp3,.ogg,.aac,.m4a,.mp4,.aiff,.aif"
                label={uploading
                  ? $t("common.uploading")
                  : $t("samples.dropAudio")}
                onupload={(files) =>
                  handleSampleUpload(name, "file", undefined, files)}
              />
            </div>
          </div>

          <div class="field">
            <label for="sample-track-{name}"
              >{$t("samples.outputTrack")}<Tooltip
                text={$t("tooltips.samples.outputTrack")}
              /></label
            >
            <input
              id="sample-track-{name}"
              class="input"
              type="text"
              placeholder={$t("samples.outputTrackPlaceholder")}
              value={def.output_track ?? ""}
              onchange={(e) =>
                setOrDelete(
                  name,
                  "output_track",
                  (e.target as HTMLInputElement).value.trim() || undefined,
                )}
            />
          </div>

          <!-- Advanced settings toggle -->
          <button
            class="advanced-toggle"
            type="button"
            onclick={() => (advancedOpen[name] = !advancedOpen[name])}
          >
            <span class="advanced-chevron" class:open={advancedOpen[name]}
              >&#9662;</span
            >
            {$t("samples.advanced")}
            {#if hasAdvancedSettings(def)}
              <span class="advanced-dot"></span>
            {/if}
          </button>

          {#if advancedOpen[name]}
            <div class="advanced-body">
              <div class="field-row-2">
                <div class="field">
                  <label for="sample-release-{name}"
                    >{$t("samples.releaseBehavior")}<Tooltip
                      text={$t("tooltips.samples.releaseBehavior")}
                    /></label
                  >
                  <select
                    id="sample-release-{name}"
                    class="input"
                    value={def.release_behavior ?? "play_to_completion"}
                    onchange={(e) => {
                      const v = (e.target as HTMLSelectElement).value;
                      setOrDelete(
                        name,
                        "release_behavior",
                        v === "play_to_completion" ? undefined : v,
                      );
                    }}
                  >
                    <option value="play_to_completion"
                      >{$t("samples.playToCompletion")}</option
                    >
                    <option value="stop">{$t("samples.releaseStop")}</option>
                    <option value="fade">{$t("samples.releaseFade")}</option>
                  </select>
                </div>

                <div class="field">
                  <label for="sample-retrigger-{name}"
                    >{$t("samples.retrigger")}<Tooltip
                      text={$t("tooltips.samples.retrigger")}
                    /></label
                  >
                  <select
                    id="sample-retrigger-{name}"
                    class="input"
                    value={def.retrigger ?? "cut"}
                    onchange={(e) => {
                      const v = (e.target as HTMLSelectElement).value;
                      setOrDelete(
                        name,
                        "retrigger",
                        v === "cut" ? undefined : v,
                      );
                    }}
                  >
                    <option value="cut">{$t("samples.retriggerCut")}</option>
                    <option value="polyphonic"
                      >{$t("samples.retriggerPolyphonic")}</option
                    >
                  </select>
                </div>
              </div>

              <div class="field-row-2">
                <div class="field">
                  <label for="sample-max-voices-{name}"
                    >{$t("samples.maxVoices")}<Tooltip
                      text={$t("tooltips.samples.maxVoices")}
                    /></label
                  >
                  <input
                    id="sample-max-voices-{name}"
                    class="input"
                    type="number"
                    placeholder={$t("samples.maxVoicesPlaceholder")}
                    value={def.max_voices ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLInputElement).value;
                      setOrDelete(
                        name,
                        "max_voices",
                        v ? parseInt(v) : undefined,
                      );
                    }}
                  />
                </div>

                <div class="field">
                  <label for="sample-fade-ms-{name}"
                    >{$t("samples.fadeTimeMs")}<Tooltip
                      text={$t("tooltips.samples.fadeTimeMs")}
                    /></label
                  >
                  <input
                    id="sample-fade-ms-{name}"
                    class="input"
                    type="number"
                    placeholder="50"
                    value={def.fade_time_ms ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLInputElement).value;
                      setOrDelete(
                        name,
                        "fade_time_ms",
                        v ? parseInt(v) : undefined,
                      );
                    }}
                  />
                </div>
              </div>

              <!-- Velocity -->
              <div class="subsection">
                <div class="field-header">
                  <span class="field-label">{$t("samples.velocity")}</span>
                </div>

                <div class="field-row-2">
                  <div class="field">
                    <label for="sample-vel-mode-{name}"
                      >{$t("samples.velocityMode")}<Tooltip
                        text={$t("tooltips.samples.velocityMode")}
                      /></label
                    >
                    <select
                      id="sample-vel-mode-{name}"
                      class="input"
                      value={def.velocity?.mode ?? "ignore"}
                      onchange={(e) => {
                        const v = (e.target as HTMLSelectElement).value;
                        if (v === "ignore") {
                          delete samples[name].velocity;
                        } else {
                          if (!samples[name].velocity)
                            samples[name].velocity = {};
                          samples[name].velocity.mode = v;
                        }
                        onchange();
                      }}
                    >
                      <option value="ignore"
                        >{$t("samples.velocityIgnore")}</option
                      >
                      <option value="scale"
                        >{$t("samples.velocityScale")}</option
                      >
                      <option value="layers"
                        >{$t("samples.velocityLayers")}</option
                      >
                    </select>
                  </div>

                  {#if (def.velocity?.mode ?? "ignore") === "ignore"}
                    <div class="field">
                      <label for="sample-vel-default-{name}"
                        >{$t("samples.defaultVelocity")}<Tooltip
                          text={$t("tooltips.samples.defaultVelocity")}
                        /></label
                      >
                      <input
                        id="sample-vel-default-{name}"
                        class="input"
                        type="number"
                        min="0"
                        max="127"
                        placeholder="100"
                        value={def.velocity?.default ?? ""}
                        onchange={(e) => {
                          const v = (e.target as HTMLInputElement).value;
                          if (!def.velocity) samples[name].velocity = {};
                          if (v) {
                            samples[name].velocity.default = parseInt(v);
                          } else {
                            delete samples[name].velocity?.default;
                          }
                          onchange();
                        }}
                      />
                    </div>
                  {/if}
                </div>

                {#if def.velocity?.mode === "layers"}
                  <div class="layers-section">
                    <div class="field-header">
                      <span class="field-label">{$t("samples.layers")}</span>
                      <button class="btn btn-sm" onclick={() => addLayer(name)}
                        >{$t("samples.addLayer")}</button
                      >
                    </div>

                    <label class="toggle-inline">
                      <input
                        type="checkbox"
                        checked={def.velocity?.scale ?? false}
                        onchange={(e) => {
                          if (!samples[name].velocity)
                            samples[name].velocity = {};
                          const checked = (e.target as HTMLInputElement)
                            .checked;
                          if (checked) {
                            samples[name].velocity.scale = true;
                          } else {
                            delete samples[name].velocity.scale;
                          }
                          onchange();
                        }}
                      />
                      {$t("samples.scaleByVelocity")}<Tooltip
                        text={$t("tooltips.samples.scaleByVelocity")}
                      />
                    </label>

                    {#each def.velocity?.layers ?? [] as layer, li (li)}
                      <div class="layer-card">
                        <div class="layer-row">
                          <span class="layer-label"
                            >Velocity {layer.range?.[0] ?? 0}-{layer
                              .range?.[1] ?? 127}</span
                          >
                          <input
                            class="input layer-range"
                            type="number"
                            min="0"
                            max="127"
                            placeholder="Min"
                            value={layer.range?.[0] ?? ""}
                            onchange={(e) => {
                              samples[name].velocity.layers[li].range[0] =
                                parseInt(
                                  (e.target as HTMLInputElement).value,
                                ) || 0;
                              onchange();
                            }}
                          />
                          <span class="range-sep">-</span>
                          <input
                            class="input layer-range"
                            type="number"
                            min="0"
                            max="127"
                            placeholder="Max"
                            value={layer.range?.[1] ?? ""}
                            onchange={(e) => {
                              samples[name].velocity.layers[li].range[1] =
                                parseInt(
                                  (e.target as HTMLInputElement).value,
                                ) || 127;
                              onchange();
                            }}
                          />
                          <button
                            class="btn btn-danger btn-sm"
                            onclick={() => removeLayer(name, li)}>X</button
                          >
                        </div>
                        <input
                          class="input"
                          type="text"
                          placeholder={$t("samples.layerFilePlaceholder")}
                          value={layer.file ?? ""}
                          onchange={(e) => {
                            samples[name].velocity.layers[li].file = (
                              e.target as HTMLInputElement
                            ).value.trim();
                            onchange();
                          }}
                        />
                        <div class="browse-row">
                          <button
                            class="btn"
                            onclick={() => {
                              onbrowse({
                                sampleName: name,
                                field: "layer",
                                layerIndex: li,
                              });
                            }}>{$t("samples.browseFilesystem")}</button
                          >
                        </div>
                        <div class="upload-area">
                          <FileUpload
                            accept=".wav,.flac,.mp3,.ogg,.aac,.m4a,.mp4,.aiff,.aif"
                            label={uploading
                              ? $t("common.uploading")
                              : $t("samples.dropAudio")}
                            onupload={(files) =>
                              handleSampleUpload(name, "layer", li, files)}
                          />
                        </div>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/each}

  {#if uploadMsg}
    <div class="msg" class:error={uploadMsg.toLowerCase().includes("fail")}>
      {uploadMsg}
    </div>
  {/if}

  <button class="btn btn-primary" onclick={addSample}
    >{$t("samples.addSample")}</button
  >
</div>

<style>
  .samples-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .empty-state {
    text-align: center;
    padding: 40px 20px;
    color: var(--text-dim);
  }
  .empty-state p {
    margin-bottom: 4px;
    font-size: 14px;
  }
  .sample-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }
  .sample-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 16px;
    cursor: pointer;
    background: none;
    border: none;
    width: 100%;
    text-align: left;
    font-family: var(--sans);
    transition: background 0.15s;
  }
  .sample-header:hover {
    background: var(--bg-card-hover);
  }
  .sample-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  .name-text {
    cursor: text;
  }
  .name-input {
    width: 200px;
    font-size: 14px;
    font-weight: 600;
  }
  .sample-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .collapse-icon {
    font-family: var(--mono);
    font-size: 15px;
    color: var(--text-dim);
    width: 16px;
    text-align: center;
  }
  .sample-body {
    padding: 0 16px 16px;
    border-top: 1px solid var(--border);
    padding-top: 12px;
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
  .advanced-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    cursor: pointer;
    padding: 4px 0;
    font-family: var(--sans);
    transition: color 0.15s;
  }
  .advanced-toggle:hover {
    color: var(--text-muted);
  }
  .advanced-chevron {
    font-size: 10px;
    transition: transform 0.15s;
    transform: rotate(-90deg);
  }
  .advanced-chevron.open {
    transform: rotate(0deg);
  }
  .advanced-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .advanced-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .browse-row {
    margin-top: 8px;
  }
  .upload-area {
    margin-top: 8px;
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
  .subsection {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 4px;
    border-top: 1px solid var(--border);
  }
  .layers-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .toggle-inline {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .layer-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .layer-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .layer-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    min-width: 80px;
  }
  .layer-range {
    width: 60px;
    flex-shrink: 0;
  }
  .range-sep {
    color: var(--text-dim);
    font-size: 13px;
  }
  .btn-sm {
    padding: 2px 8px;
    font-size: 12px;
  }
  .msg {
    font-size: 13px;
    color: var(--green);
  }
  .msg.error {
    color: var(--red);
  }
  @media (max-width: 600px) {
    .field-row-2 {
      grid-template-columns: 1fr;
    }
  }
</style>
