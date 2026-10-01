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
  /**
   * The inspector for the Venues plan's selection (design §10): one fixture
   * shows its fields; several show what they share, plus Arrange and Aim.
   * Every operation is one venue save, made through the plan's own `persist`
   * so it and a drag can never race, and the plan redraws from the reload.
   */
  import { t } from "svelte-i18n";
  import { untrack } from "svelte";
  import { guardUnsaved } from "../../lib/dirtyGuard";
  import { fixtureTypeChanges, venueChanges } from "../../lib/lighting/changes";
  import { get } from "svelte/store";
  import TagInput from "../config/TagInput.svelte";
  import Tooltip from "../config/Tooltip.svelte";
  import {
    fetchFixtureTypeGdtf,
    fetchFixtureTypes,
    fetchVenue,
    fetchVenuePatch,
    type FixtureTypeEntry,
    type FixtureTypeGdtf,
    type VenueData,
    type VenuePatch,
  } from "../../lib/api/config";
  import {
    collisions,
    room,
    stripCells,
    stripWindow,
    type Span,
  } from "../../lib/lighting/patch";
  import { isRefused } from "../../lib/lighting/modes";
  import { metadataStore, poseStore } from "../../lib/ws/stores";
  import type {
    FixtureMetadata,
    Vec3,
    VenueMetadata,
  } from "../../lib/ws/stores";
  import {
    BEARINGS,
    MOVER_MOUNTINGS,
    aimRotation,
    deckFootprint,
    faceRotation,
    restAim,
  } from "../../lib/stage/aim";
  import {
    alignOnLine,
    mirrorAcrossCentre,
    spaceEvenly,
    type Placed,
  } from "../../lib/stage/arrange";

  interface Props {
    /** The selected fixtures' names. */
    selection: string[];
    onSelect: (names: string[]) => void;
    /** The plan's read-change-write of the venue file. */
    persist: (update: (venue: VenueData) => void) => Promise<boolean>;
    saving: boolean;
    /** The fixture types directory override (a profile's setting). */
    fixtureTypesDir?: string;
    /** The fixtures the plan shows: the live venue's metadata, or a venue
     *  file's fixtures when the plan is a view of the file. */
    fixtures: Record<string, FixtureMetadata>;
    /** The venue the plan shows. */
    venue: VenueMetadata | null;
    /** Whether the plan is the running venue; a file view has no engine
     *  poses to refresh. */
    live?: boolean;
  }

  let {
    selection,
    onSelect,
    persist,
    saving,
    fixtureTypesDir = "",
    fixtures: plotFixtures,
    venue: plotVenue,
    live = true,
  }: Props = $props();

  // --- What the selection is made of

  let fixtureTypes = $state<Record<string, FixtureTypeEntry>>({});
  $effect(() => {
    const dir = fixtureTypesDir;
    fetchFixtureTypes(dir || undefined)
      .then((r) => (fixtureTypes = r.fixtureTypes))
      .catch(() => {
        // Without types every fixture counts as fixed.
      });
  });
  let typeNames = $derived(Object.keys(fixtureTypes).sort());

  let allNames = $derived(Object.keys(plotFixtures).sort());
  let focusNames = $derived(Object.keys(plotVenue?.focus_points ?? {}).sort());

  /** A fixture the engine reports as able to pan or tilt is a mover; anything
   *  else aims only through its mounting. The metadata says so for every
   *  type, GDTF-referential ones included. */
  function isMover(name: string): boolean {
    return plotFixtures[name]?.capabilities?.includes("pan_tilt") ?? false;
  }

  let picked = $derived(selection.filter((n) => n in plotFixtures));
  let movers = $derived(picked.filter(isMover));
  let fixed = $derived(picked.filter((n) => !isMover(n)));
  let placed = $derived(picked.filter((n) => plotFixtures[n]?.position));
  let single = $derived(picked.length === 1 ? picked[0] : null);

  let sharedType = $derived.by(() => {
    const types = new Set(picked.map((n) => plotFixtures[n]?.type));
    return types.size === 1 ? [...types][0] : null;
  });
  let sharedTags = $derived.by(() => {
    if (picked.length === 0) return [] as string[];
    const [first, ...rest] = picked.map((n) => plotFixtures[n]?.tags ?? []);
    return first.filter((tag) => rest.every((tags) => tags.includes(tag)));
  });

  // --- Status line

  let status = $state<{ text: string; ok: boolean } | null>(null);

  function toggle(name: string) {
    onSelect(
      selection.includes(name)
        ? selection.filter((n) => n !== name)
        : [...selection, name],
    );
  }

  // --- One operation, one save

  interface Change {
    position?: Vec3;
    rotation?: Vec3;
  }

  /** The plan draws a fixed fixture's beam from the pose the engine pushes;
   *  until that arrives, draw it from the rotation just saved. */
  function refreshPoses(names: string[]) {
    if (!live) return;
    const meta = get(metadataStore);
    poseStore.update((poses) => {
      const next = { ...poses };
      for (const name of names) {
        const { position, rotation } = meta[name] ?? {};
        if (isMover(name) || !position || !rotation) continue;
        const aim = restAim(rotation);
        next[name] = {
          pan: 0,
          tilt: 0,
          aim,
          floor: deckFootprint(position, aim),
        };
      }
      return next;
    });
  }

  async function apply(
    compute: (venue: VenueData) => {
      changes: Record<string, Change>;
      notes?: string[];
    },
  ) {
    status = null;
    let notes: string[] = [];
    const rotated: string[] = [];
    const ok = await persist((venue) => {
      const result = compute(venue);
      notes = result.notes ?? [];
      for (const [name, change] of Object.entries(result.changes)) {
        const fixture = venue.fixtures[name];
        if (!fixture) continue;
        if (change.position) fixture.position = change.position;
        if (change.rotation) {
          fixture.rotation = change.rotation;
          rotated.push(name);
        }
      }
    });
    if (ok) {
      refreshPoses(rotated);
      status = notes.length ? { text: notes.join(" "), ok: true } : null;
    }
  }

  /** The selected fixtures the file places, as arrange wants them. */
  function placedIn(venue: VenueData, names: string[]): Placed[] {
    return names
      .filter((n) => venue.fixtures[n]?.position)
      .map((n) => ({
        name: n,
        position: [...(venue.fixtures[n].position as Vec3)] as Vec3,
        ...(venue.fixtures[n].rotation
          ? { rotation: [...(venue.fixtures[n].rotation as Vec3)] as Vec3 }
          : {}),
      }));
  }

  function arrange(fn: (items: Placed[]) => Placed[]) {
    void apply((venue) => {
      const out = fn(placedIn(venue, picked));
      return {
        changes: Object.fromEntries(
          out.map((p) => [
            p.name,
            {
              position: p.position,
              // Mirroring turns the aim over too.
              ...(p.rotation ? { rotation: p.rotation } : {}),
            },
          ]),
        ),
      };
    });
  }

  // --- Aim: face a direction

  type Direction = keyof typeof BEARINGS | "bearing";
  let direction = $state<Direction>("upstage");
  let bearing = $state(0);
  let tilt = $state(20);

  let faceBearing = $derived(
    direction === "bearing" ? bearing : BEARINGS[direction],
  );

  function face() {
    const rotation = faceRotation(faceBearing ?? 0, tilt ?? 0);
    void apply(() => ({
      changes: Object.fromEntries(fixed.map((n) => [n, { rotation }])),
    }));
  }

  // --- Aim: at a focus point

  let focusChoice = $state("");
  $effect(() => {
    if (!focusNames.includes(focusChoice)) focusChoice = focusNames[0] ?? "";
  });

  function aimAtPoint() {
    void apply((venue) => {
      const target = venue.focus_points?.[focusChoice];
      const changes: Record<string, Change> = {};
      const atPoint: string[] = [];
      const unplaced: string[] = [];
      if (!target) return { changes };
      for (const name of fixed) {
        const position = venue.fixtures[name]?.position;
        if (!position) {
          unplaced.push(name);
          continue;
        }
        const rotation = aimRotation(position, target);
        if (rotation) changes[name] = { rotation };
        else atPoint.push(name);
      }
      const notes: string[] = [];
      if (atPoint.length)
        notes.push(
          get(t)("venues.inspector.skippedAtPoint", {
            values: { names: atPoint.join(", ") },
          }),
        );
      if (unplaced.length)
        notes.push(
          get(t)("venues.inspector.skippedUnplaced", {
            values: { names: unplaced.join(", ") },
          }),
        );
      return { changes, notes };
    });
  }

  // --- Aim: the raw rotation

  /** The rotations of the selection, or null where they differ. */
  let sharedRotation = $derived.by(() => {
    const rots = picked.map((n) => plotFixtures[n]?.rotation ?? null);
    if (rots.length === 0) return null;
    const first = rots[0];
    return rots.every(
      (r) =>
        (r === null && first === null) ||
        (r !== null &&
          first !== null &&
          r.every((v, i) => Math.abs(v - first[i]) < 1e-9)),
    )
      ? (first ?? ([0, 0, 0] as Vec3))
      : null;
  });
  /** The triple being typed; it follows the selection until edited. */
  let rotationDraft = $derived<(number | null)[]>(
    sharedRotation ? [...sharedRotation] : [null, null, null],
  );
  function draftAxis(i: number, e: Event) {
    const v = (e.currentTarget as HTMLInputElement).valueAsNumber;
    rotationDraft = rotationDraft.map((old, j) =>
      j === i ? (Number.isNaN(v) ? null : v) : old,
    );
  }
  let rotationValid = $derived(
    rotationDraft.every((v) => typeof v === "number" && Number.isFinite(v)),
  );

  function setRotation(rotation: Vec3) {
    void apply(() => ({
      changes: Object.fromEntries(picked.map((n) => [n, { rotation }])),
    }));
  }

  // --- One fixture's own fields

  interface Fields {
    name: string;
    fixture_type: string;
    universe: number;
    start_channel: number;
    tags: string[];
    /** The fixture's own GDTF mode as its line writes it; null is its
     *  type's default. Changed through the mode select, which saves. */
    mode: string | null;
  }
  let fields = $state<Fields | null>(null);
  // Unapplied edits to the fields (the mode select saves at once, so it is
  // not one of them): leaving with some asks first.
  let fieldsSnapshot = $state("");
  const fieldsState = (f: Fields | null) =>
    f
      ? JSON.stringify([
          f.name,
          f.fixture_type,
          Number(f.universe),
          Number(f.start_channel),
          f.tags,
        ])
      : "";
  let fieldsDirty = $derived(
    !!fields && fieldsState(fields) !== fieldsSnapshot,
  );
  $effect(() =>
    guardUnsaved(
      () => fieldsDirty,
      get(t)("lighting.discard.inspector", { values: { name: single ?? "" } }),
    ),
  );
  $effect(() => {
    const name = single;
    // The file changed (this inspector's save, the venue form, a rename):
    // the fields show it as it is now.
    void $venueChanges;
    fields = null;
    patchMsg = null;
    const shown = plotVenue;
    if (!name || !shown) return;
    let stale = false;
    fetchVenue(shown.name, shown.dir ?? undefined)
      .then(({ venue: file }) => {
        const f = file.fixtures[name];
        if (stale || !f) return;
        fields = {
          name: f.name ?? name,
          fixture_type: f.fixture_type,
          universe: f.universe,
          start_channel: f.start_channel,
          tags: [...f.tags],
          mode: f.mode ?? null,
        };
        fieldsSnapshot = fieldsState(fields);
      })
      .catch(() => {
        // The fields stay hidden; position, aim and the list still work.
      });
    return () => (stale = true);
  });

  async function applyFields() {
    const from = single;
    const draft = fields;
    if (!from || !draft) return;
    const to = draft.name.trim();
    status = null;
    patchMsg = null;
    if (!to || !draft.fixture_type.trim()) return;
    if (to !== from && to in plotFixtures) {
      status = {
        text: get(t)("venues.inspector.nameTaken", { values: { name: to } }),
        ok: false,
      };
      return;
    }
    // A new address is checked against the patch before it is saved, in
    // the fixture's current mode (a new type's footprint is not known yet).
    const own = patch?.spans.find((s) => s.fixture === from);
    if (own && own.type === draft.fixture_type.trim()) {
      const candidate: Span = {
        fixture: from,
        universe: Number(draft.universe),
        address: Number(draft.start_channel),
        footprint: own.footprint,
      };
      const hits = collisions(spans, candidate);
      if (hits.length > 0 && candidate.footprint) {
        patchMsg = {
          ok: false,
          text: get(t)("venues.inspector.addressRefused", {
            values: {
              name: from,
              universe: candidate.universe,
              from: candidate.address,
              to: candidate.address + candidate.footprint - 1,
              names: unique(hits.map((h) => h.fixture)).join(", "),
              count: unique(hits.map((h) => h.fixture)).length,
            },
          }),
        };
        return;
      }
    }
    const ok = await persist((venue) => {
      const current = venue.fixtures[from];
      if (!current) return;
      const type = draft.fixture_type.trim();
      const next = {
        ...current,
        // A mode names a mode of its type's archive: a fixture moved to
        // another type takes that type's default — or, for a fixture with
        // no default, its first drivable mode, so the line stays valid.
        mode:
          type === current.fixture_type
            ? (current.mode ?? null)
            : modeForNewType(type),
        name: to,
        fixture_type: type,
        universe: Number(draft.universe),
        start_channel: Number(draft.start_channel),
        tags: draft.tags,
      };
      // Rebuilt in place, so the fixture keeps its slot in the file.
      venue.fixtures = Object.fromEntries(
        Object.entries(venue.fixtures).map(([key, f]) =>
          key === from ? [to, next] : [key, f],
        ),
      );
    });
    if (ok) patchTick++;
    if (ok && to !== from) onSelect([to]);
  }

  // --- The mode and the patch (design §21)

  const unique = (names: string[]) => [...new Set(names)];

  /** A refusal from the patch check, said beside the strip. */
  let patchMsg = $state<{ text: string; ok: boolean } | null>(null);
  /** The strip's scroller: kept on this fixture's cells. */
  let stripEl = $state<HTMLDivElement | undefined>(undefined);
  $effect(() => {
    void strip;
    const el = stripEl;
    const mine = el?.querySelector<HTMLElement>(
      ".patch-cell--me, .patch-cell--clash",
    );
    if (el && mine)
      el.scrollLeft = Math.max(
        0,
        mine.offsetLeft - el.offsetLeft - el.clientWidth / 3,
      );
  });

  /** The venue's spans, from its files; refreshed after every save. */
  let patch = $state<VenuePatch | null>(null);
  let patchTick = $state(0);
  $effect(() => {
    void patchTick;
    void $venueChanges;
    const shown = plotVenue;
    const typesDir = fixtureTypesDir;
    patch = null;
    if (!shown) return;
    let stale = false;
    fetchVenuePatch(shown.name, shown.dir ?? undefined, typesDir || undefined)
      .then((p) => {
        if (!stale) patch = p;
      })
      .catch(() => {
        // No strip and no check: the save path is unchanged.
      });
    return () => (stale = true);
  });
  let spans = $derived<Span[]>(patch?.spans ?? []);

  /** Each GDTF type's archive, for its modes; fetched once per type. */
  let archives = $state<Record<string, FixtureTypeGdtf | null>>({});
  let gdtfType = $derived(
    fields && fixtureTypes[fields.fixture_type]?.referential
      ? fields.fixture_type
      : null,
  );
  // A type changed (renamed, a new default): its modes are re-read.
  $effect(() => {
    if ($fixtureTypeChanges > 0) untrack(() => (archives = {}));
  });
  $effect(() => {
    const type = gdtfType;
    const typesDir = fixtureTypesDir;
    if (!type || type in archives) return;
    archives[type] = null;
    fetchFixtureTypeGdtf(type, typesDir || undefined)
      .then((answer) => (archives[type] = answer))
      .catch(() => {
        // No archive, no select: the fixture keeps the mode it has.
      });
  });
  let archive = $derived(gdtfType ? (archives[gdtfType] ?? null) : null);
  /** The mode a fixture moved to `type` takes: for a fixture from a GDTF,
   *  the first mode mtrack can drive; a hand-written type has none. */
  function modeForNewType(type: string): string | null {
    if (!fixtureTypes[type]?.referential) return null;
    return (
      archives[type]?.inspection.modes.find((m) => !isRefused(m))?.name ?? null
    );
  }

  /** The addresses a mode occupies. */
  function footprintOf(mode: string): number | null {
    if (!archive || !mode) return null;
    return (
      archive.inspection.modes.find((m) => m.name === mode)?.footprint ?? null
    );
  }

  /** The fixture as the file patches it now. */
  let ownSpan = $derived(
    single ? (spans.find((s) => s.fixture === single) ?? null) : null,
  );

  async function chooseMode(next: string, select: HTMLSelectElement) {
    const name = single;
    const draft = fields;
    if (!name || !draft) return;
    const previous = draft.mode ?? "";
    if (next === previous) return;
    status = null;
    patchMsg = null;
    const footprint = footprintOf(next);
    if (ownSpan && footprint) {
      const candidate: Span = { ...ownSpan, footprint };
      const hits = collisions(spans, candidate);
      if (hits.length > 0) {
        // Refused before the save: the select goes back to what is saved.
        select.value = previous;
        const names = unique(hits.map((h) => h.fixture));
        const fits = room(spans, candidate);
        patchMsg = {
          ok: false,
          text: get(t)(
            fits > 0
              ? "venues.inspector.modeRefused"
              : "venues.inspector.modeRefusedNoRoom",
            {
              values: {
                mode: next,
                from: candidate.address,
                to: candidate.address + footprint - 1,
                names: names.join(", "),
                count: names.length,
                first: names[0],
                room: fits,
              },
            },
          ),
        };
        return;
      }
    }
    const ok = await persist((venue) => {
      const fixture = venue.fixtures[name];
      if (fixture) fixture.mode = next || null;
    });
    if (ok) {
      draft.mode = next || null;
      patchTick++;
    } else {
      select.value = previous;
    }
  }

  /** The strip: addresses around the fixture in its saved mode. */
  let strip = $derived.by(() => {
    if (!ownSpan || !ownSpan.footprint) return null;
    const { from, to } = stripWindow(ownSpan.address, ownSpan.footprint);
    return {
      universe: ownSpan.universe,
      from,
      to,
      cells: stripCells(spans, ownSpan, from, to),
    };
  });
