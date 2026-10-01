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
  import { get } from "svelte/store";
  import { showConfirm } from "../../lib/dialog.svelte";
  import Tooltip from "../config/Tooltip.svelte";
  import GdtfModePicker from "./GdtfModePicker.svelte";
  import FixtureTypeDetails from "./FixtureTypeDetails.svelte";
  import { trimNumber } from "../../lib/lighting/fixtureFacts";
  import {
    channelProblems,
    type ChannelProblem,
  } from "../../lib/lighting/venueRows";
  import {
    fetchFixtureTypes,
    fetchFixtureType,
    saveFixtureType,
    saveFixtureTypeText,
    deleteFixtureType,
    inspectGdtf,
    importGdtf,
    type FixtureTypeEntry,
    type GdtfSummary,
    type GdtfInspection,
    type GdtfImportReport,
    type LightingFileError,
  } from "../../lib/api/config";

  interface Props {
    /** Fixture type directory override (a profile's `lighting.directories`). */
    dir?: string;
    /** Venues directory override: which venue fixtures use each type. */
    venuesDir?: string;
  }

  let { dir = "", venuesDir = "" }: Props = $props();
  let ftDir = $derived(dir);

  // --- Fixture Types state ---
  let fixtureTypes = $state<Record<string, FixtureTypeEntry>>({});
  let ftLoading = $state(false);
  let ftError = $state("");
  let ftSaving = $state(false);
  let ftMsg = $state("");
  let editingFt = $state<string | null>(null);
  let editFtName = $state("");
  let editFtChannels = $state<{ name: string; offset: number }[]>([]);
  let editFtMaxStrobe = $state<string>("");
  let editFtMinStrobe = $state<string>("");
  let editFtStrobeDmxOffset = $state<string>("");
  let isNewFt = $state(false);
  /** The channel-map form can only say what a `.light` file holds; a
   *  `.fixture` type — rich channels, or a GDTF reference — is edited as
   *  the text of its file. */
  let ftMode = $state<"form" | "text">("form");
  let editFtDsl = $state("");
  let editFtExt = $state<"light" | "fixture">("light");
  let editFtReferential = $state(false);
  /** A GDTF card's second line: "31 modes · wash, 13°", the beam only as
   *  far as the archive states it. */
  function cardModes(g: GdtfSummary): string {
    const beam = [
      g.beam?.type?.toLowerCase(),
      g.beam?.angle != null
        ? $t("lighting.ftCard.angle", {
            values: { angle: trimNumber(g.beam.angle) },
          })
        : null,
    ]
      .filter(Boolean)
      .join(", ");
    const modes = $t("lighting.gdtf.modeCount", { values: { count: g.modes } });
    return beam ? `${modes} · ${beam}` : modes;
  }

  /** An existing GDTF type opens on the fixture, its text last. */
  const showFtDetails = $derived(editFtReferential && !isNewFt);
  /** The file's text as last read, so its edits can be told apart. */
  let ftDslSaved = $state("");
  /** A GDTF type's page has two ways to change its file: the settings form
   *  and the file's text. Only one may have unsaved changes at a time —
   *  each locks the other — and a save of either reloads both. */
  let ftTextDirty = $derived(showFtDetails && editFtDsl !== ftDslSaved);
  let ftFormDirty = $state(false);
  /** The settings form's last word, kept across a rename's reload. */
  let ftNotice = $state<{ ok: boolean; text: string } | null>(null);
  /** Bumped after a save, so the page re-reads the archive's answer. */
  let ftDetailsTick = $state(0);

  /** After the settings form saved: the type may have a new name, and its
   *  file new text. */
  async function onSettingsSaved(
    name: string,
    notice: { ok: boolean; text: string },
  ) {
    ftNotice = notice;
    editingFt = name;
    editFtName = name;
    ftFormDirty = false;
    await loadFixtureTypes();
    await openFtAsText(name);
    ftDetailsTick++;
  }

  /** Saves the file's text from the GDTF type's page and stays on it. */
  async function saveFtFile() {
    const name = editFtDslName;
    if (await saveFtText(true)) {
      editFtName = name;
      await openFtAsText(name);
      ftDetailsTick++;
    }
  }
  const editingGdtf = $derived(
    editingFt ? (fixtureTypes[editingFt]?.gdtf ?? null) : null,
  );
  let editFtRich = $state(false);
  let ftTextLoading = $state(false);
  /** In text mode the DSL declares the name — the file is keyed on it, and
   *  the server refuses a save where the two disagree. So the Name field
   *  reads it out rather than being a second, conflicting source. */
  let editFtDslName = $derived(declaredFixtureTypeName(editFtDsl));
  /** A rich or referential type cannot go back to `.light`: the loader
   *  skips v2 syntax there. Anything else may be saved in either form. */
  let ftExtChoosable = $derived(
    ftMode === "text" && !editFtReferential && !editFtRich,
  );
  /** Whether the "which form?" chooser is open ahead of a new type. */
  let newFtChoice = $state(false);

  /** The name a fixture type DSL declares, quoted or bare. Comment lines
   *  start with `#`, so anchoring to the start of a line skips them. */
  function declaredFixtureTypeName(dsl: string): string {
    const match = dsl.match(/^[ \t]*fixture_type[ \t]+(?:"([^"]*)"|([\w-]+))/m);
    return (match?.[1] ?? match?.[2] ?? "").trim();
  }

  /** The commented starting point for a hand-written `.fixture`. */
  const NEW_FIXTURE_TEMPLATE = `# The rich channel form, which only a .fixture file may hold:
#
#   channel "name" @ coarse [fine N] [range a..b] [{ function ... }]
fixture_type "Name" {
  channel "dimmer" @ 1
}
`;

  let ftFileErrors = $state<LightingFileError[]>([]);

  // Available fixture type names

  async function loadFixtureTypes() {
    ftLoading = true;
    ftError = "";
    ftFileErrors = [];
    try {
      const result = await fetchFixtureTypes(
        ftDir || undefined,
        venuesDir || undefined,
      );
      fixtureTypes = result.fixtureTypes;
      ftFileErrors = result.errors;
    } catch (e: any) {
      ftError = e.message;
    } finally {
      ftLoading = false;
    }
  }

  $effect(() => {
    // Re-load when the directory changes
    void ftDir;
    void venuesDir;
    loadFixtureTypes();
  });

  // --- Fixture Type editing ---

  async function startEditFt(name: string) {
    const entry = fixtureTypes[name];
    ftNotice = null;
    editingFt = name;
    editFtName = name;
    isNewFt = false;
    ftMsg = "";
    editFtExt = entry.extension === "fixture" ? "fixture" : "light";
    editFtReferential = entry.referential;
    editFtRich = entry.rich;

    if (editFtExt === "fixture" || entry.rich || entry.referential) {
      await openFtAsText(name);
      return;
    }

    ftMode = "form";
    const ft = entry.fixture_type;
    editFtChannels = Object.entries(ft.channels)
      .sort(([, a], [, b]) => a - b)
      .map(([n, o]) => ({ name: n, offset: o }));
    editFtMaxStrobe =
      ft.max_strobe_frequency != null ? String(ft.max_strobe_frequency) : "";
    editFtMinStrobe =
      ft.min_strobe_frequency != null ? String(ft.min_strobe_frequency) : "";
    editFtStrobeDmxOffset =
      ft.strobe_dmx_offset != null ? String(ft.strobe_dmx_offset) : "";
  }

  /** Opens a type as the text of its file, whatever form it is in. The
   *  listing carries the parsed type, not its source, and a referential type
   *  has no channels here at all — so the file itself is fetched. */
  async function openFtAsText(name: string) {
    ftMode = "text";
    editFtDsl = "";
    ftTextLoading = true;
    try {
      const full = await fetchFixtureType(name, ftDir || undefined);
      editFtDsl = full.dsl;
      ftDslSaved = full.dsl;
    } catch (e: any) {
      ftMsg = e.message;
    } finally {
      ftTextLoading = false;
    }
  }

  /** Opens a `.light` type in the text editor — the way out of the channel
   *  map, and the only path from v1 to the rich form. */
  async function editFtAsText(name: string) {
    const entry = fixtureTypes[name];
    editingFt = name;
    editFtName = name;
    isNewFt = false;
    ftMsg = "";
    editFtExt = entry.extension === "fixture" ? "fixture" : "light";
    editFtReferential = entry.referential;
    editFtRich = entry.rich;
    await openFtAsText(name);
  }

  function startNewFt(ext: "light" | "fixture") {
    newFtChoice = false;
    editingFt = "__new__";
    editFtName = "";
    editFtExt = ext;
    editFtReferential = false;
    editFtRich = false;
    isNewFt = true;
    ftMsg = "";
    if (ext === "fixture") {
      ftMode = "text";
      editFtDsl = NEW_FIXTURE_TEMPLATE;
      return;
    }
    ftMode = "form";
    editFtChannels = [{ name: "dimmer", offset: 1 }];
    editFtMaxStrobe = "";
    editFtMinStrobe = "";
    editFtStrobeDmxOffset = "";
  }

  // GDTF import flow: pick a file → inspect (modes) → pick a mode in the
  // picker, which says what each lets a show do → import.
  let gdtfFileInput = $state<HTMLInputElement | null>(null);
  let gdtfFile = $state<File | null>(null);
  let gdtfInspection = $state<GdtfInspection | null>(null);
  let gdtfBusy = $state(false);
  let gdtfError = $state("");
  let gdtfReport = $state<GdtfImportReport | null>(null);

  function resetGdtf() {
    gdtfFile = null;
    gdtfInspection = null;
    gdtfError = "";
    gdtfReport = null;
  }

  async function onGdtfFileChosen(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    resetGdtf();
    gdtfFile = file;
    gdtfBusy = true;
    try {
      gdtfInspection = await inspectGdtf(file);
    } catch (err) {
      gdtfError = err instanceof Error ? err.message : String(err);
      gdtfFile = null;
    } finally {
      gdtfBusy = false;
    }
  }

  async function runGdtfImport(mode: string, name: string) {
    if (!gdtfFile || !mode) return;
    gdtfBusy = true;
    gdtfError = "";
    try {
      gdtfReport = await importGdtf(gdtfFile, mode, name || undefined);
      gdtfInspection = null;
      gdtfFile = null;
      await loadFixtureTypes();
    } catch (err) {
      gdtfError = err instanceof Error ? err.message : String(err);
    } finally {
      gdtfBusy = false;
    }
  }

  function cancelEditFt() {
    ftNotice = null;
    editingFt = null;
    newFtChoice = false;
  }

  function addFtChannel() {
    const nextOffset =
      editFtChannels.length > 0
        ? Math.max(...editFtChannels.map((c) => c.offset)) + 1
        : 1;
    editFtChannels = [...editFtChannels, { name: "", offset: nextOffset }];
  }

  /** Set by a refused save: the channel rows are then checked as edited. */
  let checkChannels = $state(false);
  let channelIssues = $derived(
    checkChannels
      ? channelProblems(editFtChannels)
      : new Map<number, ChannelProblem[]>(),
  );

  function removeFtChannel(i: number) {
    editFtChannels = editFtChannels.filter((_, idx) => idx !== i);
  }

  async function saveFt() {
    if (ftMode === "text") {
      await saveFtText();
      return;
    }
    if (!editFtName.trim()) {
      ftMsg = get(t)("lighting.nameRequired");
      return;
    }
    // Every channel row is saved or the save is refused: a blank name, a
    // name another row has, or an address below 1 is marked, never dropped
    // or merged into its namesake.
    const found = channelProblems(editFtChannels);
    if (found.size > 0) {
      checkChannels = true;
      ftMsg = get(t)("lighting.channelRowsNeedAttention", {
        values: { count: found.size },
      });
      const first = Math.min(...found.keys());
      queueMicrotask(() =>
        document
          .querySelector<HTMLElement>(
            `[data-channel-row="${first}"] [aria-invalid="true"]`,
          )
          ?.focus(),
      );
      return;
    }
    const channels: Record<string, number> = {};
    for (const ch of editFtChannels) {
      channels[ch.name.trim()] = ch.offset;
    }
    if (Object.keys(channels).length === 0) {
      ftMsg = get(t)("lighting.channelRequired");
      return;
    }
    const newName = editFtName.trim();
    const oldName = editingFt !== "__new__" ? editingFt : null;
    const isRename = oldName && oldName !== newName;
    ftSaving = true;
    ftMsg = "";
    try {
      await saveFixtureType(
        newName,
        {
          channels,
          max_strobe_frequency: editFtMaxStrobe
            ? parseFloat(editFtMaxStrobe)
            : null,
          min_strobe_frequency: editFtMinStrobe
            ? parseFloat(editFtMinStrobe)
            : null,
          strobe_dmx_offset: editFtStrobeDmxOffset
            ? parseInt(editFtStrobeDmxOffset)
            : null,
        },
        ftDir || undefined,
      );
      if (isRename) {
        await deleteFixtureType(oldName, ftDir || undefined);
      }
      await loadFixtureTypes();
      editingFt = null;
      ftMsg = get(t)("common.saved");
      setTimeout(() => (ftMsg = ""), 2000);
    } catch (e: any) {
      ftMsg = e.message;
    } finally {
      ftSaving = false;
    }
  }

  /** Saves the file's text; `stay` keeps the type open afterwards (the GDTF
   *  page's file disclosure). Whether it saved. */
  async function saveFtText(stay = false): Promise<boolean> {
    // The text is the file, so the name it declares is the name to save
    // under: a URL naming anything else would write a file the panel could
    // never reach again, and the server refuses that outright.
    const newName = editFtDslName;
    if (!newName) {
      ftMsg = get(t)("lighting.fixtureTypeNoName");
      return false;
    }
    const oldName = editingFt !== "__new__" ? editingFt : null;
    const isRename = oldName && oldName !== newName;
    ftSaving = true;
    ftMsg = "";
    try {
      await saveFixtureTypeText(
        newName,
        editFtDsl,
        editFtExt,
        ftDir || undefined,
      );
      if (isRename) {
        await deleteFixtureType(oldName, ftDir || undefined);
      }
      await loadFixtureTypes();
      editingFt = stay ? newName : null;
      ftMsg = get(t)("common.saved");
      setTimeout(() => (ftMsg = ""), 2000);
      return true;
    } catch (e: any) {
      ftMsg = e.message;
      return false;
    } finally {
      ftSaving = false;
    }
  }

  async function removeFt(name: string) {
    if (
      !(await showConfirm(
        get(t)("lighting.deleteFixtureType", { values: { name } }),
        { danger: true },
      ))
    )
      return;
    try {
      await deleteFixtureType(name, ftDir || undefined);
      await loadFixtureTypes();
    } catch (e: any) {
      ftMsg = e.message;
    }
  }
