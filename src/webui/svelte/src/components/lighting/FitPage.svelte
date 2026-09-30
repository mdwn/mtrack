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
  import { untrack } from "svelte";
  import { t, locale } from "svelte-i18n";
  import { get } from "svelte/store";
  import { BadAnswerError } from "../../lib/lighting/answer";
  import {
    ConflictError,
    fetchLightingFit,
    fetchVenue,
    saveVenue,
  } from "../../lib/api/config";
  import {
    olaPatchLine,
    orderGroups,
    sameSet,
    tagsFor,
    withTags,
    type Cluster,
    type Depth,
    type Fit,
    type FitGroup,
    type Height,
    type Want,
  } from "../../lib/lighting/fit";
  import { addUniverseToRunningProfile } from "../../lib/profileStore";
  import { metadataStore, reloadStore, venueStore } from "../../lib/ws/stores";
  import StageView from "../StageView.svelte";

  interface Props {
    /** The `?group=` value from the hash: the group to select first. */
    groupParam: string | null;
    /** The running profile's name, for the link to its Groups page. */
    profileName: string | null;
  }

  let { groupParam, profileName }: Props = $props();

  let fit = $state<Fit | null>(null);
  let error = $state<string | null>(null);
  let selectedName = $state<string | null>(null);
  /** The fixtures a click on Apply (or Tag N fixtures) would tag. */
  let selection = $state<string[]>([]);
  let picking = $state(false);
  let tagging = $state(false);
  let tagMsg = $state<{ ok: boolean; text: string } | null>(null);
  let placing = $state<string | null>(null);
  let focusMsg = $state<{ ok: boolean; text: string } | null>(null);
  let outputMsg = $state<{ ok: boolean; text: string } | null>(null);
  let copied = $state<number | null>(null);

  // A refresh that arrives while one is running is folded into a single
  // follow-up: a reload sends both a reload and a metadata message.
  let inflight = false;
  let again = false;
  /** The venue file's version the page was loaded at. */
  let venueVersion: string | undefined;

  async function refresh() {
    if (inflight) {
      again = true;
      return;
    }
    inflight = true;
    try {
      fit = await fetchLightingFit();
      error = null;
      settleSelection();
      // The venue file's version as of what the page now shows: Apply saves
      // against it, so a file changed since is not overwritten.
      const shown = fit.venue?.name;
      if (shown) {
        fetchVenue(shown, get(venueStore)?.dir ?? undefined)
          .then((r) => {
            if (fit?.venue?.name === shown) venueVersion = r.version;
          })
          .catch(() => {
            venueVersion = undefined;
          });
      } else {
        venueVersion = undefined;
      }
    } catch (e) {
      error =
        e instanceof BadAnswerError
          ? $t("lighting.badAnswer")
          : e instanceof Error
            ? e.message
            : String(e);
    } finally {
      inflight = false;
      if (again) {
        again = false;
        refresh();
      }
    }
  }

  // Re-read when the venue or the config reloads, as the hub does: a tag
  // saved here reloads the venue, which is what brings the new counts in.
  $effect(() => {
    void $metadataStore;
    void $venueStore;
    void $reloadStore;
    untrack(refresh);
  });

  let ordered = $derived(fit ? orderGroups(fit.groups) : []);
  let current = $derived<FitGroup | null>(
    ordered.find((g) => g.name === selectedName) ?? null,
  );
  let tags = $derived(current ? tagsFor(current) : []);
  let canTag = $derived(current !== null && current.defined && tags.length > 0);
  let isSuggestion = $derived(
    current?.suggestion != null &&
      sameSet(selection, current.suggestion.fixtures),
  );
  /** The suggested set first, then the alternatives. */
  let clusters = $derived<Cluster[]>(
    current
      ? [
          ...(current.suggestion
            ? [
                {
                  fixtures: current.suggestion.fixtures,
                  type: current.suggestion.reason.type,
                  where: current.suggestion.reason.where,
                  height: current.suggestion.reason.height,
                  depth: current.suggestion.reason.depth,
                },
              ]
            : []),
          ...current.others,
        ]
      : [],
  );
  let found = $derived(fit?.groups.filter((g) => g.fixtures.length > 0).length);

  /** Keeps the selected group and the selection valid against fresh facts:
   *  the first group when none is chosen, and only fixtures the venue has. */
  function settleSelection() {
    if (!fit) return;
    const groups = orderGroups(fit.groups);
    if (!groups.some((g) => g.name === selectedName)) {
      const wanted = groups.find((g) => g.name === groupParam);
      select((wanted ?? groups[0])?.name ?? null);
      return;
    }
    const known = new Set(fit.venue?.fixtures.map((f) => f.name));
    selection = selection.filter((n) => known.has(n));
  }

  /** Chooses a group; the suggestion becomes the pending selection. */
  function select(name: string | null) {
    selectedName = name;
    picking = false;
    tagMsg = null;
    const group = fit?.groups.find((g) => g.name === name);
    selection = group?.suggestion ? [...group.suggestion.fixtures] : [];
  }

  // A link to another group on this page (the hub's) selects it.
  $effect(() => {
    const wanted = groupParam;
    untrack(() => {
      if (wanted && fit?.groups.some((g) => g.name === wanted)) select(wanted);
    });
  });

  function toggle(name: string) {
    if (!canTag || tagging) return;
    selection = selection.includes(name)
      ? selection.filter((n) => n !== name)
      : [...selection, name];
    tagMsg = null;
  }

  function useCluster(cluster: { fixtures: string[] }) {
    selection = [...cluster.fixtures];
    tagMsg = null;
  }

  async function tagSelection() {
    if (!fit?.venue || !current || selection.length === 0) return;
    const venueName = fit.venue.name;
    const names = [...selection];
    const toAdd = tagsFor(current);
    tagging = true;
    tagMsg = null;
    try {
      // The venue file is the truth: read it, add the tags, write it back
      // whole, so positions, rotations, focus points and provenance stay as
      // they are. The server reloads the venue after the save.
      const dir = get(venueStore)?.dir ?? undefined;
      const { venue, version: fresh } = await fetchVenue(venueName, dir);
      if (venueVersion && fresh && venueVersion !== fresh) {
        throw new ConflictError("", fresh);
      }
      const fixtures = Object.entries(venue.fixtures).map(([name, f]) => ({
        ...f,
        name: f.name ?? name,
        tags: f.tags ?? [],
      }));
      await saveVenue(
        venueName,
        {
          fixtures: withTags(fixtures, names, toAdd),
          focus_points: venue.focus_points ?? {},
          source: venue.source ?? null,
        },
        dir,
        venueVersion ?? fresh,
      );
      tagMsg = {
        ok: true,
        text: $t("lighting.fit.tagged", {
          values: { count: names.length, tags: toAdd.join(", ") },
        }),
      };
      selection = [];
      picking = false;
      await refresh();
    } catch (e) {
      if (e instanceof ConflictError) {
        // The venue file changed since the page loaded: show it as it is
        // now and leave the tagging to be done again.
        tagMsg = { ok: false, text: $t("lighting.fit.changedElsewhere") };
        selection = [];
        picking = false;
        await refresh();
        return;
      }
      tagMsg = {
        ok: false,
        text: $t("lighting.fit.tagFailed", {
          values: { error: e instanceof Error ? e.message : String(e) },
        }),
      };
    } finally {
      tagging = false;
    }
  }

  function onFocusPlaced(name: string, ok: boolean) {
    placing = null;
    focusMsg = {
      ok,
      text: $t(
        ok ? "lighting.fit.focus.placed" : "lighting.fit.focus.placeFailed",
        {
          values: { name },
        },
      ),
    };
    if (ok) void refresh();
  }

  async function addUniverse(universe: number) {
    outputMsg = null;
    try {
      const profile = await addUniverseToRunningProfile(universe);
      outputMsg = {
        ok: true,
        text: $t("lighting.fit.output.added", {
          values: { universe, profile },
        }),
      };
      await refresh();
    } catch (e) {
      outputMsg = {
        ok: false,
        text: $t("lighting.fit.output.addFailed", {
          values: {
            universe,
            error: e instanceof Error ? e.message : String(e),
          },
        }),
      };
    }
  }

  async function copyPatch(universe: number) {
    try {
      await navigator.clipboard.writeText(olaPatchLine(universe));
      copied = universe;
      setTimeout(() => {
        if (copied === universe) copied = null;
      }, 2000);
    } catch {
      // The line is on the page to select by hand.
    }
  }

  // --- Wording, from the structured facts the server sends.

  function list(items: string[]): string {
    return new Intl.ListFormat($locale ?? undefined, {
      style: "long",
      type: "conjunction",
    }).format(items);
  }

  function wantsText(wants: Want[]): string {
    return list(wants.map((w) => $t(`lighting.fit.can.${w}`)));
  }

  function whereText(height: Height | null, depth: Depth | null): string {
    return [
      depth ? $t(`lighting.fit.where.${depth}`) : null,
      height ? $t(`lighting.fit.where.${height}`) : null,
    ]
      .filter(Boolean)
      .join(" ");
  }

  function reasonText(group: FitGroup): string {
    const s = group.suggestion;
    if (!s) return "";
    const where = whereText(s.reason.height, s.reason.depth);
    const plain = s.reason.can.length === 0;
    const key = `lighting.fit.suggestion.${where ? "placed" : "unplaced"}${plain ? "Plain" : ""}`;
    return $t(key, {
      values: {
        count: s.reason.count,
        type: s.reason.type,
        where,
        can: wantsText(s.reason.can),
        group: group.name,
      },
    });
  }

  function clusterLabel(c: Cluster): string {
    const where = whereText(c.height, c.depth);
    return $t(
      where ? "lighting.fit.others.usePlaced" : "lighting.fit.others.use",
      {
        values: { count: c.fixtures.length, type: c.type, where },
      },
    );
  }

  function needsParts(group: FitGroup): string[] {
    const n = group.needs;
    const parts: string[] = [];
    if (n.all_of.length)
      parts.push(
        $t("lighting.fit.needs.allOf", {
          values: { tags: n.all_of.join(", ") },
        }),
      );
    if (n.any_of.length)
      parts.push(
        $t("lighting.fit.needs.anyOf", {
          values: { tags: n.any_of.join(", ") },
        }),
      );
    if (n.prefer.length)
      parts.push(
        $t("lighting.fit.needs.prefer", {
          values: { tags: n.prefer.join(", ") },
        }),
      );
    return parts.length ? parts : [$t("lighting.fit.needs.none")];
  }

  let groupsHref = $derived(
    profileName
      ? `#/lighting/groups?profile=${encodeURIComponent(profileName)}`
      : "#/lighting/groups",
  );
