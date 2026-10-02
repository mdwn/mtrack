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
  import { fixtureTypeChanges, venueChanges } from "../../lib/lighting/changes";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { showConfirm } from "../../lib/dialog.svelte";
  import Tooltip from "../config/Tooltip.svelte";
  import TagInput from "../config/TagInput.svelte";
  import MvrExportDialog from "./MvrExportDialog.svelte";
  import { venueStore } from "../../lib/ws/stores";
  import { collisions, lastAddress, nextPatch } from "../../lib/lighting/patch";
  import { isRefused } from "../../lib/lighting/modes";
  import { untrack } from "svelte";
  import { lightingHref } from "../../lib/lightingRoute";
  import { guardUnsaved } from "../../lib/dirtyGuard";
  import {
    nextFixtureName,
    rowProblems,
    type RowProblem,
  } from "../../lib/lighting/venueRows";
  import {
    ConflictError,
    fetchFixtureTypeGdtf,
    fetchFixtureTypes,
    fetchVenues,
    saveVenue,
    deleteVenue,
    type FixtureTypeEntry,
    type FixtureTypeGdtf,
    type VenueData,
    type VenueSource,
    type Vec3,
    type LightingFileError,
  } from "../../lib/api/config";

  interface Props {
    /** Directory overrides (a profile's `lighting.directories`). */
    fixtureTypesDir?: string;
    venuesDir?: string;
    /** The venue picked in the list (the address's `#/lighting/venues/<name>`);
     *  the plot on this page shows it. Null leaves the plot on the current
     *  venue, live. */
    selected?: string | null;
    /** `?edit`: the selected venue's form is open. */
    edit?: boolean;
    /** `?new=venue`: a new venue's form is open. */
    creating?: string | null;
    /** `?view=3d`: the stage card shows 3D; kept across this panel's moves. */
    view?: "plot" | "3d";
  }

  let {
    fixtureTypesDir = "",
    venuesDir = "",
    selected = null,
    edit = false,
    creating = null,
    view = "plot",
  }: Props = $props();

  // The selection and an open form are the address, so the section's link,
  // Back and a reload land where they say; every action navigates.
  const go = (hash: string) => {
    if (window.location.hash !== hash) window.location.hash = hash;
  };
  /** The stage card's view travels with the venue: another venue, or its
   *  form, keeps 3D showing when it was. */
  const keep = (extra: Record<string, string> = {}) =>
    view === "3d" ? { ...extra, view: "3d" } : extra;
  const select = (name: string | null) =>
    go(lightingHref("venues", name, keep()));

  $effect(() => {
    const want = selected;
    const editing = edit;
    const fresh = creating === "venue";
    untrack(() => {
      if (fresh) {
        if (editingVenue !== "__new__") startNewVenue();
      } else if (editing && want) {
        if (editingVenue !== want) void startEditVenue(want);
      } else if (editingVenue !== null) {
        editingVenue = null;
      }
    });
  });
  let ftDir = $derived(fixtureTypesDir);
  let venueDir = $derived(venuesDir);

  // Fixture types, for the type dropdown on a venue's fixtures.
  let fixtureTypes = $state<Record<string, FixtureTypeEntry>>({});
  // --- Venues state ---
  let venues = $state<Record<string, VenueData>>({});
  /** Each venue file's version as listed; the editor saves against the one
   *  it was opened at, so a file changed elsewhere is not overwritten. */
  let venueVersions = $state<Record<string, string>>({});
  let editVersion = $state<string | undefined>(undefined);
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
      beam_angle?: number | null;
      /** The fixture's own GDTF mode; it belongs to the type it was chosen
       *  for (`modeOfType`), so a fixture moved to another type takes that
       *  type's first drivable mode. */
      mode?: string | null;
      modeOfType?: string;
      /** Added or re-typed: take the first drivable mode once known. */
      pickFirst?: boolean;
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
  /** A fixture from a GDTF: every row of it names its own mode. */
  const fromGdtf = (type: string) => !!fixtureTypes[type]?.referential;

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
      venueVersions = result.versions;
      venueFileErrors = result.errors;
    } catch (e: any) {
      venueError = e.message;
    } finally {
      venueLoading = false;
    }
  }

  // Re-read on a directory change, and whenever a fixture type or a venue
  // changed anywhere on the page (the plot, the inspector, the fixture
  // page): the cards, the type list and the versions follow the files.
  $effect(() => {
    void ftDir;
    void $fixtureTypeChanges;
    loadFixtureTypes();
  });

  $effect(() => {
    void venueDir;
    void $venueChanges;
    loadVenues();
  });

  // --- Venue editing ---

  /** Opens the editor on the venue as its file is now. The list may be
   *  older than the file: the plot and inspector on this same page save
   *  the venue too, and an editor opened on the list's copy would carry a
   *  stale version and be refused with "changed elsewhere". */
  async function startEditVenue(name: string) {
    await loadVenues();
    const v = venues[name];
    if (!v) return;
    editVersion = venueVersions[name];
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
        beam_angle: f.beam_angle ?? null,
        mode: f.mode ?? null,
        modeOfType: f.fixture_type,
      }));
    editVenueFocusPoints = { ...(v.focus_points ?? {}) };
    editVenueSource = v.source ?? null;
    isNewVenue = false;
    checkRows = false;
    markVenueClean();
  }

  function startNewVenue() {
    checkRows = false;
    editingVenue = "__new__";
    editVenueName = "";
    editVenueFixtures = [];
    editVenueFocusPoints = {};
    editVenueSource = null;
    editVersion = undefined;
    isNewVenue = true;
    markVenueClean();
  }

  /** Closes the form through the address: unsaved edits ask first (the
   *  app's guard), and staying keeps them. */
  function cancelEditVenue() {
    select(isNewVenue ? null : selected);
  }

  // --- Unsaved edits: the form as it was opened.
  let venueSnapshot = $state("");
  const venueState = () =>
    JSON.stringify([editVenueName, editVenueFixtures, editVenueFocusPoints]);
  function markVenueClean() {
    venueSnapshot = venueState();
  }
  let venueDirty = $derived(!!editingVenue && venueState() !== venueSnapshot);
  $effect(() =>
    guardUnsaved(
      () => venueDirty,
      isNewVenue
        ? get(t)("lighting.discard.newVenue")
        : get(t)("lighting.discard.venue", {
            values: { name: editingVenue ?? "" },
          }),
    ),
  );

  /** The addresses a fixture of `type` occupies, in its type's default
   *  mode (a row's own mode is not known here without its archive); null
   *  when the listing does not know. */
  function footprintOf(type: string): number | null {
    return fixtureTypes[type]?.footprint ?? null;
  }

  // --- A row's own mode (design §21): each GDTF type's modes, fetched once
  // per type for the life of the form, lazily, never blocking typing.

  let archives = $state<Record<string, FixtureTypeGdtf | null>>({});
  $effect(() => {
    if ($fixtureTypeChanges > 0) untrack(() => (archives = {}));
  });
  let gdtfTypes = $derived(
    editingVenue
      ? [
          ...new Set(
            editVenueFixtures
              .map((f) => f.fixture_type)
              .filter((type) => fixtureTypes[type]?.referential),
          ),
        ]
      : [],
  );
  $effect(() => {
    const dir = ftDir;
    for (const type of gdtfTypes) {
      if (untrack(() => type in archives)) continue;
      archives[type] = null;
      fetchFixtureTypeGdtf(type, dir || undefined)
        .then((answer) => (archives[type] = answer))
        .catch(() => {
          // No select for this type: its rows keep the mode they have.
        });
    }
  });

  type Row = (typeof editVenueFixtures)[number];

  /** The modes a row may choose, when its type's archive has been read. */
  const modesOf = (row: Row) =>
    fixtureTypes[row.fixture_type]?.referential
      ? (archives[row.fixture_type]?.inspection.modes ?? null)
      : null;

  /** The first mode of a type's GDTF that mtrack can drive, once read. */
  const firstDrivable = (type: string) =>
    archives[type]?.inspection.modes.find((m) => !isRefused(m))?.name ?? null;

  // A row added or switched to a fixture from a GDTF takes the first mode
  // mtrack can drive as soon as its GDTF's modes are known (`pickFirst`),
  // so Add Fixture → Save always writes a line with its mode. A row read
  // from a file without one is not filled in: it is marked to be chosen.
  $effect(() => {
    for (const row of editVenueFixtures) {
      if (!row.pickFirst || row.mode) continue;
      const first = firstDrivable(row.fixture_type);
      if (first) {
        row.mode = first;
        row.pickFirst = false;
      }
    }
  });

  /** The addresses a row occupies: a fixture from a GDTF, its mode's (once
   *  the GDTF has been read); a hand-written one, its type's; null when not
   *  known. */
  function rowFootprint(row: Row): number | null {
    if (!fromGdtf(row.fixture_type)) return footprintOf(row.fixture_type);
    const own = row.mode ?? null;
    return (
      (own && modesOf(row)?.find((m) => m.name === own)?.footprint) || null
    );
  }

  /** Each row's span, for the strip of words under it and the overlap check. */
  let rowSpans = $derived(
    editVenueFixtures.map((row, i) => ({
      fixture: String(i),
      universe: Number(row.universe),
      address: Number(row.start_channel),
      footprint: rowFootprint(row),
    })),
  );

  /** What each row runs into: other rows it partly overlaps (identical spans
   *  gang, as on the server), and the end of the universe. */
  let rowClashes = $derived(
    rowSpans.map((span) => {
      const hits = collisions(rowSpans, span).map((c) => ({
        row: Number(c.fixture),
        from: c.from,
        to: c.to,
      }));
      const last = lastAddress(span);
      return { hits, overrun: last !== null && last > 512 ? last : null };
    }),
  );

  /** A changed type takes its own default mode: a mode names a mode of one
   *  archive. */
  function typeChanged(row: Row) {
    row.modeOfType = row.fixture_type;
    row.mode = fromGdtf(row.fixture_type)
      ? firstDrivable(row.fixture_type)
      : null;
    row.pickFirst = fromGdtf(row.fixture_type) && !row.mode;
  }

  /** A new row continues the patch from the last row: its type, its
   *  universe, and the address after it. It gets the first unused
   *  `Fixture N`, so adding and saving just works. */
  function addVenueFixture() {
    const last = editVenueFixtures[editVenueFixtures.length - 1];
    const type = last?.fixture_type || fixtureTypeNames[0] || "";
    // A fixture from a GDTF: the previous row's mode when it is the same
    // fixture, else its first drivable mode (or, until its GDTF has been
    // read, whatever the effect above fills in).
    const mode = fromGdtf(type)
      ? last?.fixture_type === type && last.mode
        ? last.mode
        : firstDrivable(type)
      : null;
    const at = nextPatch(
      last
        ? {
            universe: Number(last.universe),
            address: Number(last.start_channel),
            footprint: rowFootprint(last),
          }
        : null,
      rowFootprint({ fixture_type: type, mode } as Row),
    );
    editVenueFixtures = [
      ...editVenueFixtures,
      {
        name: nextFixtureName(editVenueFixtures.map((f) => f.name)),
        fixture_type: type,
        universe: at.universe,
        start_channel: at.address,
        tags: [],
        mode,
        modeOfType: type,
        pickFirst: fromGdtf(type) && !mode,
      },
    ];
  }

  /** Set by a refused save: from then on the rows are checked as they are
   *  edited, so a fixed row clears its message. */
  let checkRows = $state(false);
  let problems = $derived(
    checkRows
      ? rowProblems(editVenueFixtures, fromGdtf)
      : new Map<number, RowProblem[]>(),
  );

  /** Words for what a row runs into. */
  function clashMessage(i: number): string {
    const clash = rowClashes[i];
    if (!clash) return "";
    const parts = clash.hits.map((h) =>
      get(t)("lighting.venueRow.runsInto", {
        values: {
          name: editVenueFixtures[h.row]?.name || `#${h.row + 1}`,
          from: h.from,
          to: h.to,
        },
      }),
    );
    if (clash.overrun !== null)
      parts.push(
        get(t)("lighting.venueRow.overrun", {
          values: { last: clash.overrun },
        }),
      );
    return parts.join(" ");
  }

  function rowMessage(list: RowProblem[]): string {
    return list.map((p) => get(t)(`lighting.venueRow.${p}`)).join(" ");
  }

  function removeVenueFixture(i: number) {
    editVenueFixtures = editVenueFixtures.filter((_, idx) => idx !== i);
  }

  async function saveVenueData() {
    if (!editVenueName.trim()) {
      venueMsg = get(t)("lighting.nameRequired");
      return;
    }
    // Every row is saved or the save is refused: a row that cannot be
    // saved is marked, never left out.
    const found = rowProblems(editVenueFixtures, fromGdtf);
    if (found.size > 0) {
      checkRows = true;
      venueMsg = get(t)("lighting.venueRowsNeedAttention", {
        values: { count: found.size },
      });
      const first = Math.min(...found.keys());
      queueMicrotask(() =>
        document
          .querySelector<HTMLElement>(
            `[data-venue-row="${first}"] [aria-invalid="true"]`,
          )
          ?.focus(),
      );
      return;
    }
    // Overlaps and overruns do not block (a hand-made venue may have them),
    // but a save with any asks first, naming them.
    const clashing = rowClashes
      .map((c, i) => ({ c, i }))
      .filter(({ c }) => c.hits.length > 0 || c.overrun !== null);
    if (clashing.length > 0) {
      const list = clashing
        .map(
          ({ i }) =>
            `${editVenueFixtures[i].name || `#${i + 1}`}: ${clashMessage(i)}`,
        )
        .join("\n");
      if (
        !(await showConfirm(
          get(t)("lighting.venueRow.saveAnyway", { values: { list } }),
          { danger: true },
        ))
      )
        return;
    }
    const fixtures = editVenueFixtures.map((f) => ({
      name: f.name.trim(),
      fixture_type: f.fixture_type.trim(),
      universe: f.universe,
      start_channel: f.start_channel,
      tags: f.tags,
      position: f.position ?? null,
      rotation: f.rotation ?? null,
      beam_angle: f.beam_angle ?? null,
      mode: f.fixture_type.trim() === f.modeOfType ? (f.mode ?? null) : null,
    }));
    const newName = editVenueName.trim();
    const oldName = editingVenue !== "__new__" ? editingVenue : null;
    const isRename = oldName && oldName !== newName;
    if ((isNewVenue || isRename) && newName in venues) {
      venueMsg = get(t)("lighting.venueExists", { values: { name: newName } });
      return;
    }
    venueSaving = true;
    venueMsg = "";
    try {
      const saved = await saveVenue(
        newName,
        {
          fixtures,
          focus_points: editVenueFocusPoints,
          source: editVenueSource,
        },
        venueDir || undefined,
        // A new or renamed venue is a new file; an existing one is saved
        // against the version this form was opened at.
        isNewVenue || isRename ? undefined : editVersion,
      );
      if (isRename) {
        await deleteVenue(oldName, venueDir || undefined);
      }
      await loadVenues();
      editingVenue = null;
      select(newName);
      if (saved.venueError) {
        // Saved, and the venue no longer loads: said here, and left up.
        venueMsg = get(t)("lighting.venueError.saved", {
          values: { ...saved.venueError },
        });
      } else {
        venueMsg = get(t)("common.saved");
        setTimeout(() => (venueMsg = ""), 2000);
      }
    } catch (e: any) {
      if (e instanceof ConflictError) {
        // The file changed since the form was opened: show it as it is now
        // and leave the change to be made again.
        const name = editingVenue;
        await loadVenues();
        if (name && name !== "__new__" && name in venues)
          await startEditVenue(name);
        venueMsg = get(t)("lighting.venueChangedElsewhere");
      } else {
        venueMsg = e.message;
      }
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
      if (selected === name) select(null);
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
          {@const rowIssues = problems.get(i) ?? []}
          {@const bad = (p: RowProblem) => rowIssues.includes(p)}
          <div
            class="venue-fixture-card"
            class:venue-fixture-card--invalid={rowIssues.length > 0}
            data-venue-row={i}
            data-testid="venue-fixture-row"
          >
            <div class="venue-fixture-row">
              <input
                class="input"
                placeholder={$t("lighting.fixtureName")}
                aria-label={$t("lighting.fixtureName")}
                aria-invalid={bad("noName") || bad("duplicateName")}
                aria-describedby={rowIssues.length > 0
                  ? `venue-row-error-${i}`
                  : undefined}
                bind:value={fix.name}
              />
              {#if fixtureTypeNames.length > 0}
                <select
                  class="input"
                  aria-label={$t("lighting.fixtureType")}
                  aria-invalid={bad("noType")}
                  bind:value={fix.fixture_type}
                  onchange={() => typeChanged(fix)}
                >
                  <option value="">{$t("lighting.selectType")}</option>
                  {#each fixtureTypeNames as ftName (ftName)}
                    <option value={ftName}>{ftName}</option>
                  {/each}
                </select>
              {:else}
                <input
                  class="input"
                  placeholder={$t("lighting.fixtureType")}
                  aria-label={$t("lighting.fixtureType")}
                  aria-invalid={bad("noType")}
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
                  aria-invalid={bad("badUniverse")}
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
                  aria-invalid={bad("badAddress")}
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
            {#if modesOf(fix)}
              {@const modes = modesOf(fix) ?? []}
              <div class="venue-fixture-row">
                <div class="field compact-field" style="flex: 2;">
                  <label for={`fix-mode-${i}`}
                    >{$t("venues.inspector.mode")}</label
                  >
                  <select
                    id={`fix-mode-${i}`}
                    class="input mode-select"
                    data-testid="venue-row-mode"
                    value={fix.mode ?? ""}
                    onchange={(e) => (fix.mode = e.currentTarget.value || null)}
                  >
                    {#if !fix.mode}
                      <!-- A mode is always chosen; this shows only for a
                           line read without one, which blocks the save. -->
                      <option value="" disabled
                        >{$t("venues.inspector.modeChoose")}</option
                      >
                    {/if}
                    {#each modes as m (m.name)}
                      <option
                        value={m.name}
                        disabled={m.refused !== undefined}
                        title={m.refused ?? undefined}
                        >{$t("venues.inspector.modeOption", {
                          values: { name: m.name, count: m.footprint },
                        })}</option
                      >
                    {/each}
                    {#if fix.mode && !modes.some((m) => m.name === fix.mode)}
                      <!-- Saved, but not a mode of this archive: kept unless
                           changed, and said so. -->
                      <option value={fix.mode}
                        >{$t("lighting.venueRow.unknownMode", {
                          values: { mode: fix.mode },
                        })}</option
                      >
                    {/if}
                  </select>
                </div>
              </div>
            {/if}
            {#if rowSpans[i]?.footprint}
              {@const span = rowSpans[i]}
              <p class="row-span" data-testid="venue-row-span">
                {span.footprint === 1
                  ? $t("lighting.venueRow.spanOne", {
                      values: { from: span.address },
                    })
                  : $t("lighting.venueRow.span", {
                      values: {
                        from: span.address,
                        to: span.address + (span.footprint ?? 1) - 1,
                      },
                    })}
              </p>
            {/if}
            {#if (rowClashes[i]?.hits.length ?? 0) > 0 || rowClashes[i]?.overrun}
              <p class="row-warning" data-testid="venue-row-overlap">
                {clashMessage(i)}
              </p>
            {/if}
            {#if rowIssues.length > 0}
              <p
                class="row-error"
                id={`venue-row-error-${i}`}
                data-testid="venue-row-error"
              >
                {rowMessage(rowIssues)}
              </p>
            {/if}
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
        <button
          class="btn btn-primary"
          onclick={() =>
            go(lightingHref("venues", null, keep({ new: "venue" })))}
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
            class:item-card--selected={selected === name}
            data-testid="venue-card-{name}"
            role="button"
            tabindex="0"
            aria-pressed={selected === name}
            onclick={() => select(name)}
            onkeydown={(e) => {
              if (e.key === "Enter") select(name);
            }}
          >
            <div class="item-card-header">
              <span class="item-name">{name}</span>
              <span class="item-actions">
                {#if $venueStore?.name === name}
                  <span class="badge" data-testid="venue-live-{name}"
                    >{$t("lighting.venueLive")}</span
                  >
                {/if}
                <button
                  class="btn btn-sm"
                  data-testid="venue-edit-{name}"
                  onclick={(e) => {
                    e.stopPropagation();
                    go(lightingHref("venues", name, keep({ edit: "" })));
                  }}>{$t("lighting.venueEdit")}</button
                >
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

  .item-card--selected {
    border-color: var(--accent);
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

  .venue-fixture-card--invalid {
    border-color: var(--red);
  }

  .venue-fixture-card :global([aria-invalid="true"]) {
    border-color: var(--red);
  }

  .row-span {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .row-warning {
    margin: 0;
    font-size: 12px;
    color: var(--yellow, var(--text));
  }

  .mode-select {
    font-family: var(--mono);
    font-size: 12px;
  }

  .row-error {
    margin: 0;
    font-size: 12px;
    color: var(--red);
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