</script>

<div class="sub-panel">
  {#if editingFt}
    <!-- Fixture Type Editor -->
    <div class="editor-form">
      <div class="editor-header">
        {#if showFtDetails}
          <!-- A fixture from an archive is titled as the fixture it is. -->
          <div>
            <h4 class="editor-title" data-testid="ft-title">{editingFt}</h4>
            {#if editingGdtf}
              <div class="item-meta" data-testid="ft-title-sub">
                {editingGdtf.manufacturer} · {editingGdtf.fixture}
              </div>
            {/if}
          </div>
        {:else}
          <h4 class="editor-title">
            {isNewFt
              ? $t("lighting.newFixtureType")
              : $t("lighting.editFixtureType", {
                  values: { name: editingFt },
                })}
          </h4>
        {/if}
        <div class="editor-actions">
          {#if ftMsg}
            <span
              class="save-msg"
              class:save-error={ftMsg !== get(t)("common.saved")}>{ftMsg}</span
            >
          {/if}
          <button class="btn" onclick={cancelEditFt}
            >{showFtDetails ? $t("common.back") : $t("common.cancel")}</button
          >
          {#if !showFtDetails}
            <!-- A GDTF type's page saves its settings and its file each
                 with their own button. -->
            <button
              class="btn btn-primary"
              onclick={saveFt}
              disabled={ftSaving}
            >
              {ftSaving ? $t("common.saving") : $t("common.save")}
            </button>
          {/if}
        </div>
      </div>

      {#if showFtDetails}
        <!-- The fixture first, the user's settings for it, and the file
             they are saved in behind a disclosure. -->
        <FixtureTypeDetails
          name={editingFt}
          dir={ftDir || undefined}
          venuesDir={venuesDir || undefined}
          fileDirty={ftTextDirty}
          onformdirty={(dirty) => (ftFormDirty = dirty)}
          onsaved={onSettingsSaved}
          notice={ftNotice}
          refresh={ftDetailsTick}
        >
          {#snippet file()}
            <div class="field" data-testid="ft-text-editor">
              <p class="field-hint" data-testid="ft-referential-note">
                {$t("lighting.fixtureTypeReferential")}
              </p>
              {#if ftTextLoading}
                <p class="status-text">{$t("common.loading")}</p>
              {:else}
                <textarea
                  class="raw-textarea"
                  data-testid="ft-dsl"
                  bind:value={editFtDsl}
                  readonly={ftFormDirty}
                  spellcheck="false"
                ></textarea>
              {/if}
              <div class="editor-actions">
                <button
                  class="btn btn-primary btn-sm"
                  type="button"
                  data-testid="ft-file-save"
                  disabled={!ftTextDirty || ftSaving}
                  onclick={saveFtFile}
                  >{ftSaving
                    ? $t("common.saving")
                    : $t("lighting.settings.saveFile")}</button
                >
                {#if ftTextDirty}
                  <button
                    class="btn btn-sm"
                    type="button"
                    onclick={() => (editFtDsl = ftDslSaved)}
                    >{$t("common.discard")}</button
                  >
                {/if}
              </div>
            </div>
          {/snippet}
        </FixtureTypeDetails>
      {/if}

      {#if !showFtDetails}
        <div class="field">
          <label for="ft-name">{$t("lighting.name")}</label>
          {#if ftMode === "text"}
            <!-- The file is keyed on the name the DSL declares, so the
                   field reads it out instead of competing with it. -->
            <input
              id="ft-name"
              class="input"
              data-testid="ft-name-derived"
              value={editFtDslName}
              readonly
            />
            <span class="field-hint"
              >{$t("lighting.fixtureTypeNameFromDsl")}</span
            >
          {:else}
            <input
              id="ft-name"
              class="input"
              bind:value={editFtName}
              placeholder="e.g. RGBW_Par"
            />
          {/if}
        </div>

        {#if ftMode === "text"}
          <div class="field" data-testid="ft-text-editor">
            <span class="field-label"
              >{$t("lighting.fixtureTypeTextMode", {
                values: { ext: editFtExt },
              })}</span
            >
            {#if editFtReferential}
              <p class="field-hint" data-testid="ft-referential-note">
                {$t("lighting.fixtureTypeReferential")}
              </p>
            {:else}
              <p class="field-hint">{$t("lighting.fixtureTypeTextHint")}</p>
            {/if}
            {#if ftExtChoosable}
              <!-- The one way out of v1: a plain type may be saved back as
                     a `.light` or converted to a `.fixture`. A rich or
                     referential type has no choice — v2 syntax in a `.light`
                     file is skipped by the loader. -->
              <label class="ext-choice">
                {$t("lighting.fixtureTypeSaveAs")}
                <select
                  class="input"
                  data-testid="ft-ext-select"
                  bind:value={editFtExt}
                >
                  <option value="light">.light</option>
                  <option value="fixture">.fixture</option>
                </select>
              </label>
            {/if}
            {#if ftTextLoading}
              <p class="status-text">{$t("common.loading")}</p>
            {:else}
              <textarea
                class="raw-textarea"
                data-testid="ft-dsl"
                bind:value={editFtDsl}
                spellcheck="false"
              ></textarea>
            {/if}
          </div>
        {:else}
          <div class="subsection">
            <div class="subsection-header">
              <span class="field-label"
                >{$t("lighting.channelMap")}<Tooltip
                  text={$t("tooltips.lighting.channelMap")}
                /></span
              >
              <button class="btn btn-sm" onclick={addFtChannel}
                >{$t("lighting.addChannel")}</button
              >
            </div>
            {#each editFtChannels as ch, i (i)}
              {@const issues = channelIssues.get(i) ?? []}
              <div
                class="channel-row"
                data-channel-row={i}
                data-testid="ft-channel-row"
              >
                <input
                  class="input channel-name"
                  placeholder={$t("lighting.channelName")}
                  aria-label={$t("lighting.channelName")}
                  aria-invalid={issues.includes("noName") ||
                    issues.includes("duplicateName")}
                  aria-describedby={issues.length > 0
                    ? `ft-channel-error-${i}`
                    : undefined}
                  bind:value={ch.name}
                />
                <input
                  class="input channel-offset"
                  type="number"
                  min="1"
                  placeholder={$t("lighting.offset")}
                  aria-label={$t("lighting.offset")}
                  aria-invalid={issues.includes("badOffset")}
                  bind:value={ch.offset}
                />
                <button
                  class="btn btn-danger btn-sm"
                  onclick={() => removeFtChannel(i)}>X</button
                >
              </div>
              {#if issues.length > 0}
                <p
                  class="row-error"
                  id={`ft-channel-error-${i}`}
                  data-testid="ft-channel-error"
                >
                  {issues.map((p) => $t(`lighting.channelRow.${p}`)).join(" ")}
                </p>
              {/if}
            {/each}
          </div>

          <div class="field-row-3">
            <div class="field">
              <label for="ft-max-strobe"
                >{$t("lighting.maxStrobeFreq")}<Tooltip
                  text={$t("tooltips.lighting.maxStrobeFreq")}
                /></label
              >
              <input
                id="ft-max-strobe"
                class="input"
                type="number"
                step="0.1"
                placeholder="e.g. 25.0"
                bind:value={editFtMaxStrobe}
              />
            </div>
            <div class="field">
              <label for="ft-min-strobe"
                >{$t("lighting.minStrobeFreq")}<Tooltip
                  text={$t("tooltips.lighting.minStrobeFreq")}
                /></label
              >
              <input
                id="ft-min-strobe"
                class="input"
                type="number"
                step="0.1"
                placeholder="e.g. 0.4"
                bind:value={editFtMinStrobe}
              />
            </div>
            <div class="field">
              <label for="ft-strobe-offset"
                >{$t("lighting.strobeDmxOffset")}<Tooltip
                  text={$t("tooltips.lighting.strobeDmxOffset")}
                /></label
              >
              <input
                id="ft-strobe-offset"
                class="input"
                type="number"
                min="0"
                placeholder="e.g. 7"
                bind:value={editFtStrobeDmxOffset}
              />
            </div>
          </div>
        {/if}
      {/if}
    </div>
  {:else}
    <!-- Fixture Type List -->
    <div class="list-header">
      <span class="field-hint"
        >{$t("lighting.fixtureTypeCount", {
          values: { count: Object.keys(fixtureTypes).length },
        })}</span
      >
      <div class="list-actions">
        {#if ftMsg}
          <span
            class="save-msg"
            class:save-error={ftMsg !== get(t)("common.saved")}>{ftMsg}</span
          >
        {/if}
        <button class="btn" onclick={loadFixtureTypes} disabled={ftLoading}
          >{$t("common.refresh")}</button
        >
        <button
          class="btn"
          data-testid="import-gdtf"
          onclick={() => gdtfFileInput?.click()}
          >{$t("lighting.importGdtf")}</button
        >
        <button
          class="btn btn-primary"
          onclick={() => (newFtChoice = !newFtChoice)}
          >{$t("lighting.newFixtureType")}</button
        >
        <input
          type="file"
          accept=".gdtf"
          style="display: none"
          bind:this={gdtfFileInput}
          onchange={onGdtfFileChosen}
        />
      </div>
    </div>
    {#if newFtChoice}
      <!-- The form a type is born in is a choice, not a default: the
               channel map cannot say what a .fixture file holds. -->
      <div class="new-ft-choice" data-testid="new-ft-choice">
        <button class="btn" onclick={() => startNewFt("light")}
          >{$t("lighting.newFixtureTypeLight")}</button
        >
        <button
          class="btn"
          data-testid="new-ft-fixture"
          onclick={() => startNewFt("fixture")}
          >{$t("lighting.newFixtureTypeFixture")}</button
        >
      </div>
    {/if}
    {#if gdtfError}
      <div class="file-errors" data-testid="gdtf-error">{gdtfError}</div>
    {/if}
    {#if gdtfInspection}
      <GdtfModePicker
        inspection={gdtfInspection}
        busy={gdtfBusy}
        oncancel={resetGdtf}
        onimport={runGdtfImport}
      />
    {/if}
    {#if gdtfReport}
      <div class="editor-form" data-testid="gdtf-report">
        <div class="editor-header">
          <h4 class="editor-title">
            {$t("lighting.gdtfImported", {
              values: { name: gdtfReport.type_name },
            })}
          </h4>
          <div class="editor-actions">
            <button class="btn" onclick={resetGdtf}>{$t("common.close")}</button
            >
          </div>
        </div>
        <div class="field-hint">{gdtfReport.fixture_file}</div>
        <ul>
          {#each gdtfReport.channels as [offset, channel] (offset)}
            <li>channel {offset}: {channel}</li>
          {/each}
        </ul>
        {#if gdtfReport.warnings.length > 0}
          <div class="field-hint">{$t("lighting.gdtfWarnings")}</div>
          <ul class="file-errors">
            {#each gdtfReport.warnings as warning (warning)}
              <li>{warning}</li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
    {#if ftFileErrors.length > 0}
      <ul class="file-errors" data-testid="fixture-type-file-errors">
        {#each ftFileErrors as fe (fe.file)}
          <li class="status-text error-text">{fe.file}: {fe.error}</li>
        {/each}
      </ul>
    {/if}
    {#if ftLoading}
      <p class="status-text">{$t("common.loading")}</p>
    {:else if ftError}
      <p class="status-text error-text">{ftError}</p>
    {:else if Object.keys(fixtureTypes).length === 0}
      <div class="empty-state">
        <p>{$t("lighting.noFixtureTypes")}</p>
        <p>
          {$t("lighting.fixtureTypeHint")}
        </p>
      </div>
    {:else}
      <div class="item-grid">
        {#each Object.entries(fixtureTypes).sort( ([a], [b]) => a.localeCompare(b), ) as [name, entry] (name)}
          {@const ft = entry.fixture_type}
          <div
            class="item-card"
            class:item-card--gdtf={entry.referential}
            role="button"
            tabindex="0"
            onclick={() => startEditFt(name)}
            onkeydown={(e) => {
              if (e.key === "Enter") startEditFt(name);
            }}
          >
            {#if entry.referential}
              <!-- A fixture from an archive: its picture, or a box saying
                   there is none yet (rigs are made when a type loads). -->
              {#if entry.gdtf?.thumbnail}
                <img
                  class="card-thumb"
                  src={`/api/lighting/assets/${entry.gdtf.thumbnail}`}
                  alt=""
                  data-testid="ft-card-thumb"
                />
              {:else}
                <div class="card-thumb card-thumb--none" aria-hidden="true">
                  {$t("lighting.ftCard.noModel")}
                </div>
              {/if}
            {/if}
            <div class="card-body">
              <div class="item-card-header">
                <span class="item-name">{name}</span>
                <span class="ext-badge" data-testid="ft-ext"
                  >.{entry.extension}</span
                >
                {#if entry.extension === "light"}
                  <!-- The channel map is the default way in; this is the
                           way out of it, and the only path from v1 to the
                           rich form. -->
                  <button
                    class="btn btn-sm"
                    data-testid="ft-edit-text"
                    onclick={(e) => {
                      e.stopPropagation();
                      editFtAsText(name);
                    }}>{$t("lighting.editAsText")}</button
                  >
                {/if}
                <button
                  class="btn btn-danger btn-sm"
                  onclick={(e) => {
                    e.stopPropagation();
                    removeFt(name);
                  }}>{$t("common.delete")}</button
                >
              </div>
              <div class="item-meta item-file">{entry.file}</div>
              {#if entry.referential && entry.gdtf}
                {@const g = entry.gdtf}
                <div class="item-meta" data-testid="ft-card-fixture">
                  {g.manufacturer} · {g.fixture}
                </div>
                <div class="item-meta card-modes" data-testid="ft-card-modes">
                  <span>{cardModes(g)}</span>
                  {#if g.in_use.length > 0}
                    <!-- The modes the venues use, most-used first. -->
                    {#each g.in_use.slice(0, 3) as use (use.mode)}
                      <span class="mode-pill" data-testid="ft-card-mode"
                        >{$t("lighting.ftCard.modeUsed", {
                          values: { mode: use.mode, count: use.count },
                        })}</span
                      >
                    {/each}
                    {#if g.in_use.length > 3}
                      <span class="item-meta" data-testid="ft-card-more"
                        >{$t("lighting.ftCard.moreModes", {
                          values: { count: g.in_use.length - 3 },
                        })}</span
                      >
                    {/if}
                  {:else if g.mode}
                    <span class="mode-pill" data-testid="ft-card-mode"
                      >{g.mode}</span
                    >
                  {:else}
                    <span class="item-meta" data-testid="ft-card-mode"
                      >{$t("lighting.ftCard.noDefault")}</span
                    >
                  {/if}
                </div>
              {:else if entry.referential}
                <!-- The channels only exist once the lighting system
                         expands the archive; "0 channels" would be a lie. -->
                <div class="item-meta">
                  {$t("lighting.fixtureTypeFromGdtf")}
                </div>
              {:else}
                <div class="item-meta">
                  {$t("lighting.channels", {
                    values: {
                      count: Object.keys(ft.channels).length,
                      names: Object.entries(ft.channels)
                        .sort(([, a], [, b]) => a - b)
                        .map(([n]) => n)
                        .join(", "),
                    },
                  })}
                </div>
              {/if}
              {#if ft.max_strobe_frequency}
                <div class="item-meta">
                  {$t("lighting.strobeMax", {
                    values: { freq: ft.max_strobe_frequency },
                  })}
                </div>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .sub-panel {
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

  .field-hint {
    font-size: 12px;
    color: var(--text-dim);
  }

  .field-row-3 {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 12px;
  }

  .subsection {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .subsection-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .list-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .list-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .item-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 8px;
  }

  .item-card {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px;
    cursor: pointer;
    transition: border-color 0.15s;
  }

  .item-card:hover {
    border-color: var(--border-focus);
  }

  /* A fixture from an archive: picture left, words right. */
  .item-card--gdtf {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr);
    gap: 12px;
    align-items: start;
  }

  .card-body {
    min-width: 0;
  }

  .card-thumb {
    width: 64px;
    height: 64px;
    border-radius: var(--radius);
    background: #0b0e13;
    object-fit: contain;
  }

  .card-thumb--none {
    display: grid;
    place-items: center;
    background: var(--bg-input);
    border: 1px dashed var(--border);
    color: var(--text-dim);
    font-size: 10px;
    text-align: center;
  }

  .card-modes {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
  }

  .mode-pill {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--accent-subtle);
    color: var(--accent);
    white-space: nowrap;
  }

  .item-card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 4px;
  }

  .item-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }

  .item-meta {
    font-size: 12px;
    color: var(--text-dim);
    margin-top: 2px;
  }

  .item-file {
    font-family: var(--mono);
  }

  .ext-badge {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1px 5px;
    margin-right: auto;
    margin-left: 6px;
  }

  .ext-choice {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-dim);
    margin-bottom: 8px;
  }

  .ext-choice select {
    width: auto;
    font-family: var(--mono);
  }

  .new-ft-choice {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-bottom: 8px;
  }

  .raw-textarea {
    min-height: 300px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-family: var(--mono);
    font-size: 14px;
    padding: 12px;
    resize: vertical;
    tab-size: 4;
    line-height: 1.5;
  }

  .raw-textarea:focus {
    border-color: var(--border-focus);
    outline: none;
  }

  .editor-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .editor-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .editor-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    margin: 0;
  }

  .editor-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .row-error {
    margin: 0;
    font-size: 12px;
    color: var(--red);
  }

  .channel-row :global([aria-invalid="true"]) {
    border-color: var(--red);
  }

  .channel-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .channel-name {
    flex: 1;
  }

  .channel-offset {
    width: 80px;
  }

  .save-msg {
    font-size: 13px;
    color: var(--green);
  }

  .save-error {
    color: var(--red);
  }

  .status-text {
    font-size: 14px;
    color: var(--text-dim);
    padding: 8px 0;
  }

  .error-text {
    color: var(--red);
  }

  .empty-state {
    text-align: center;
    padding: 32px 20px;
    color: var(--text-dim);
  }

  .empty-state p {
    margin-bottom: 4px;
    font-size: 14px;
  }

  .btn-sm {
    padding: 4px 8px;
    font-size: 12px;
  }

  @media (max-width: 600px) {
    .item-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
