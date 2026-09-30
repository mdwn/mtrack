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
  import TagInput from "../config/TagInput.svelte";
  import MvrExportDialog from "./MvrExportDialog.svelte";
  import {
    fetchFixtureTypes,
    fetchVenues,
    saveVenue,
    deleteVenue,
    type FixtureTypeEntry,
    type VenueData,
    type VenueSource,
    type Vec3,
    type LightingFileError,
  } from "../../lib/api/config";

  interface Props {
    /** Directory overrides (a profile's `lighting.directories`). */
    fixtureTypesDir?: string;
    venuesDir?: string;
  }

  let { fixtureTypesDir = "", venuesDir = "" }: Props = $props();
  let ftDir = $derived(fixtureTypesDir);
  let venueDir = $derived(venuesDir);

  // Fixture types, for the type dropdown on a venue's fixtures.
  let fixtureTypes = $state<Record<string, FixtureTypeEntry>>({});
  // --- Venues state ---
  let venues = $state<Record<string, VenueData>>({});
  let venueLoading = $state(false);
  let venueError = $state("");
  /** Files in the venue directory that would not parse. Reported alongside the
   *  venues that did, since one bad file no longer empties the list — and a
   *  venue quietly missing is the only other signal. */
  let venueFileErrors = $state<LightingFileError[]>([]);
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
  /** The venue whose "Export an MVR" dialog is open. */
  let exportingVenue = $state<string | null>(null);

  // Available fixture type names for venue fixture dropdowns
  let fixtureTypeNames = $derived(Object.keys(fixtureTypes).sort());

  async function loadFixtureTypes() {
    try {
      fixtureTypes = (await fetchFixtureTypes(ftDir || undefined)).fixtureTypes;
    } catch {
      // The dropdown falls back to a text input when no types are listed.
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
    void ftDir;
    loadFixtureTypes();
  });

  $effect(() => {
    void venueDir;
    loadVenues();
  });

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
</script>

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
        <a class="btn" href="#/lighting/import" data-testid="venues-import-mvr"
          >{$t("lighting.mvr.import.button")}</a
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
              <span class="item-actions">
                <button
                  class="btn btn-sm"
                  data-testid="venue-export-mvr-{name}"
                  onclick={(e) => {
                    e.stopPropagation();
                    exportingVenue = name;
                  }}>{$t("lighting.mvr.export.button")}</button
                >
                <button
                  class="btn btn-danger btn-sm"
                  onclick={(e) => {
                    e.stopPropagation();
                    removeVenue(name);
                  }}>{$t("common.delete")}</button
                >
              </span>
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

{#if exportingVenue}
  <MvrExportDialog
    venue={exportingVenue}
    fixtureTypesDir={ftDir}
    venuesDir={venueDir}
    onclose={() => (exportingVenue = null)}
    onchanged={loadVenues}
  />
{/if}

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

  .item-card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 4px;
  }

  .item-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  a.btn {
    text-decoration: none;
    display: inline-flex;
    align-items: center;
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
