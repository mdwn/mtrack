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
    saveFixtureType,
    deleteFixtureType,
    fetchVenues,
    saveVenue,
    deleteVenue,
    type FixtureTypeData,
    type VenueData,
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
  let fixtureTypes = $state<Record<string, FixtureTypeData>>({});
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

  // --- Venues state ---
  let venues = $state<Record<string, VenueData>>({});
  let venueLoading = $state(false);
  let venueError = $state("");
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
    }[]
  >([]);
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
    try {
      fixtureTypes = await fetchFixtureTypes(ftDir || undefined);
    } catch (e: any) {
      ftError = e.message;
    } finally {
      ftLoading = false;
    }
  }

  async function loadVenues() {
    venueLoading = true;
    venueError = "";
    try {
      venues = await fetchVenues(venueDir || undefined);
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

  function startEditFt(name: string) {
    const ft = fixtureTypes[name];
    editingFt = name;
    editFtName = name;
    editFtChannels = Object.entries(ft.channels)
      .sort(([, a], [, b]) => a - b)
      .map(([n, o]) => ({ name: n, offset: o }));
    editFtMaxStrobe =
      ft.max_strobe_frequency != null ? String(ft.max_strobe_frequency) : "";
    editFtMinStrobe =
      ft.min_strobe_frequency != null ? String(ft.min_strobe_frequency) : "";
    editFtStrobeDmxOffset =
      ft.strobe_dmx_offset != null ? String(ft.strobe_dmx_offset) : "";
    isNewFt = false;
  }

  function startNewFt() {
    editingFt = "__new__";
    editFtName = "";
    editFtChannels = [{ name: "dimmer", offset: 1 }];
    editFtMaxStrobe = "";
    editFtMinStrobe = "";
    editFtStrobeDmxOffset = "";
    isNewFt = true;
  }

  function cancelEditFt() {
    editingFt = null;
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
      }));
    isNewVenue = false;
  }

  function startNewVenue() {
    editingVenue = "__new__";
    editVenueName = "";
    editVenueFixtures = [];
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
      }));
    const newName = editVenueName.trim();
    const oldName = editingVenue !== "__new__" ? editingVenue : null;
    const isRename = oldName && oldName !== newName;
    venueSaving = true;
    venueMsg = "";
    try {
      await saveVenue(newName, { fixtures }, venueDir || undefined);
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
            <input
              id="ft-name"
              class="input"
              bind:value={editFtName}
              placeholder="e.g. RGBW_Par"
            />
          </div>

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
            <button class="btn btn-primary" onclick={startNewFt}
              >{$t("lighting.newFixtureType")}</button
            >
          </div>
        </div>
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
            {#each Object.entries(fixtureTypes).sort( ([a], [b]) => a.localeCompare(b), ) as [name, ft] (name)}
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
                  <button
                    class="btn btn-danger btn-sm"
                    onclick={(e) => {
                      e.stopPropagation();
                      removeFt(name);
                    }}>{$t("common.delete")}</button
                  >
                </div>
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
                  {#if Object.keys(v.groups).length > 0}
                    &middot; {$t("lighting.groupCount", {
                      values: { count: Object.keys(v.groups).length },
                    })}
                  {/if}
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
