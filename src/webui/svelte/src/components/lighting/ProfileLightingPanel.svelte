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
  import Tooltip from "../config/Tooltip.svelte";
  import TagInput from "../config/TagInput.svelte";
  import { fetchVenues, type VenueData } from "../../lib/api/config";

  interface Props {
    /** One profile's `dmx.lighting`. */
    lighting: any;
    onchange: () => void;
  }

  let { lighting = $bindable(), onchange }: Props = $props();

  // Venue names for the current_venue dropdown
  let venues = $state<Record<string, VenueData>>({});
  let venueDir = $derived(lighting.directories?.venues || "");
  let venueNames = $derived(Object.keys(venues).sort());

  async function loadVenues() {
    try {
      venues = (await fetchVenues(venueDir || undefined)).venues;
    } catch {
      // The field falls back to a text input when no venues are listed.
    }
  }

  $effect(() => {
    void venueDir;
    loadVenues();
  });

  const constraintTooltipKeys: Record<string, string> = {
    AllOf: "tooltips.lighting.allOf",
    AnyOf: "tooltips.lighting.anyOf",
    Prefer: "tooltips.lighting.prefer",
    MinCount: "tooltips.lighting.minCount",
    MaxCount: "tooltips.lighting.maxCount",
    FallbackTo: "tooltips.lighting.fallbackTo",
    AllowEmpty: "tooltips.lighting.allowEmpty",
  };

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

  /** Why the last rename was refused, by the field it was typed in. */
  let renameError = $state<Record<string, string>>({});

  /** A refused rename puts the old name back in the field and says why,
   *  instead of leaving the typed name showing over data that kept the
   *  old one. */
  function refuseRename(
    key: string,
    field: HTMLInputElement,
    oldName: string,
    newName: string,
  ): boolean {
    if (newName === oldName) return true;
    if (!newName) {
      field.value = oldName;
      renameError[key] = $t("lighting.renameRefused.empty", {
        values: { name: oldName },
      });
      return true;
    }
    return false;
  }

  function renameInlineFixture(
    oldName: string,
    newName: string,
    field: HTMLInputElement,
  ) {
    delete renameError[`fixture:${oldName}`];
    if (refuseRename(`fixture:${oldName}`, field, oldName, newName)) return;
    if (lighting.fixtures[newName]) {
      field.value = oldName;
      renameError[`fixture:${oldName}`] = $t("lighting.renameRefused.taken", {
        values: { name: newName },
      });
      return;
    }
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

  function renameGroup(
    oldName: string,
    newName: string,
    field: HTMLInputElement,
  ) {
    delete renameError[`group:${oldName}`];
    if (refuseRename(`group:${oldName}`, field, oldName, newName)) return;
    if (lighting.groups[newName]) {
      field.value = oldName;
      renameError[`group:${oldName}`] = $t("lighting.renameRefused.taken", {
        values: { name: newName },
      });
      return;
    }
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
          <label for="lighting-venues-dir">{$t("lighting.venuesDir")}</label>
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
            aria-invalid={!!renameError[`fixture:${name}`]}
            onchange={(e) =>
              renameInlineFixture(
                name,
                (e.target as HTMLInputElement).value.trim(),
                e.target as HTMLInputElement,
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
        {#if renameError[`fixture:${name}`]}
          <p class="rename-error" data-testid="rename-error">
            {renameError[`fixture:${name}`]}
          </p>
        {/if}
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
        <button class="btn btn-sm" onclick={addGroup}>{$t("common.add")}</button
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
                  aria-invalid={!!renameError[`group:${name}`]}
                  onchange={(e) =>
                    renameGroup(
                      name,
                      (e.target as HTMLInputElement).value.trim(),
                      e.target as HTMLInputElement,
                    )}
                />
                {#if renameError[`group:${name}`]}
                  <p class="rename-error" data-testid="rename-error">
                    {renameError[`group:${name}`]}
                  </p>
                {/if}
              </div>

              <div class="constraints-section">
                <div class="subsection-header">
                  <span class="field-label">{$t("lighting.constraints")}</span>
                  <button class="btn btn-sm" onclick={() => addConstraint(name)}
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
                    <Tooltip text={$t(constraintTooltipKeys[cType] ?? "")} />

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
                            parseInt((e.target as HTMLInputElement).value) || 0,
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

<style>
  .rename-error {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--red);
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

  .btn-sm {
    padding: 4px 8px;
    font-size: 12px;
  }

  @media (max-width: 600px) {
    .constraint-row {
      flex-wrap: wrap;
    }
    .constraint-type {
      width: 100%;
    }
    .fixture-name {
      width: 100%;
    }
  }
</style>