</script>

<aside class="inspector" aria-label={$t("venues.inspector.title")}>
  <h3 class="inspector__title">{$t("venues.inspector.title")}</h3>

  <fieldset class="inspector__list">
    <legend>{$t("venues.inspector.fixtures")}</legend>
    <ul>
      {#each allNames as name (name)}
        <li>
          <label>
            <input
              type="checkbox"
              checked={selection.includes(name)}
              onchange={() => toggle(name)}
            />
            <span class="inspector__name">{name}</span>
            {#if !plotFixtures[name]?.position}
              <span class="inspector__dim"
                >{$t("venues.inspector.unplaced")}</span
              >
            {/if}
          </label>
        </li>
      {/each}
    </ul>
  </fieldset>

  <div class="inspector__count" role="status" aria-live="polite">
    {#if picked.length === 0}
      {$t("venues.inspector.none")}
    {:else}
      {$t("venues.inspector.count", { values: { count: picked.length } })}
      <button class="btn btn-sm" type="button" onclick={() => onSelect([])}>
        {$t("venues.inspector.clear")}
      </button>
    {/if}
  </div>

  {#if single && fields}
    <section class="inspector__section" aria-labelledby="insp-fields">
      <h4 id="insp-fields">{$t("venues.inspector.fields")}</h4>
      <div class="field">
        <label for="insp-name">{$t("lighting.fixtureName")}</label>
        <input id="insp-name" class="input" bind:value={fields.name} />
      </div>
      <div class="field">
        <label for="insp-type">{$t("lighting.fixtureType")}</label>
        {#if typeNames.length > 0}
          <select id="insp-type" class="input" bind:value={fields.fixture_type}>
            {#each typeNames as name (name)}
              <option value={name}>{name}</option>
            {/each}
            {#if !typeNames.includes(fields.fixture_type)}
              <option value={fields.fixture_type}>{fields.fixture_type}</option>
            {/if}
          </select>
        {:else}
          <input
            id="insp-type"
            class="input"
            bind:value={fields.fixture_type}
          />
        {/if}
      </div>
      {#if gdtfType}
        <div class="field">
          <label for="insp-mode">{$t("venues.inspector.mode")}</label>
          {#if archive}
            <select
              id="insp-mode"
              class="input insp-mode"
              data-testid="insp-mode"
              value={fields.mode ?? ""}
              disabled={saving}
              onchange={(e) =>
                chooseMode(e.currentTarget.value, e.currentTarget)}
            >
              {#if !fields.mode}
                <!-- A line read without its mode: the venue does not load
                     until one is chosen here. -->
                <option value="" disabled
                  >{$t("venues.inspector.modeChoose")}</option
                >
              {/if}
              {#each archive.inspection.modes as m (m.name)}
                <option
                  value={m.name}
                  disabled={isRefused(m)}
                  title={m.refused ?? undefined}
                  >{$t("venues.inspector.modeOption", {
                    values: { name: m.name, count: m.footprint },
                  })}</option
                >
              {/each}
              {#if fields.mode && !archive.inspection.modes.some((m) => m.name === fields?.mode)}
                <option value={fields.mode}>{fields.mode}</option>
              {/if}
            </select>
          {:else}
            <span class="inspector__hint">{$t("common.loading")}</span>
          {/if}
        </div>
      {/if}
      {#if strip}
        <div class="field">
          <span class="field__label"
            >{$t("venues.inspector.patchStrip", {
              values: {
                universe: strip.universe,
                from: strip.from,
                to: strip.to,
              },
            })}</span
          >
          <div class="patch-scroll" bind:this={stripEl}>
            <ol class="patch-strip" data-testid="insp-patch-strip">
              {#each strip.cells as cell (cell.address)}
                <li
                  class="patch-cell"
                  class:patch-cell--me={cell.mine && !cell.clash}
                  class:patch-cell--other={!cell.mine && cell.owners.length > 0}
                  class:patch-cell--clash={cell.clash}
                  data-address={cell.address}
                  title={[
                    ...(cell.mine && single ? [single] : []),
                    ...cell.owners,
                  ].join(", ") || $t("venues.inspector.patchFree")}
                >
                  {cell.address}
                </li>
              {/each}
            </ol>
          </div>
          <div class="patch-legend">
            <span><i class="patch-key patch-key--me"></i>{single}</span>
            <span
              ><i class="patch-key patch-key--other"></i>{$t(
                "venues.inspector.patchOthers",
              )}</span
            >
            <span
              ><i class="patch-key patch-key--clash"></i>{$t(
                "venues.inspector.patchOverlap",
              )}</span
            >
          </div>
        </div>
      {/if}
      {#if patchMsg}
        <p
          class="inspector__status inspector__status--error"
          role="alert"
          data-testid="insp-patch-msg"
        >
          {patchMsg.text}
        </p>
      {/if}
      <div class="row">
        <div class="field">
          <label for="insp-universe">{$t("lighting.universe")}</label>
          <input
            id="insp-universe"
            class="input"
            type="number"
            min="1"
            bind:value={fields.universe}
          />
        </div>
        <div class="field">
          <label for="insp-channel">{$t("lighting.channelLabel")}</label>
          <input
            id="insp-channel"
            class="input"
            type="number"
            min="1"
            bind:value={fields.start_channel}
          />
        </div>
      </div>
      <div class="field">
        <span class="field__label"
          >{$t("lighting.tags")}<Tooltip
            text={$t("tooltips.lighting.fixtureTags")}
          /></span
        >
        <TagInput
          tags={fields.tags}
          onchange={(tags) => fields && (fields.tags = tags)}
          placeholder={$t("lighting.tagPlaceholder")}
        />
      </div>
      <button
        class="btn btn-primary btn-sm"
        type="button"
        disabled={saving}
        onclick={applyFields}
      >
        {$t("venues.inspector.apply")}
      </button>
    </section>
  {:else if picked.length > 1}
    <section class="inspector__section" aria-labelledby="insp-shared">
      <h4 id="insp-shared">{$t("venues.inspector.shared")}</h4>
      <dl class="inspector__facts">
        <dt>{$t("lighting.fixtureType")}</dt>
        <dd data-testid="shared-type">
          {sharedType ?? $t("venues.inspector.mixed")}
        </dd>
        <dt>{$t("lighting.tags")}</dt>
        <dd data-testid="shared-tags">
          {sharedTags.length ? sharedTags.join(", ") : "—"}
        </dd>
      </dl>
    </section>

    <section class="inspector__section" aria-labelledby="insp-arrange">
      <h4 id="insp-arrange">{$t("venues.inspector.arrange")}</h4>
      <div class="buttons">
        <button
          class="btn btn-sm"
          type="button"
          disabled={saving || placed.length < 2}
          onclick={() => arrange(alignOnLine)}
        >
          {$t("venues.inspector.align")}
        </button>
        <button
          class="btn btn-sm"
          type="button"
          disabled={saving || placed.length < 3}
          onclick={() => arrange(spaceEvenly)}
        >
          {$t("venues.inspector.space")}
        </button>
        <button
          class="btn btn-sm"
          type="button"
          disabled={saving || placed.length < 2}
          onclick={() => arrange(mirrorAcrossCentre)}
        >
          {$t("venues.inspector.mirror")}
        </button>
      </div>
      <p class="inspector__hint">{$t("venues.inspector.nudgeHint")}</p>
    </section>
  {/if}

  {#if picked.length > 0}
    <section class="inspector__section" aria-labelledby="insp-aim">
      <h4 id="insp-aim">{$t("venues.inspector.aim")}</h4>

      {#if fixed.length > 0}
        <fieldset class="inspector__group">
          <legend
            >{$t("venues.inspector.face", {
              values: { count: fixed.length },
            })}</legend
          >
          <div class="field">
            <label for="insp-direction"
              >{$t("venues.inspector.direction")}</label
            >
            <select
              id="insp-direction"
              class="input"
              bind:value={direction}
              onchange={() => {
                if (direction !== "bearing") bearing = BEARINGS[direction];
              }}
            >
              <option value="stageLeft"
                >{$t("venues.inspector.dir.stageLeft")}</option
              >
              <option value="stageRight"
                >{$t("venues.inspector.dir.stageRight")}</option
              >
              <option value="upstage"
                >{$t("venues.inspector.dir.upstage")}</option
              >
              <option value="downstage"
                >{$t("venues.inspector.dir.downstage")}</option
              >
              <option value="bearing"
                >{$t("venues.inspector.dir.bearing")}</option
              >
            </select>
          </div>
          {#if direction === "bearing"}
            <div class="field">
              <label for="insp-bearing">{$t("venues.inspector.bearing")}</label>
              <input
                id="insp-bearing"
                class="input"
                type="number"
                step="1"
                bind:value={bearing}
              />
            </div>
          {/if}
          <div class="field">
            <label for="insp-tilt">{$t("venues.inspector.tilt")}</label>
            <div class="row">
              <input
                id="insp-tilt"
                class="input"
                type="number"
                min="-90"
                max="90"
                step="1"
                bind:value={tilt}
              />
              <input
                type="range"
                min="-90"
                max="90"
                step="1"
                aria-label={$t("venues.inspector.tilt")}
                bind:value={tilt}
              />
            </div>
          </div>
          <button
            class="btn btn-sm"
            type="button"
            disabled={saving}
            onclick={face}
          >
            {$t("venues.inspector.faceButton")}
          </button>
        </fieldset>

        <fieldset class="inspector__group">
          <legend>{$t("venues.inspector.atPoint")}</legend>
          {#if focusNames.length === 0}
            <p class="inspector__hint">{$t("venues.inspector.noPoints")}</p>
          {:else}
            <div class="field">
              <label for="insp-point">{$t("venues.inspector.point")}</label>
              <select id="insp-point" class="input" bind:value={focusChoice}>
                {#each focusNames as name (name)}
                  <option value={name}>{name}</option>
                {/each}
              </select>
            </div>
            <button
              class="btn btn-sm"
              type="button"
              disabled={saving}
              onclick={aimAtPoint}
            >
              {$t("venues.inspector.aimButton")}
            </button>
          {/if}
        </fieldset>
      {/if}

      {#if movers.length > 0}
        <fieldset class="inspector__group">
          <legend
            >{$t("venues.inspector.mount", {
              values: { count: movers.length },
            })}</legend
          >
          <p class="inspector__hint">{$t("venues.inspector.moverHint")}</p>
          <div class="buttons">
            <button
              class="btn btn-sm"
              type="button"
              disabled={saving}
              onclick={() =>
                void apply(() => ({
                  changes: Object.fromEntries(
                    movers.map((n) => [
                      n,
                      { rotation: [...MOVER_MOUNTINGS.hung] as Vec3 },
                    ]),
                  ),
                }))}
            >
              {$t("venues.inspector.hung")}
            </button>
            <button
              class="btn btn-sm"
              type="button"
              disabled={saving}
              onclick={() =>
                void apply(() => ({
                  changes: Object.fromEntries(
                    movers.map((n) => [
                      n,
                      { rotation: [...MOVER_MOUNTINGS.standing] as Vec3 },
                    ]),
                  ),
                }))}
            >
              {$t("venues.inspector.standing")}
            </button>
          </div>
        </fieldset>
      {/if}

      <fieldset class="inspector__group">
        <legend>{$t("venues.inspector.rotation")}</legend>
        <div class="row">
          {#each ["X", "Y", "Z"] as axis, i (axis)}
            <div class="field">
              <label for={`insp-rot-${axis}`}>{axis}</label>
              <input
                id={`insp-rot-${axis}`}
                class="input"
                type="number"
                step="any"
                placeholder={sharedRotation ? "" : $t("venues.inspector.mixed")}
                value={rotationDraft[i] ?? ""}
                oninput={(e) => draftAxis(i, e)}
              />
            </div>
          {/each}
        </div>
        <button
          class="btn btn-sm"
          type="button"
          disabled={saving || !rotationValid}
          onclick={() => setRotation(rotationDraft as Vec3)}
        >
          {$t("venues.inspector.setRotation")}
        </button>
      </fieldset>
    </section>
  {/if}

  {#if status}
    <p
      class="inspector__status"
      class:inspector__status--error={!status.ok}
      role="status"
    >
      {status.text}
    </p>
  {/if}
</aside>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    min-width: 0;
    border-top: 1px solid var(--card-border);
  }
  .inspector__title,
  h4 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  fieldset {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin: 0;
    padding: 8px 10px;
    min-width: 0;
  }
  legend {
    font-size: 12px;
    color: var(--text-muted);
    padding: 0 4px;
  }
  .inspector__list ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 160px;
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 2px 8px;
  }
  .inspector__list label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    min-height: 28px;
  }
  .inspector__name {
    font-family: var(--nc-font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .inspector__dim,
  .inspector__hint {
    font-size: 12px;
    color: var(--text-dim);
  }
  .inspector__hint {
    margin: 0;
  }
  .inspector__count {
    font-size: 13px;
    color: var(--text-dim);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .inspector__section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .inspector__group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .inspector__facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 12px;
    margin: 0;
    font-size: 13px;
  }
  .inspector__facts dt {
    color: var(--text-muted);
  }
  .inspector__facts dd {
    margin: 0;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .field label,
  .field__label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: end;
  }
  .buttons {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .inspector__status {
    margin: 0;
    font-size: 13px;
    color: var(--text-dim);
  }
  .inspector__status--error {
    color: var(--red);
  }
  .btn-sm {
    padding: 4px 8px;
    font-size: 12px;
  }
  .insp-mode {
    font-family: var(--nc-font-mono);
    font-size: 12px;
  }
  /* The patch strip scrolls inside itself, never the page. */
  .patch-scroll {
    overflow-x: auto;
    max-width: 100%;
  }
  .patch-strip {
    display: grid;
    grid-template-columns: repeat(32, minmax(18px, 1fr));
    gap: 2px;
    min-width: 600px;
    margin: 2px 0 0;
    padding: 0;
    list-style: none;
  }
  .patch-cell {
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 3px;
    border: 1px solid var(--border);
    background: var(--bg-input);
    color: var(--text-dim);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
  }
  .patch-cell--other {
    background: var(--accent-subtle);
    border-color: transparent;
    color: var(--accent);
  }
  .patch-cell--me {
    background: var(--accent);
    border-color: transparent;
    color: var(--bg);
    font-weight: 600;
  }
  .patch-cell--clash {
    background: var(--red);
    border-color: transparent;
    color: var(--bg);
    font-weight: 600;
  }
  .patch-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .patch-key {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 2px;
    margin-right: 5px;
    vertical-align: -1px;
  }
  .patch-key--me {
    background: var(--accent);
  }
  .patch-key--other {
    background: var(--accent-subtle);
  }
  .patch-key--clash {
    background: var(--red);
  }
  @media (max-width: 600px) {
    .inspector {
      padding: 12px 16px;
    }
  }
</style>
