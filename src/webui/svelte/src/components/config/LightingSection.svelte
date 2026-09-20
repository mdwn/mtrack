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
  import TagInput from "./TagInput.svelte";
  import {
    fetchFixtureTypes,
    fetchFixtureType,
    saveFixtureType,
    saveFixtureTypeText,
    deleteFixtureType,
    fetchVenues,
    saveVenue,
    deleteVenue,
    inspectGdtf,
    importGdtf,
    type FixtureTypeEntry,
    type VenueData,
    type VenueSource,
    type Vec3,
    type GdtfInspection,
    type GdtfImportReport,
  } from "../../lib/api/config";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { showConfirm } from "../../lib/dialog.svelte";
  import Tooltip from "./Tooltip.svelte";

  const constraintTooltipKeys: Record<string, string> = {
    AllOf: "tooltips.lighting.allOf",
    AnyOf: "tooltips.lighting.anyOf",
    Prefer: "tooltips.lighting.prefer",
    MinCount: "tooltips.lighting.minCount",
    MaxCount: "tooltips.lighting.maxCount",
    FallbackTo: "tooltips.lighting.fallbackTo",
    AllowEmpty: "tooltips.lighting.allowEmpty",
  };

  interface Props {
    lighting: any;
    onchange: () => void;
  }

  let { lighting = $bindable(), onchange }: Props = $props();

  // Sub-tab navigation
  type SubTab = "fixture_types" | "venues" | "profile";
  let activeSubTab = $state<SubTab>("fixture_types");

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

  // --- Venues state ---
  let venues = $state<Record<string, VenueData>>({});
  let venueLoading = $state(false);
  let venueError = $state("");
  /** Files in the venue directory that would not parse. Reported alongside the
   *  venues that did, since one bad file no longer empties the list — and a
   *  venue quietly missing is the only other signal. */
  let venueFileErrors = $state<
    import("../../lib/api/config").LightingFileError[]
  >([]);
  let ftFileErrors = $state<import("../../lib/api/config").LightingFileError[]>(
    [],
  );
  let venueSaving = $state(false);
  let venueMsg = $state("");
  let editingVenue = $state<string | null>(null);
  let editVenueName = $state("");
  let editVenueFixtures = $state<
    {
      name: string;
      fixture_type: string;
      universe: number;
      start_channel: number;
      tags: string[];
      /** Carried through the edit untouched: positions are edited on the
       *  stage view, not here, and a save must not drop them. */
      position?: Vec3 | null;
      rotation?: Vec3 | null;
    }[]
  >([]);
  /** Likewise carried through: the venue's focus points and MVR provenance. */
  let editVenueFocusPoints = $state<Record<string, Vec3>>({});
  let editVenueSource = $state<VenueSource | null>(null);
  let isNewVenue = $state(false);

  // Effective directories (from profile config or defaults)
  let ftDir = $derived(lighting.directories?.fixture_types || "");
  let venueDir = $derived(lighting.directories?.venues || "");

  // Available fixture type names for venue fixture dropdowns
  let fixtureTypeNames = $derived(Object.keys(fixtureTypes).sort());
  // Available venue names for current_venue dropdown
  let venueNames = $derived(Object.keys(venues).sort());

  async function loadFixtureTypes() {
    ftLoading = true;
    ftError = "";
    ftFileErrors = [];
    try {
      const result = await fetchFixtureTypes(ftDir || undefined);
      fixtureTypes = result.fixtureTypes;
      ftFileErrors = result.errors;
    } catch (e: any) {
      ftError = e.message;
    } finally {
      ftLoading = false;
    }
  }

  async function loadVenues() {
    venueLoading = true;
    venueError = "";
    venueFileErrors = [];
    try {
      const result = await fetchVenues(venueDir || undefined);
      venues = result.venues;
      venueFileErrors = result.errors;
    } catch (e: any) {
      venueError = e.message;
    } finally {
      venueLoading = false;
    }
  }

  $effect(() => {
    // Re-load when directories change
    void ftDir;
    loadFixtureTypes();
  });

  $effect(() => {
    void venueDir;
    loadVenues();
  });

  // --- Fixture Type editing ---

  async function startEditFt(name: string) {
    const entry = fixtureTypes[name];
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

  // GDTF import flow: pick a file → inspect (modes) → pick a mode → import.
  let gdtfFileInput = $state<HTMLInputElement | null>(null);
  let gdtfFile = $state<File | null>(null);
  let gdtfInspection = $state<GdtfInspection | null>(null);
  let gdtfMode = $state("");
  let gdtfName = $state("");
  let gdtfBusy = $state(false);
  let gdtfError = $state("");
  let gdtfReport = $state<GdtfImportReport | null>(null);

  function resetGdtf() {
    gdtfFile = null;
    gdtfInspection = null;
    gdtfMode = "";
    gdtfName = "";
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
      gdtfMode = gdtfInspection.modes[0]?.name ?? "";
      gdtfName = gdtfInspection.fixture;
    } catch (err) {
      gdtfError = err instanceof Error ? err.message : String(err);
      gdtfFile = null;
    } finally {
      gdtfBusy = false;
    }
  }

  async function runGdtfImport() {
    if (!gdtfFile || !gdtfMode) return;
    gdtfBusy = true;
    gdtfError = "";
    try {
      gdtfReport = await importGdtf(
        gdtfFile,
        gdtfMode,
        gdtfName.trim() || undefined,
      );
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
    const channels: Record<string, number> = {};
    for (const ch of editFtChannels) {
      if (!ch.name.trim()) continue;
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

  async function saveFtText() {
    // The text is the file, so the name it declares is the name to save
    // under: a URL naming anything else would write a file the panel could
    // never reach again, and the server refuses that outright.
    const newName = editFtDslName;
    if (!newName) {
      ftMsg = get(t)("lighting.fixtureTypeNoName");
      return;
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
      editingFt = null;
      ftMsg = get(t)("common.saved");
      setTimeout(() => (ftMsg = ""), 2000);
    } catch (e: any) {
      ftMsg = e.message;
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

  // --- Venue editing ---

  function startEditVenue(name: string) {
    const v = venues[name];
    editingVenue = name;
    editVenueName = name;
    editVenueFixtures = Object.values(v.fixtures)
      .sort(
        (a, b) => a.universe - b.universe || a.start_channel - b.start_channel,
      )
      .map((f) => ({
        name: f.name,
        fixture_type: f.fixture_type,
        universe: f.universe,
        start_channel: f.start_channel,
        tags: [...f.tags],
        position: f.position ?? null,
        rotation: f.rotation ?? null,
      }));
    editVenueFocusPoints = { ...(v.focus_points ?? {}) };
    editVenueSource = v.source ?? null;
    isNewVenue = false;
  }

  function startNewVenue() {
    editingVenue = "__new__";
    editVenueName = "";
    editVenueFixtures = [];
    editVenueFocusPoints = {};
    editVenueSource = null;
    isNewVenue = true;
  }

  function cancelEditVenue() {
    editingVenue = null;
  }

  function addVenueFixture() {
    editVenueFixtures = [
      ...editVenueFixtures,
      {
        name: "",
        fixture_type: fixtureTypeNames[0] ?? "",
        universe: 1,
        start_channel: 1,
        tags: [],
      },
    ];
  }

  function removeVenueFixture(i: number) {
    editVenueFixtures = editVenueFixtures.filter((_, idx) => idx !== i);
  }

  async function saveVenueData() {
    if (!editVenueName.trim()) {
      venueMsg = get(t)("lighting.nameRequired");
      return;
    }
    const fixtures = editVenueFixtures
      .filter((f) => f.name.trim() && f.fixture_type.trim())
      .map((f) => ({
        name: f.name.trim(),
        fixture_type: f.fixture_type.trim(),
        universe: f.universe,
        start_channel: f.start_channel,
        tags: f.tags,
        position: f.position ?? null,
        rotation: f.rotation ?? null,
      }));
    const newName = editVenueName.trim();
    const oldName = editingVenue !== "__new__" ? editingVenue : null;
    const isRename = oldName && oldName !== newName;
    venueSaving = true;
    venueMsg = "";
    try {
      await saveVenue(
        newName,
        {
          fixtures,
          focus_points: editVenueFocusPoints,
          source: editVenueSource,
        },
        venueDir || undefined,
      );
      if (isRename) {
        await deleteVenue(oldName, venueDir || undefined);
      }
      await loadVenues();
      editingVenue = null;
      venueMsg = get(t)("common.saved");
      setTimeout(() => (venueMsg = ""), 2000);
    } catch (e: any) {
      venueMsg = e.message;
    } finally {
      venueSaving = false;
    }
  }

  async function removeVenue(name: string) {
    if (
      !(await showConfirm(
        get(t)("lighting.deleteVenue", { values: { name } }),
        { danger: true },
      ))
    )
      return;
    try {
      await deleteVenue(name, venueDir || undefined);
      await loadVenues();
    } catch (e: any) {
      venueMsg = e.message;
    }
  }

  // --- Profile settings helpers ---

  function ensureDirectories() {
    if (!lighting.directories) lighting.directories = {};
    return lighting.directories;
  }

  function setDirectory(key: string, value: string) {
    if (value) {
      ensureDirectories()[key] = value;
    } else {
      if (lighting.directories) {
        delete lighting.directories[key];
        if (Object.keys(lighting.directories).length === 0) {
          delete lighting.directories;
        }
      }
    }
    onchange();
  }

  function setVenueSelection(value: string) {
    if (value) {
      lighting.current_venue = value;
    } else {
      delete lighting.current_venue;
    }
    onchange();
  }

  let inlineFixtureEntries = $derived(
    lighting.fixtures
      ? (Object.entries(lighting.fixtures) as [string, string][])
      : [],
  );

  function addInlineFixture() {
    if (!lighting.fixtures) lighting.fixtures = {};
    let name = "new_fixture";
    let i = 1;
    while (lighting.fixtures[name]) {
      name = `new_fixture_${i++}`;
    }
    lighting.fixtures[name] = "FixtureType @ 1:1";
    onchange();
  }

  function removeInlineFixture(name: string) {
    delete lighting.fixtures[name];
    if (Object.keys(lighting.fixtures).length === 0) {
      delete lighting.fixtures;
    }
    onchange();
  }

  function renameInlineFixture(oldName: string, newName: string) {
    if (!newName || newName === oldName) return;
    if (lighting.fixtures[newName]) return;
    const value = lighting.fixtures[oldName];
    delete lighting.fixtures[oldName];
    lighting.fixtures[newName] = value;
    onchange();
  }

  function setInlineFixtureValue(name: string, value: string) {
    lighting.fixtures[name] = value;
    onchange();
  }

  // --- Logical Groups ---

  let groupEntries = $derived(
    lighting.groups ? (Object.entries(lighting.groups) as [string, any][]) : [],
  );

  let expandedGroups: Record<string, boolean> = $state({});

  function addGroup() {
    if (!lighting.groups) lighting.groups = {};
    let name = "new_group";
    let i = 1;
    while (lighting.groups[name]) {
      name = `new_group_${i++}`;
    }
    lighting.groups[name] = { name, constraints: [] };
    expandedGroups[name] = true;
    onchange();
  }

  function removeGroup(name: string) {
    delete lighting.groups[name];
    if (Object.keys(lighting.groups).length === 0) {
      delete lighting.groups;
    }
    delete expandedGroups[name];
    onchange();
  }

  function renameGroup(oldName: string, newName: string) {
    if (!newName || newName === oldName) return;
    if (lighting.groups[newName]) return;
    const group = lighting.groups[oldName];
    delete lighting.groups[oldName];
    group.name = newName;
    lighting.groups[newName] = group;
    expandedGroups[newName] = expandedGroups[oldName];
    delete expandedGroups[oldName];
    onchange();
  }

  type ConstraintType =
    | "AllOf"
    | "AnyOf"
    | "Prefer"
    | "MinCount"
    | "MaxCount"
    | "FallbackTo"
    | "AllowEmpty";

  const constraintTypes: { value: ConstraintType; labelKey: string }[] = [
    { value: "AllOf", labelKey: "lighting.allOfTags" },
    { value: "AnyOf", labelKey: "lighting.anyOfTags" },
    { value: "Prefer", labelKey: "lighting.preferTags" },
    { value: "MinCount", labelKey: "lighting.minCount" },
    { value: "MaxCount", labelKey: "lighting.maxCount" },
    { value: "FallbackTo", labelKey: "lighting.fallbackTo" },
    { value: "AllowEmpty", labelKey: "lighting.allowEmptyLabel" },
  ];

  function getConstraintType(constraint: any): ConstraintType {
    if (typeof constraint === "object") {
      return Object.keys(constraint)[0] as ConstraintType;
    }
    return "AllOf";
  }

  function getConstraintValue(constraint: any): any {
    if (typeof constraint === "object") {
      return constraint[Object.keys(constraint)[0]];
    }
    return [];
  }

  function makeConstraint(type: ConstraintType): any {
    switch (type) {
      case "AllOf":
      case "AnyOf":
      case "Prefer":
        return { [type]: [] };
      case "MinCount":
      case "MaxCount":
        return { [type]: 1 };
      case "FallbackTo":
        return { [type]: "" };
      case "AllowEmpty":
        return { [type]: true };
    }
  }

  function addConstraint(groupName: string) {
    lighting.groups[groupName].constraints.push(makeConstraint("AllOf"));
    onchange();
  }

  function removeConstraint(groupName: string, index: number) {
    lighting.groups[groupName].constraints.splice(index, 1);
    onchange();
  }

  function setConstraintType(
    groupName: string,
    index: number,
    newType: ConstraintType,
  ) {
    lighting.groups[groupName].constraints[index] = makeConstraint(newType);
    onchange();
  }

  function setConstraintValue(groupName: string, index: number, value: any) {
    const constraint = lighting.groups[groupName].constraints[index];
    const type = getConstraintType(constraint);
    lighting.groups[groupName].constraints[index] = { [type]: value };
    onchange();
  }
</script>

<div class="lighting-section">
  <!-- Sub-tab navigation -->
  <div class="sub-tab-bar">
    <button
      class="sub-tab"
      class:active={activeSubTab === "fixture_types"}
      onclick={() => (activeSubTab = "fixture_types")}
      >{$t("lighting.fixtureTypes")}</button
    >
    <button
      class="sub-tab"
      class:active={activeSubTab === "venues"}
      onclick={() => (activeSubTab = "venues")}>{$t("lighting.venues")}</button
    >
    <button
      class="sub-tab"
      class:active={activeSubTab === "profile"}
      onclick={() => (activeSubTab = "profile")}
      >{$t("lighting.profileSettings")}</button
    >
  </div>

  <!-- Fixture Types sub-tab -->
  {#if activeSubTab === "fixture_types"}
    <div class="sub-panel">
      {#if editingFt}
        <!-- Fixture Type Editor -->
        <div class="editor-form">
          <div class="editor-header">
            <h4 class="editor-title">
              {isNewFt
                ? $t("lighting.newFixtureType")
                : $t("lighting.editFixtureType", {
                    values: { name: editingFt },
                  })}
            </h4>
            <div class="editor-actions">
              {#if ftMsg}
                <span
                  class="save-msg"
                  class:save-error={ftMsg !== get(t)("common.saved")}
                  >{ftMsg}</span
                >
              {/if}
              <button class="btn" onclick={cancelEditFt}
                >{$t("common.cancel")}</button
              >
              <button
                class="btn btn-primary"
                onclick={saveFt}
                disabled={ftSaving}
              >
                {ftSaving ? $t("common.saving") : $t("common.save")}
              </button>
            </div>
          </div>

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
                <div class="channel-row">
                  <input
                    class="input channel-name"
                    placeholder={$t("lighting.channelName")}
                    bind:value={ch.name}
                  />
                  <input
                    class="input channel-offset"
                    type="number"
                    min="1"
                    placeholder={$t("lighting.offset")}
                    bind:value={ch.offset}
                  />
                  <button
                    class="btn btn-danger btn-sm"
                    onclick={() => removeFtChannel(i)}>X</button
                  >
                </div>
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
                class:save-error={ftMsg !== get(t)("common.saved")}
                >{ftMsg}</span
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
          <div class="editor-form" data-testid="gdtf-mode-picker">
            <div class="editor-header">
              <h4 class="editor-title">
                {gdtfInspection.fixture} — {gdtfInspection.manufacturer}
              </h4>
              <div class="editor-actions">
                <button class="btn" onclick={resetGdtf}
                  >{$t("common.cancel")}</button
                >
                <button
                  class="btn btn-primary"
                  data-testid="gdtf-import-confirm"
                  onclick={runGdtfImport}
                  disabled={gdtfBusy || !gdtfMode}
                >
                  {gdtfBusy ? $t("common.saving") : $t("lighting.importGdtf")}
                </button>
              </div>
            </div>
            <div class="field">
              <label for="gdtf-mode">{$t("lighting.gdtfMode")}</label>
              <select id="gdtf-mode" class="input" bind:value={gdtfMode}>
                {#each gdtfInspection.modes as mode (mode.name)}
                  <option value={mode.name}>
                    {mode.name} ({mode.channel_count} ch, footprint {mode.footprint})
                  </option>
                {/each}
              </select>
            </div>
            <div class="field">
              <label for="gdtf-name">{$t("lighting.name")}</label>
              <input
                id="gdtf-name"
                class="input"
                bind:value={gdtfName}
                placeholder={gdtfInspection.fixture}
              />
            </div>
          </div>
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
                <button class="btn" onclick={resetGdtf}
                  >{$t("common.close")}</button
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
                role="button"
                tabindex="0"
                onclick={() => startEditFt(name)}
                onkeydown={(e) => {
                  if (e.key === "Enter") startEditFt(name);
                }}
              >
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
                {#if entry.referential}
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
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  {/if}

  <!-- Venues sub-tab -->
  {#if activeSubTab === "venues"}
    <div class="sub-panel">
      {#if editingVenue}
        <!-- Venue Editor -->
        <div class="editor-form">
          <div class="editor-header">
            <h4 class="editor-title">
              {isNewVenue
                ? $t("lighting.newVenue")
                : $t("lighting.editVenue", { values: { name: editingVenue } })}
            </h4>
            <div class="editor-actions">
              {#if venueMsg}
                <span
                  class="save-msg"
                  class:save-error={venueMsg !== get(t)("common.saved")}
                  >{venueMsg}</span
                >
              {/if}
              <button class="btn" onclick={cancelEditVenue}
                >{$t("common.cancel")}</button
              >
              <button
                class="btn btn-primary"
                onclick={saveVenueData}
                disabled={venueSaving}
              >
                {venueSaving ? $t("common.saving") : $t("common.save")}
              </button>
            </div>
          </div>

          <div class="field">
            <label for="venue-name">{$t("lighting.name")}</label>
            <input
              id="venue-name"
              class="input"
              bind:value={editVenueName}
              placeholder="e.g. main_stage"
            />
          </div>

          <div class="subsection">
            <div class="subsection-header">
              <span class="field-label">{$t("lighting.fixtures")}</span>
              <button class="btn btn-sm" onclick={addVenueFixture}
                >{$t("lighting.addFixture")}</button
              >
            </div>

            {#each editVenueFixtures as fix, i (i)}
              <div class="venue-fixture-card">
                <div class="venue-fixture-row">
                  <input
                    class="input"
                    placeholder={$t("lighting.fixtureName")}
                    bind:value={fix.name}
                  />
                  {#if fixtureTypeNames.length > 0}
                    <select class="input" bind:value={fix.fixture_type}>
                      <option value="">{$t("lighting.selectType")}</option>
                      {#each fixtureTypeNames as ftName (ftName)}
                        <option value={ftName}>{ftName}</option>
                      {/each}
                    </select>
                  {:else}
                    <input
                      class="input"
                      placeholder={$t("lighting.fixtureType")}
                      bind:value={fix.fixture_type}
                    />
                  {/if}
                  <button
                    class="btn btn-danger btn-sm"
                    onclick={() => removeVenueFixture(i)}>X</button
                  >
                </div>
                <div class="venue-fixture-row">
                  <div class="field compact-field">
                    <label for={`fix-universe-${i}`}
                      >{$t("lighting.universe")}</label
                    >
                    <input
                      id={`fix-universe-${i}`}
                      class="input"
                      type="number"
                      min="1"
                      bind:value={fix.universe}
                    />
                  </div>
                  <div class="field compact-field">
                    <label for={`fix-channel-${i}`}
                      >{$t("lighting.channelLabel")}</label
                    >
                    <input
                      id={`fix-channel-${i}`}
                      class="input"
                      type="number"
                      min="1"
                      bind:value={fix.start_channel}
                    />
                  </div>
                  <div class="field compact-field" style="flex: 2;">
                    <label for={`fix-tags-${i}`}
                      >{$t("lighting.tags")}<Tooltip
                        text={$t("tooltips.lighting.fixtureTags")}
                      /></label
                    >
                    <TagInput
                      tags={fix.tags}
                      onchange={(tags) => (editVenueFixtures[i].tags = tags)}
                      placeholder={$t("lighting.tagPlaceholder")}
                    />
                  </div>
                </div>
              </div>
            {/each}
          </div>
        </div>
      {:else}
        <!-- Venue List -->
        <div class="list-header">
          <span class="field-hint"
            >{$t("lighting.venueCount", {
              values: { count: Object.keys(venues).length },
            })}</span
          >
          <div class="list-actions">
            {#if venueMsg}
              <span
                class="save-msg"
                class:save-error={venueMsg !== get(t)("common.saved")}
                >{venueMsg}</span
              >
            {/if}
            <button class="btn" onclick={loadVenues} disabled={venueLoading}
              >{$t("common.refresh")}</button
            >
            <button class="btn btn-primary" onclick={startNewVenue}
              >{$t("lighting.newVenue")}</button
            >
          </div>
        </div>
        {#if venueFileErrors.length > 0}
          <ul class="file-errors" data-testid="venue-file-errors">
            {#each venueFileErrors as fe (fe.file)}
              <li class="status-text error-text">{fe.file}: {fe.error}</li>
            {/each}
          </ul>
        {/if}
        {#if venueLoading}
          <p class="status-text">{$t("common.loading")}</p>
        {:else if venueError}
          <p class="status-text error-text">{venueError}</p>
        {:else if Object.keys(venues).length === 0}
          <div class="empty-state">
            <p>{$t("lighting.noVenues")}</p>
            <p>
              {$t("lighting.venueHint")}
            </p>
          </div>
        {:else}
          <div class="item-grid">
            {#each Object.entries(venues).sort( ([a], [b]) => a.localeCompare(b), ) as [name, v] (name)}
              <div
                class="item-card"
                role="button"
                tabindex="0"
                onclick={() => startEditVenue(name)}
                onkeydown={(e) => {
                  if (e.key === "Enter") startEditVenue(name);
                }}
              >
                <div class="item-card-header">
                  <span class="item-name">{name}</span>
                  <button
                    class="btn btn-danger btn-sm"
                    onclick={(e) => {
                      e.stopPropagation();
                      removeVenue(name);
                    }}>{$t("common.delete")}</button
                  >
                </div>
                <div class="item-meta">
                  {$t("lighting.fixtureCount", {
                    values: { count: Object.keys(v.fixtures).length },
                  })}
                </div>
                <div class="item-meta">
                  {Object.values(v.fixtures)
                    .map((f) => f.name)
                    .sort()
                    .join(", ")}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  {/if}

  <!-- Profile Settings sub-tab -->
  {#if activeSubTab === "profile"}
    <div class="sub-panel">
      <div class="section-fields">
        <!-- Directories -->
        <div class="subsection">
          <h4 class="subsection-title">{$t("lighting.directories")}</h4>
          <span class="field-hint">
            {$t("lighting.directoriesHint")}
          </span>
          <div class="field-row-2">
            <div class="field">
              <label for="lighting-fixture-types-dir"
                >{$t("lighting.fixtureTypesDir")}</label
              >
              <input
                id="lighting-fixture-types-dir"
                class="input"
                type="text"
                placeholder={$t("lighting.fixtureTypesDirPlaceholder")}
                value={lighting.directories?.fixture_types ?? ""}
                onchange={(e) =>
                  setDirectory(
                    "fixture_types",
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
            </div>
            <div class="field">
              <label for="lighting-venues-dir">{$t("lighting.venuesDir")}</label
              >
              <input
                id="lighting-venues-dir"
                class="input"
                type="text"
                placeholder={$t("lighting.venuesDirPlaceholder")}
                value={lighting.directories?.venues ?? ""}
                onchange={(e) =>
                  setDirectory(
                    "venues",
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
            </div>
          </div>
        </div>

        <!-- Current Venue -->
        <div class="field">
          <label for="lighting-venue"
            >{$t("lighting.currentVenue")}<Tooltip
              text={$t("tooltips.lighting.currentVenue")}
            /></label
          >
          {#if venueNames.length > 0}
            <select
              id="lighting-venue"
              class="input"
              value={lighting.current_venue ?? ""}
              onchange={(e) =>
                setVenueSelection((e.target as HTMLSelectElement).value)}
            >
              <option value="">{$t("lighting.noneVenue")}</option>
              {#each venueNames as vn (vn)}
                <option value={vn}>{vn}</option>
              {/each}
            </select>
          {:else}
            <input
              id="lighting-venue"
              class="input"
              type="text"
              placeholder="e.g. main_stage"
              value={lighting.current_venue ?? ""}
              onchange={(e) =>
                setVenueSelection((e.target as HTMLInputElement).value.trim())}
            />
          {/if}
          <span class="field-hint">{$t("lighting.venueHintField")}</span>
        </div>

        <!-- Inline Fixtures -->
        <div class="subsection">
          <div class="subsection-header">
            <h4 class="subsection-title">
              {$t("lighting.inlineFixtures")}<Tooltip
                text={$t("tooltips.lighting.inlineFixtures")}
              />
            </h4>
            <button class="btn btn-sm" onclick={addInlineFixture}
              >{$t("common.add")}</button
            >
          </div>
          <span class="field-hint">
            {$t("lighting.inlineFixturesHint")}
          </span>
          {#each inlineFixtureEntries as [name, value] (name)}
            <div class="fixture-row">
              <input
                class="input fixture-name"
                value={name}
                placeholder="Name"
                onchange={(e) =>
                  renameInlineFixture(
                    name,
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
              <input
                class="input fixture-value"
                {value}
                placeholder="FixtureType @ 1:1"
                onchange={(e) =>
                  setInlineFixtureValue(
                    name,
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
              <button
                class="btn btn-danger btn-sm"
                onclick={() => removeInlineFixture(name)}>X</button
              >
            </div>
          {/each}
        </div>

        <!-- Logical Groups -->
        <div class="subsection">
          <div class="subsection-header">
            <h4 class="subsection-title">
              {$t("lighting.logicalGroups")}<Tooltip
                text={$t("tooltips.lighting.logicalGroups")}
              />
            </h4>
            <button class="btn btn-sm" onclick={addGroup}
              >{$t("common.add")}</button
            >
          </div>
          <span class="field-hint">
            {$t("lighting.logicalGroupsHint")}
          </span>

          {#each groupEntries as [name, group] (name)}
            <div class="group-card">
              <div
                class="group-header"
                onclick={() => (expandedGroups[name] = !expandedGroups[name])}
                onkeydown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    expandedGroups[name] = !expandedGroups[name];
                  }
                }}
                role="button"
                tabindex="0"
              >
                <span class="group-name">{name}</span>
                <div class="group-controls">
                  <span class="constraint-count"
                    >{$t("lighting.constraintCount", {
                      values: { count: group.constraints?.length ?? 0 },
                    })}</span
                  >
                  <button
                    class="btn btn-danger btn-sm"
                    onclick={(e) => {
                      e.stopPropagation();
                      removeGroup(name);
                    }}>X</button
                  >
                  <span class="collapse-icon"
                    >{expandedGroups[name] ? "-" : "+"}</span
                  >
                </div>
              </div>

              {#if expandedGroups[name]}
                <div class="group-body">
                  <div class="field">
                    <label for={`group-name-${name}`}
                      >{$t("lighting.groupName")}</label
                    >
                    <input
                      id={`group-name-${name}`}
                      class="input"
                      value={name}
                      onchange={(e) =>
                        renameGroup(
                          name,
                          (e.target as HTMLInputElement).value.trim(),
                        )}
                    />
                  </div>

                  <div class="constraints-section">
                    <div class="subsection-header">
                      <span class="field-label"
                        >{$t("lighting.constraints")}</span
                      >
                      <button
                        class="btn btn-sm"
                        onclick={() => addConstraint(name)}
                        >{$t("common.add")}</button
                      >
                    </div>

                    {#each group.constraints ?? [] as constraint, ci (ci)}
                      {@const cType = getConstraintType(constraint)}
                      {@const cValue = getConstraintValue(constraint)}
                      <div class="constraint-row">
                        <select
                          class="input constraint-type"
                          value={cType}
                          onchange={(e) =>
                            setConstraintType(
                              name,
                              ci,
                              (e.target as HTMLSelectElement)
                                .value as ConstraintType,
                            )}
                        >
                          {#each constraintTypes as ct (ct.value)}
                            <option value={ct.value}>{$t(ct.labelKey)}</option>
                          {/each}
                        </select>
                        <Tooltip
                          text={$t(constraintTooltipKeys[cType] ?? "")}
                        />

                        {#if cType === "AllOf" || cType === "AnyOf" || cType === "Prefer"}
                          <div class="constraint-value">
                            <TagInput
                              tags={cValue ?? []}
                              onchange={(tags) =>
                                setConstraintValue(name, ci, tags)}
                              placeholder={$t("lighting.tagPlaceholder")}
                            />
                          </div>
                        {:else if cType === "MinCount" || cType === "MaxCount"}
                          <input
                            class="input constraint-value"
                            type="number"
                            min="0"
                            value={cValue}
                            onchange={(e) =>
                              setConstraintValue(
                                name,
                                ci,
                                parseInt(
                                  (e.target as HTMLInputElement).value,
                                ) || 0,
                              )}
                          />
                        {:else if cType === "FallbackTo"}
                          <input
                            class="input constraint-value"
                            placeholder={$t("lighting.groupNamePlaceholder")}
                            value={cValue}
                            onchange={(e) =>
                              setConstraintValue(
                                name,
                                ci,
                                (e.target as HTMLInputElement).value.trim(),
                              )}
                          />
                        {:else if cType === "AllowEmpty"}
                          <label class="constraint-check">
                            <input
                              type="checkbox"
                              checked={cValue === true}
                              onchange={(e) =>
                                setConstraintValue(
                                  name,
                                  ci,
                                  (e.target as HTMLInputElement).checked,
                                )}
                            />
                            {$t("lighting.allowEmpty")}
                          </label>
                        {/if}

                        <button
                          class="btn btn-danger btn-sm"
                          onclick={() => removeConstraint(name, ci)}>X</button
                        >
                      </div>
                    {/each}
                  </div>
                </div>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .lighting-section {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .sub-tab-bar {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border);
    margin-bottom: 16px;
  }
  .sub-tab {
    padding: 8px 14px;
    font-size: 13px;
    font-weight: 500;
    font-family: var(--sans);
    color: var(--text-dim);
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
    transition:
      color 0.15s,
      border-color 0.15s;
  }
  .sub-tab:hover {
    color: var(--text);
  }
  .sub-tab.active {
    color: var(--text);
    border-bottom-color: var(--text-muted);
  }
  .sub-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 16px;
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
  .subsection {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .subsection-title {
    font-size: 13px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
    margin: 0;
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
  .venue-fixture-card {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .venue-fixture-row {
    display: flex;
    gap: 8px;
    align-items: end;
  }
  .venue-fixture-row .input {
    flex: 1;
  }
  .compact-field {
    flex: 1;
    gap: 2px !important;
  }
  .compact-field label {
    font-size: 11px !important;
  }
  .fixture-row {
    display: flex;
    gap: 8px;
  }
  .fixture-name {
    width: 160px;
    flex-shrink: 0;
  }
  .fixture-value {
    flex: 1;
  }
  .group-card {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .group-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 12px;
    cursor: pointer;
    transition: background 0.15s;
  }
  .group-header:hover {
    background: var(--bg-card-hover);
  }
  .group-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  .group-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .constraint-count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .collapse-icon {
    font-family: var(--mono);
    font-size: 15px;
    color: var(--text-dim);
    width: 16px;
    text-align: center;
  }
  .group-body {
    padding: 12px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .constraints-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .constraint-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .constraint-type {
    width: 160px;
    flex-shrink: 0;
  }
  .constraint-value {
    flex: 1;
  }
  .constraint-check {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    color: var(--text-muted);
    cursor: pointer;
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
    .field-row-2,
    .field-row-3 {
      grid-template-columns: 1fr;
    }
    .constraint-row {
      flex-wrap: wrap;
    }
    .constraint-type {
      width: 100%;
    }
    .fixture-row,
    .venue-fixture-row {
      flex-wrap: wrap;
    }
    .fixture-name {
      width: 100%;
    }
    .item-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