</script>

<div class="fit" data-testid="lighting-fit">
  <p class="fit__lede">{$t("lighting.fit.lede")}</p>

  {#if error}
    <p class="fit__error" role="alert" data-testid="fit-error">
      {$t("lighting.fit.error", { values: { error } })}
      <button class="btn btn-sm" onclick={refresh}>{$t("common.retry")}</button>
    </p>
  {/if}

  {#if !fit && !error}
    <p class="fit__quiet" data-testid="fit-loading">
      {$t("lighting.fit.loading")}
    </p>
  {/if}

  {#if fit && !fit.venue}
    <p class="fit__quiet" data-testid="fit-no-venue">
      {$t("lighting.fit.noVenue")}
      <a href={groupsHref}>{$t("lighting.area.groups")}</a>
    </p>
  {/if}

  {#if fit && fit.venue}
    <div class="fit__columns">
      <!-- 1. Groups -->
      <section class="fit__col" aria-labelledby="fit-groups-title">
        <h2 id="fit-groups-title" class="fit__title">
          {$t("lighting.fit.columns.groups")}
        </h2>
        {#if ordered.length === 0}
          <p class="fit__quiet" data-testid="fit-groups-none">
            {$t("lighting.fit.groups.none")}
          </p>
        {:else}
          <ul class="groups" data-testid="fit-groups">
            {#each ordered as group (group.name)}
              <li>
                <button
                  type="button"
                  class="group"
                  class:group--selected={group.name === selectedName}
                  class:group--empty={group.fixtures.length === 0}
                  aria-pressed={group.name === selectedName}
                  data-testid="fit-group-{group.name}"
                  onclick={() => select(group.name)}
                >
                  <span class="group__name">{group.name}</span>
                  <span class="group__state">
                    {group.fixtures.length === 0
                      ? $t("lighting.fit.groups.empty")
                      : $t("lighting.fit.groups.finds", {
                          values: { count: group.fixtures.length },
                        })}
                  </span>
                  <span class="group__needs"
                    >{needsParts(group).join("; ")}</span
                  >
                  <span class="group__songs"
                    >{$t("lighting.fit.groups.usedBy", {
                      values: { songs: group.songs.join(", ") },
                    })}</span
                  >
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <!-- 2. The plan -->
      <section class="fit__col fit__col--plan" aria-labelledby="fit-plan-title">
        <h2 id="fit-plan-title" class="fit__title">
          {$t("lighting.fit.columns.plan")}
        </h2>
        <StageView
          highlight={current?.fixtures ?? []}
          selected={selection}
          onFixtureClick={toggle}
          placeFocus={placing}
          {onFocusPlaced}
        />
        <p class="fit__quiet fit__legend">{$t("lighting.fit.plan.legend")}</p>
      </section>

      <!-- 3. Fixes -->
      <section class="fit__col" aria-labelledby="fit-fixes-title">
        <h2 id="fit-fixes-title" class="fit__title">
          {$t("lighting.fit.columns.fixes")}
        </h2>

        {#if current}
          <div class="fix" data-testid="fit-group-fix">
            <h3 class="fix__title">{current.name}</h3>
            <p class="fix__needs">
              <strong>{$t("lighting.fit.needs.label")}:</strong>
              {needsParts(current).join("; ")}
            </p>

            {#if !current.defined}
              <p data-testid="fit-undefined">
                {$t("lighting.fit.group.undefined", {
                  values: { group: current.name },
                })}
                <a href={groupsHref}>{$t("lighting.area.groups")}</a>
              </p>
            {:else if tags.length === 0}
              <p data-testid="fit-no-tags">
                {$t("lighting.fit.group.noTags", {
                  values: { group: current.name },
                })}
              </p>
            {:else}
              {#if current.fixtures.length > 0}
                <p data-testid="fit-finds">
                  {$t("lighting.fit.group.finds", {
                    values: {
                      group: current.name,
                      count: current.fixtures.length,
                      names: current.fixtures.join(", "),
                    },
                  })}
                </p>
              {/if}

              {#if current.suggestion}
                <div class="suggestion" data-testid="fit-suggestion">
                  <h4 class="fix__sub">
                    {$t("lighting.fit.suggestion.title")}
                  </h4>
                  <p>{reasonText(current)}</p>
                  <p class="fit__quiet">
                    {$t("lighting.fit.suggestion.adds", {
                      values: {
                        tagCount: current.suggestion.tags.length,
                        tags: current.suggestion.tags.join(", "),
                        fixtures: current.suggestion.fixtures.join(", "),
                      },
                    })}
                  </p>
                </div>
              {:else if current.fixtures.length === 0}
                <div class="suggestion" data-testid="fit-no-suggestion">
                  <h4 class="fix__sub">
                    {$t("lighting.fit.suggestion.title")}
                  </h4>
                  <p>{$t("lighting.fit.suggestion.none")}</p>
                  {#if current.unmet.length > 0}
                    <p data-testid="fit-unmet">
                      {$t(
                        current.unmet_together
                          ? "lighting.fit.suggestion.unmetTogether"
                          : "lighting.fit.suggestion.unmet",
                        {
                          values: {
                            wants: wantsText(current.unmet),
                            group: current.name,
                          },
                        },
                      )}
                    </p>
                  {/if}
                  <p class="fit__quiet">
                    {$t("lighting.fit.suggestion.byHand")}
                  </p>
                </div>
              {/if}

              <p
                class="fit__count"
                aria-live="polite"
                data-testid="fit-selection-count"
              >
                {$t("lighting.fit.selection.count", {
                  values: { count: selection.length },
                })}
              </p>

              <div class="fix__actions">
                {#if isSuggestion}
                  <button
                    type="button"
                    class="btn btn-primary"
                    disabled={tagging}
                    data-testid="fit-apply"
                    onclick={tagSelection}
                    >{tagging
                      ? $t("lighting.fit.tagging")
                      : $t("lighting.fit.apply")}</button
                  >
                {:else if selection.length > 0}
                  <button
                    type="button"
                    class="btn btn-primary"
                    disabled={tagging}
                    data-testid="fit-tag-selection"
                    onclick={tagSelection}
                    >{tagging
                      ? $t("lighting.fit.tagging")
                      : $t("lighting.fit.tagN", {
                          values: { count: selection.length },
                        })}</button
                  >
                {/if}
                <button
                  type="button"
                  class="btn"
                  aria-expanded={picking}
                  aria-controls="fit-picker"
                  data-testid="fit-pick-others"
                  onclick={() => (picking = !picking)}
                  >{current.suggestion
                    ? $t("lighting.fit.pickOthers")
                    : $t("lighting.fit.group.pickTitle")}</button
                >
              </div>

              {#if picking}
                <div class="picker" id="fit-picker" data-testid="fit-picker">
                  {#if clusters.length > 0}
                    <h4 class="fix__sub">{$t("lighting.fit.others.title")}</h4>
                    <ul class="clusters">
                      {#each clusters as cluster, i (i)}
                        <li>
                          <button
                            type="button"
                            class="btn btn-sm cluster"
                            aria-pressed={sameSet(selection, cluster.fixtures)}
                            data-testid="fit-cluster-{i}"
                            onclick={() => useCluster(cluster)}
                            >{clusterLabel(cluster)}</button
                          >
                        </li>
                      {/each}
                    </ul>
                  {/if}
                  <p class="fit__quiet">{$t("lighting.fit.others.hand")}</p>
                  <fieldset class="fixtures">
                    <legend>
                      {$t("lighting.fit.plan.fixturesLabel", {
                        values: { venue: fit.venue.name },
                      })}
                    </legend>
                    {#each fit.venue.fixtures as fixture (fixture.name)}
                      <label class="fixtures__row">
                        <input
                          type="checkbox"
                          checked={selection.includes(fixture.name)}
                          disabled={tagging}
                          data-testid="fit-fixture-{fixture.name}"
                          onchange={() => toggle(fixture.name)}
                        />
                        <span
                          >{$t("lighting.fit.plan.fixtureOption", {
                            values: { name: fixture.name, type: fixture.type },
                          })}</span
                        >
                        <span class="fit__quiet"
                          >{fixture.tags.length
                            ? $t("lighting.fit.plan.hasTags", {
                                values: { tags: fixture.tags.join(", ") },
                              })
                            : $t("lighting.fit.plan.noTags")}</span
                        >
                      </label>
                    {/each}
                  </fieldset>
                </div>
              {/if}

              {#if tagMsg}
                <p
                  class="fit__msg"
                  class:fit__msg--error={!tagMsg.ok}
                  role={tagMsg.ok ? "status" : "alert"}
                  data-testid="fit-tag-msg"
                >
                  {tagMsg.text}
                </p>
              {/if}
            {/if}
          </div>
        {/if}

        <!-- Focus points the shows aim at -->
        <div class="fix" data-testid="fit-focus">
          <h3 class="fix__title">{$t("lighting.fit.focus.title")}</h3>
          {#if fit.focus_points_wanted.length === 0}
            <p class="fit__quiet">{$t("lighting.fit.focus.none")}</p>
          {:else}
            <ul class="wanted">
              {#each fit.focus_points_wanted as point (point.name)}
                <li class="wanted__row" data-testid="fit-focus-{point.name}">
                  <span
                    >{$t("lighting.fit.focus.item", {
                      values: {
                        name: point.name,
                        songs: point.songs.join(", "),
                      },
                    })}</span
                  >
                  {#if placing === point.name}
                    <button
                      type="button"
                      class="btn btn-sm"
                      onclick={() => (placing = null)}
                      >{$t("lighting.fit.focus.cancel")}</button
                    >
                  {:else}
                    <button
                      type="button"
                      class="btn btn-sm"
                      data-testid="fit-place-{point.name}"
                      onclick={() => {
                        placing = point.name;
                        focusMsg = null;
                      }}>{$t("lighting.fit.focus.place")}</button
                    >
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if placing}
            <p role="status" data-testid="fit-placing">
              {$t("lighting.fit.focus.placing", { values: { name: placing } })}
            </p>
          {/if}
          {#if focusMsg}
            <p
              class="fit__msg"
              class:fit__msg--error={!focusMsg.ok}
              role={focusMsg.ok ? "status" : "alert"}
              data-testid="fit-focus-msg"
            >
              {focusMsg.text}
            </p>
          {/if}
        </div>

        <!-- Output -->
        <div class="fix" data-testid="fit-output">
          <h3 class="fix__title">{$t("lighting.fit.output.title")}</h3>
          {#if fit.output.unconfigured.length === 0 && fit.output.unpatched.length === 0}
            <p class="fit__quiet">{$t("lighting.fit.output.none")}</p>
          {/if}
          <ul class="wanted">
            {#each fit.output.unconfigured as universe (universe)}
              <li class="wanted__row" data-testid="fit-universe-{universe}">
                <span
                  >{$t("lighting.fit.output.unconfigured", {
                    values: { universe },
                  })}</span
                >
                <button
                  type="button"
                  class="btn btn-sm"
                  data-testid="fit-add-universe-{universe}"
                  onclick={() => addUniverse(universe)}
                  >{$t("lighting.fit.output.add")}</button
                >
              </li>
            {/each}
          </ul>
          {#if fit.output.reachable === false}
            <p class="fit__quiet" data-testid="fit-olad-unreachable">
              {$t("lighting.fit.output.unreachable", {
                values: { port: fit.output.ola_http_port ?? "" },
              })}
            </p>
          {/if}
          {#each fit.output.unpatched as universe (universe)}
            <div class="patch" data-testid="fit-patch-{universe}">
              <p>
                {$t("lighting.fit.output.unpatched", { values: { universe } })}
              </p>
              <div class="patch__row">
                <code class="patch__line">{olaPatchLine(universe)}</code>
                <button
                  type="button"
                  class="btn btn-sm"
                  onclick={() => copyPatch(universe)}
                  >{copied === universe
                    ? $t("lighting.fit.output.copied")
                    : $t("lighting.fit.output.copy")}</button
                >
              </div>
            </div>
          {/each}
          {#if outputMsg}
            <p
              class="fit__msg"
              class:fit__msg--error={!outputMsg.ok}
              role={outputMsg.ok ? "status" : "alert"}
              data-testid="fit-output-msg"
            >
              {outputMsg.text}
            </p>
          {/if}
        </div>
      </section>
    </div>

    <p class="fit__footer" data-testid="fit-footer">
      {$t("lighting.fit.footer", {
        values: { found: found ?? 0, total: fit.groups.length },
      })}
    </p>
  {/if}
</div>

<style>
  .fit {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .fit__lede {
    margin: 0;
    color: var(--text-muted);
  }
  .fit__quiet {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
  .fit__error {
    margin: 0;
    padding: 12px 16px;
    border: 1px solid var(--border-danger);
    background: var(--bg-danger);
    border-radius: var(--nc-radius-md);
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    align-items: center;
  }
  .fit__columns {
    display: grid;
    grid-template-columns: minmax(200px, 270px) minmax(0, 1fr) minmax(
        260px,
        340px
      );
    gap: 16px;
    align-items: start;
  }
  .fit__col {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .fit__title {
    margin: 0;
    font-family: var(--nc-font-display);
    font-size: 16px;
    font-weight: 700;
  }
  .fit__legend {
    margin-top: 2px;
  }
  .fit__footer {
    margin: 0;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    color: var(--text-muted);
  }

  .groups {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .group {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: left;
    padding: 10px 12px;
    background: var(--card-bg);
    color: var(--text);
    border: 1px solid var(--card-border);
    border-left: 4px solid var(--green);
    border-radius: var(--nc-radius-md);
    cursor: pointer;
    font: inherit;
  }
  .group--empty {
    border-left-color: var(--yellow);
  }
  .group--selected {
    border-color: var(--accent);
    border-left-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .group__name {
    font-weight: 700;
    overflow-wrap: anywhere;
  }
  .group__state {
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }
  .group__needs,
  .group__songs {
    font-size: 12px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .fix {
    padding: 12px 14px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .fix p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .fix__title {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    overflow-wrap: anywhere;
  }
  .fix__sub {
    margin: 0;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .fix__actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .suggestion {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    background: var(--inset-bg);
    border-left: 3px solid var(--accent);
    border-radius: var(--nc-radius-xs);
  }
  .fit__count {
    font-size: 13px;
    font-weight: 600;
  }
  .picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .clusters {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cluster {
    width: 100%;
    text-align: left;
    white-space: normal;
  }
  .cluster[aria-pressed="true"] {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .fixtures {
    margin: 0;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--nc-radius-xs);
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 240px;
    overflow-y: auto;
    min-width: 0;
  }
  .fixtures legend {
    font-size: 12px;
    font-weight: 700;
    padding: 0 4px;
  }
  .fixtures__row {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 8px;
    align-items: baseline;
  }
  .wanted {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .wanted__row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    align-items: center;
    justify-content: space-between;
  }
  .wanted__row > span {
    flex: 1 1 160px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .patch {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .patch__row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }
  .patch__line {
    flex: 1 1 200px;
    min-width: 0;
    padding: 6px 8px;
    background: var(--inset-bg);
    border-radius: var(--nc-radius-xs);
    font-size: 12px;
    overflow-wrap: anywhere;
    user-select: all;
  }
  .fit__msg {
    font-size: 13px;
    color: var(--green);
  }
  .fit__msg--error {
    color: var(--red);
  }

  @media (max-width: 960px) {
    .fit__columns {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
