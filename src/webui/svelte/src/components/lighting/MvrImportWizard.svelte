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
  import { t } from "svelte-i18n";
  import {
    importMvr,
    inspectMvr,
    type MvrHandEdit,
    type MvrImportReport,
    type MvrInspection,
    type MvrKeep,
    type MvrPlan,
    type Vec3Mm,
  } from "../../lib/api/config";
  import {
    defaultVenueName,
    fromPage,
    pageY,
    planBox,
    suggestOrigin,
  } from "../../lib/lighting/mvrPlan";

  interface Props {
    /** Directory overrides (a profile's `lighting.directories`). */
    fixtureTypesDir?: string;
    venuesDir?: string;
  }

  let { fixtureTypesDir = "", venuesDir = "" }: Props = $props();
  let dirs = $derived({
    fixtureTypesDir: fixtureTypesDir || undefined,
    venuesDir: venuesDir || undefined,
  });

  const STEPS = ["file", "origin", "review", "import"] as const;
  type Step = (typeof STEPS)[number];

  let step = $state<Step>("file");
  let file = $state<File | null>(null);
  let venueName = $state("");
  /** The name the last inspect ran with; a changed name re-inspects. */
  let inspectedName = "";
  let inspection = $state<MvrInspection | null>(null);
  let busy = $state(false);
  let error = $state("");
  let dragging = $state(false);

  // The origin, in the file's millimeters, as the three fields hold it.
  let originX = $state<number | null>(null);
  let originY = $state<number | null>(null);
  let originZ = $state<number | null>(null);
  let origin = $derived<Vec3Mm | null>(
    originX !== null && originY !== null && originZ !== null
      ? [originX, originY, originZ]
      : null,
  );

  let plan = $state<MvrPlan | null>(null);
  /** Which hand edits to keep, by "fixture\u0000field" (a focus point is
   *  "\u0000focus\u0000name"). Position and rotation start kept; patch and
   *  type are rig facts the venue owns, so they start as the MVR's. */
  let keeping = $state<Record<string, boolean>>({});
  let report = $state<MvrImportReport | null>(null);

  let scene = $derived(inspection?.scene ?? null);
  let suggestion = $derived(scene ? suggestOrigin(scene) : null);
  let box = $derived(scene ? planBox(scene) : null);

  function setOrigin(o: Vec3Mm) {
    [originX, originY, originZ] = o;
  }

  async function choose(f: File | null | undefined) {
    if (!f) return;
    file = f;
    venueName = defaultVenueName(f.name);
    inspection = null;
    plan = null;
    report = null;
    originX = originY = originZ = null;
    await runInspect();
  }

  async function runInspect() {
    if (!file) return;
    busy = true;
    error = "";
    try {
      const name = venueName.trim();
      inspection = await inspectMvr(file, name || undefined, dirs);
      inspectedName = name;
    } catch (e) {
      inspection = null;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function onFileInput(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    choose(input.files?.[0]);
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    choose(e.dataTransfer?.files?.[0]);
  }

  async function toOrigin() {
    // A changed name can change what the importer refuses (a name that is
    // another venue's), so it is checked before the plan is drawn.
    if (venueName.trim() !== inspectedName) {
      await runInspect();
      if (!inspection) return;
    }
    if (origin === null && suggestion) setOrigin(suggestion.origin);
    if (origin === null && !suggestion) setOrigin([0, 0, 0]);
    step = "origin";
  }

  async function toReview() {
    if (!file || !origin) return;
    busy = true;
    error = "";
    try {
      const result = await importMvr(
        file,
        { name: venueName.trim(), origin, write: false },
        dirs,
      );
      plan = result.plan ?? null;
      keeping = defaultKeeping(plan);
      step = "review";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  const EDIT_SEP = "\u0000";

  function editKey(fixture: string, field: string): string {
    return `${fixture}${EDIT_SEP}${field}`;
  }

  function focusKey(name: string): string {
    return `${EDIT_SEP}focus${EDIT_SEP}${name}`;
  }

  function defaultKeeping(p: MvrPlan | null): Record<string, boolean> {
    const out: Record<string, boolean> = {};
    for (const f of p?.fixtures ?? []) {
      for (const e of f.overwrites ?? []) {
        out[editKey(f.name, e.field)] =
          e.field === "position" || e.field === "rotation";
      }
    }
    for (const f of p?.focus_points ?? []) {
      if (f.overwrites?.length) out[focusKey(f.name)] = true;
    }
    return out;
  }

  /** The keep list the import request carries. */
  function keepRequest(): MvrKeep {
    const fixtures: Record<string, string[]> = {};
    for (const f of plan?.fixtures ?? []) {
      const kept = (f.overwrites ?? [])
        .map((e) => e.field)
        .filter((field) => keeping[editKey(f.name, field)]);
      if (kept.length > 0) fixtures[f.name] = kept;
    }
    const focus_points = (plan?.focus_points ?? [])
      .filter((f) => f.overwrites?.length && keeping[focusKey(f.name)])
      .map((f) => f.name);
    return { fixtures, focus_points };
  }

  function fieldLabel(e: MvrHandEdit): string {
    return $t(`lighting.mvr.field.${e.field}`);
  }

  async function doImport() {
    if (!file || !origin) return;
    step = "import";
    busy = true;
    error = "";
    report = null;
    try {
      const result = await importMvr(
        file,
        { name: venueName.trim(), origin, write: true, keep: keepRequest() },
        dirs,
      );
      report = result.report ?? null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function restart() {
    step = "file";
    file = null;
    venueName = "";
    inspection = null;
    plan = null;
    report = null;
    error = "";
    originX = originY = originZ = null;
  }

  // --- The plan ---

  let svgEl: SVGSVGElement | undefined = $state();

  function onPlanClick(e: MouseEvent) {
    if (!svgEl || !box) return;
    const ctm = svgEl.getScreenCTM();
    if (!ctm) return;
    const pt = svgEl.createSVGPoint();
    pt.x = e.clientX;
    pt.y = e.clientY;
    const local = pt.matrixTransform(ctm.inverse());
    const [x, y] = fromPage(box, local.x, local.y);
    // A click picks x and y; z stays where it was (the deck top, or the floor).
    setOrigin([x, y, originZ ?? suggestion?.origin[2] ?? 0]);
  }

  function onPlanKey(e: KeyboardEvent) {
    const amount = e.shiftKey ? 1000 : 100;
    const delta: Record<string, [number, number]> = {
      ArrowLeft: [-amount, 0],
      ArrowRight: [amount, 0],
      ArrowUp: [0, amount],
      ArrowDown: [0, -amount],
    };
    if (e.key in delta && origin) {
      e.preventDefault();
      setOrigin([
        origin[0] + delta[e.key][0],
        origin[1] + delta[e.key][1],
        origin[2],
      ]);
    } else if ((e.key === "Enter" || e.key === " ") && suggestion) {
      e.preventDefault();
      setOrigin(suggestion.origin);
    }
  }

  /** Marker sizes follow the plan's span, so they read at any scale. */
  let unit = $derived(
    box ? Math.max(box.maxX - box.minX, box.maxY - box.minY) / 100 : 1,
  );

  // --- The summaries ---

  let fixtureCount = $derived(inspection?.report.fixtures.length ?? 0);
  let typesWithMode = $derived(inspection?.report.fixture_types.length ?? 0);
  let withoutMode = $derived(
    inspection?.report.fixtures.filter((f) => f.todo).length ?? 0,
  );
  let universes = $derived(
    new Set(
      (inspection?.report.fixtures ?? [])
        .map((f) => f.patch?.[0])
        .filter((u) => u !== undefined),
    ).size,
  );

  let newTypes = $derived(plan?.fixture_types.filter((x) => !x.existing) ?? []);
  let existingTypes = $derived(
    plan?.fixture_types.filter((x) => x.existing) ?? [],
  );
  let seeded = $derived(plan?.fixtures.filter((f) => !f.todo) ?? []);
  let todos = $derived(plan?.fixtures.filter((f) => f.todo) ?? []);
  let edited = $derived(
    plan?.fixtures.filter((f) => f.overwrites?.length) ?? [],
  );
  let editedFocus = $derived(
    plan?.focus_points.filter((f) => f.overwrites?.length) ?? [],
  );

  function metres(mm: number): string {
    return (mm / 1000).toFixed(2);
  }
</script>

<div class="wizard" data-testid="mvr-wizard">
  <div class="wizard__head">
    <h2 class="wizard__title">{$t("lighting.mvr.import.title")}</h2>
    <p class="wizard__lede">{$t("lighting.mvr.import.lede")}</p>
  </div>

  <ol class="stepper" aria-label={$t("lighting.mvr.import.stepperLabel")}>
    {#each STEPS as s, i (s)}
      <li
        class="stepper__item"
        class:stepper__item--current={step === s}
        class:stepper__item--done={STEPS.indexOf(step) > i}
        aria-current={step === s ? "step" : undefined}
        data-testid="mvr-step-{s}"
      >
        <span class="stepper__num" aria-hidden="true">{i + 1}</span>
        <span class="stepper__label">{$t(`lighting.mvr.step.${s}`)}</span>
      </li>
    {/each}
  </ol>

  {#if error}
    <div class="wizard__error" role="alert" data-testid="mvr-error">
      <span>{error}</span>
      {#if step === "file" && file}
        <button class="btn btn-sm" onclick={runInspect}
          >{$t("common.retry")}</button
        >
      {:else if step === "origin"}
        <button class="btn btn-sm" onclick={toReview}
          >{$t("common.retry")}</button
        >
      {:else if step === "import"}
        <button class="btn btn-sm" onclick={doImport}
          >{$t("common.retry")}</button
        >
      {/if}
    </div>
  {/if}

  {#if step === "file"}
    <section class="panel" aria-labelledby="mvr-file-title">
      <h3 id="mvr-file-title" class="panel__title">
        {$t("lighting.mvr.step.file")}
      </h3>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="drop"
        class:drop--over={dragging}
        ondragover={(e) => {
          e.preventDefault();
          dragging = true;
        }}
        ondragleave={() => (dragging = false)}
        ondrop={onDrop}
        data-testid="mvr-drop"
      >
        <span>{$t("lighting.mvr.file.drop")}</span>
        <label class="btn drop__choose">
          {$t("lighting.mvr.file.choose")}
          <input
            class="drop__input"
            type="file"
            accept=".mvr"
            aria-label={$t("lighting.mvr.file.label")}
            onchange={onFileInput}
            data-testid="mvr-file"
          />
        </label>
        {#if file}
          <span class="drop__file" data-testid="mvr-file-name">{file.name}</span
          >
        {/if}
      </div>

      {#if busy}
        <p class="muted" data-testid="mvr-reading">
          {$t("lighting.mvr.file.reading")}
        </p>
      {/if}

      {#if inspection}
        <div class="field">
          <label for="mvr-venue-name">{$t("lighting.mvr.file.name")}</label>
          <input
            id="mvr-venue-name"
            class="input"
            bind:value={venueName}
            data-testid="mvr-venue-name"
          />
          <span class="hint">{$t("lighting.mvr.file.nameHint")}</span>
        </div>

        <h4 class="panel__sub">{$t("lighting.mvr.file.holds")}</h4>
        <dl class="facts" data-testid="mvr-holds">
          <div>
            <dt>{$t("lighting.mvr.file.fixtures")}</dt>
            <dd data-testid="mvr-count-fixtures">{fixtureCount}</dd>
          </div>
          <div>
            <dt>{$t("lighting.mvr.file.typesWithMode")}</dt>
            <dd data-testid="mvr-count-types">{typesWithMode}</dd>
          </div>
          <div>
            <dt>{$t("lighting.mvr.file.withoutMode")}</dt>
            <dd data-testid="mvr-count-without">{withoutMode}</dd>
          </div>
          <div>
            <dt>{$t("lighting.mvr.file.universes")}</dt>
            <dd>{universes}</dd>
          </div>
          <div>
            <dt>{$t("lighting.mvr.file.focusPoints")}</dt>
            <dd>{inspection.scene.focus_points.length}</dd>
          </div>
          <div>
            <dt>{$t("lighting.mvr.file.scenery")}</dt>
            <dd>{inspection.report.scenery_objects}</dd>
          </div>
        </dl>

        <div class="actions">
          <button
            class="btn btn-primary"
            onclick={toOrigin}
            disabled={busy || !venueName.trim()}
            data-testid="mvr-continue"
          >
            {$t("lighting.mvr.continue")}
          </button>
        </div>
      {/if}
    </section>
  {:else if step === "origin"}
    <section class="panel" aria-labelledby="mvr-origin-title">
      <h3 id="mvr-origin-title" class="panel__title">
        {$t("lighting.mvr.step.origin")}
      </h3>
      <p class="panel__lede">{$t("lighting.mvr.origin.instruction")}</p>

      {#if box && scene}
        <svg
          bind:this={svgEl}
          class="plan"
          viewBox="{box.minX} {box.minY} {box.maxX - box.minX} {box.maxY -
            box.minY}"
          role="button"
          tabindex="0"
          aria-label={$t("lighting.mvr.origin.planLabel")}
          onclick={onPlanClick}
          onkeydown={onPlanKey}
          data-testid="mvr-plan"
        >
          {#each scene.scenery as s, i (i)}
            {#if s.bounds_mm}
              <rect
                class={s.deck ? "plan__deck" : "plan__scenery"}
                x={s.bounds_mm.min[0]}
                y={pageY(box, s.bounds_mm.max[1])}
                width={s.bounds_mm.max[0] - s.bounds_mm.min[0]}
                height={s.bounds_mm.max[1] - s.bounds_mm.min[1]}
                data-testid={s.deck ? "mvr-plan-deck" : "mvr-plan-scenery"}
              />
            {/if}
          {/each}
          {#each scene.focus_points as f, i (i)}
            {#if f.position_mm}
              <rect
                class="plan__focus"
                x={f.position_mm[0] - unit}
                y={pageY(box, f.position_mm[1]) - unit}
                width={unit * 2}
                height={unit * 2}
                transform="rotate(45 {f.position_mm[0]} {pageY(
                  box,
                  f.position_mm[1],
                )})"
              />
            {/if}
          {/each}
          {#each scene.fixtures as f, i (i)}
            {#if f.position_mm}
              <circle
                class="plan__fixture"
                cx={f.position_mm[0]}
                cy={pageY(box, f.position_mm[1])}
                r={unit * 0.9}
              />
            {/if}
          {/each}
          {#if suggestion}
            <circle
              class="plan__suggest"
              cx={suggestion.origin[0]}
              cy={pageY(box, suggestion.origin[1])}
              r={unit * 2.2}
              data-testid="mvr-plan-suggestion"
            />
          {/if}
          {#if origin}
            <g class="plan__origin" data-testid="mvr-plan-origin">
              <line
                x1={origin[0] - unit * 3}
                x2={origin[0] + unit * 3}
                y1={pageY(box, origin[1])}
                y2={pageY(box, origin[1])}
              />
              <line
                x1={origin[0]}
                x2={origin[0]}
                y1={pageY(box, origin[1]) - unit * 3}
                y2={pageY(box, origin[1]) + unit * 3}
              />
            </g>
          {/if}
        </svg>
        <ul class="legend" aria-hidden="true">
          <li>
            <span class="key key--fixture"></span>{$t(
              "lighting.mvr.origin.fixture",
            )}
          </li>
          <li>
            <span class="key key--focus"></span>{$t(
              "lighting.mvr.origin.focus",
            )}
          </li>
          {#if scene.deck_mm}
            <li>
              <span class="key key--deck"></span>{$t(
                "lighting.mvr.origin.deck",
              )}
            </li>
          {/if}
          <li>
            <span class="key key--suggest"></span>{$t(
              "lighting.mvr.origin.suggested",
            )}
          </li>
          <li>
            <span class="key key--origin"></span>{$t(
              "lighting.mvr.origin.origin",
            )}
          </li>
        </ul>
        <p class="hint">{$t("lighting.mvr.origin.keys")}</p>
      {:else}
        <p class="muted" data-testid="mvr-no-plan">
          {$t("lighting.mvr.origin.noPositions")}
        </p>
      {/if}

      {#if suggestion}
        <div class="suggest" data-testid="mvr-suggestion">
          <span>
            {$t(`lighting.mvr.origin.suggestion.${suggestion.basis}`)}:
            <strong data-testid="mvr-suggestion-value"
              >{suggestion.origin.join(", ")}</strong
            >
            mm
          </span>
          <button
            class="btn btn-sm"
            onclick={() => setOrigin(suggestion.origin)}
            data-testid="mvr-use-suggestion"
          >
            {$t("lighting.mvr.origin.use")}
          </button>
        </div>
      {/if}

      <fieldset class="origin">
        <legend>{$t("lighting.mvr.origin.chosen")}</legend>
        <div class="field">
          <label for="mvr-origin-x">{$t("lighting.mvr.origin.x")}</label>
          <input
            id="mvr-origin-x"
            class="input"
            type="number"
            step="1"
            bind:value={originX}
            data-testid="mvr-origin-x"
          />
        </div>
        <div class="field">
          <label for="mvr-origin-y">{$t("lighting.mvr.origin.y")}</label>
          <input
            id="mvr-origin-y"
            class="input"
            type="number"
            step="1"
            bind:value={originY}
            data-testid="mvr-origin-y"
          />
        </div>
        <div class="field">
          <label for="mvr-origin-z">{$t("lighting.mvr.origin.z")}</label>
          <input
            id="mvr-origin-z"
            class="input"
            type="number"
            step="1"
            bind:value={originZ}
            data-testid="mvr-origin-z"
          />
        </div>
      </fieldset>

      <div class="actions">
        <button class="btn" onclick={() => (step = "file")}
          >{$t("lighting.mvr.back")}</button
        >
        <button
          class="btn btn-primary"
          onclick={toReview}
          disabled={busy || !origin}
          data-testid="mvr-to-review"
        >
          {busy ? $t("lighting.mvr.working") : $t("lighting.mvr.continue")}
        </button>
      </div>
    </section>
  {:else if step === "review" && plan}
    <section class="panel" aria-labelledby="mvr-review-title">
      <h3 id="mvr-review-title" class="panel__title">
        {$t("lighting.mvr.step.review")}
      </h3>
      <p class="banner" data-testid="mvr-mode" data-merge={plan.merge}>
        {#if plan.merge}
          {$t("lighting.mvr.review.merge", {
            values: { name: plan.venue_name },
          })}
        {:else}
          {$t("lighting.mvr.review.seed", {
            values: { name: plan.venue_name },
          })}
        {/if}
      </p>
      <p class="muted" data-testid="mvr-review-origin">
        {$t("lighting.mvr.review.origin", {
          values: {
            x: metres(origin?.[0] ?? 0),
            y: metres(origin?.[1] ?? 0),
            z: metres(origin?.[2] ?? 0),
          },
        })}
      </p>

      <dl class="facts">
        <div>
          <dt>{$t("lighting.mvr.review.seedCount")}</dt>
          <dd data-testid="mvr-review-seeded">{seeded.length}</dd>
        </div>
        <div>
          <dt>{$t("lighting.mvr.review.focusCount")}</dt>
          <dd>{plan.focus_points.length}</dd>
        </div>
        <div>
          <dt>{$t("lighting.mvr.review.scenery")}</dt>
          <dd data-testid="mvr-review-scenery">
            {$t("lighting.mvr.review.sceneryValue", {
              values: {
                objects: plan.scenery_objects,
                undrawn: plan.scenery_meshes_undrawn,
              },
            })}
          </dd>
        </div>
      </dl>

      <h4 class="panel__sub">
        {$t("lighting.mvr.review.typesNew", {
          values: { count: newTypes.length },
        })}
      </h4>
      <ul class="list" data-testid="mvr-review-types-new">
        {#each newTypes as ft (ft.name)}
          <li>{ft.name} <span class="muted">({ft.mode})</span></li>
        {/each}
      </ul>
      {#if existingTypes.length > 0}
        <h4 class="panel__sub">
          {$t("lighting.mvr.review.typesExisting", {
            values: { count: existingTypes.length },
          })}
        </h4>
        <ul class="list" data-testid="mvr-review-types-existing">
          {#each existingTypes as ft (ft.name)}
            <li>{ft.name} <span class="muted">({ft.mode})</span></li>
          {/each}
        </ul>
      {/if}

      {#if todos.length > 0}
        <h4 class="panel__sub">
          {$t("lighting.mvr.review.todo", { values: { count: todos.length } })}
        </h4>
        <ul class="list" data-testid="mvr-review-todos">
          {#each todos as f (f.name)}
            <li><strong>{f.name}</strong>: {f.todo}</li>
          {/each}
        </ul>
      {/if}

      {#if plan.merge}
        <h4 class="panel__sub">{$t("lighting.mvr.review.mergeKeeps")}</h4>
        <ul class="list" data-testid="mvr-review-merge">
          {#if plan.kept_fixtures.length > 0}
            <li>
              {$t("lighting.mvr.review.keptFixtures", {
                values: { names: plan.kept_fixtures.join(", ") },
              })}
            </li>
          {/if}
          {#if plan.removed_fixtures.length > 0}
            <li>
              {$t("lighting.mvr.review.removedFixtures", {
                values: {
                  names: plan.removed_fixtures.map((f) => f.name).join(", "),
                },
              })}
            </li>
          {/if}
          {#if plan.kept_focus_points.length > 0}
            <li>
              {$t("lighting.mvr.review.keptFocus", {
                values: { names: plan.kept_focus_points.join(", ") },
              })}
            </li>
          {/if}
          <li>{$t("lighting.mvr.review.keepsTags")}</li>
        </ul>
      {/if}

      {#if edited.length > 0 || editedFocus.length > 0}
        <h4 class="panel__sub">
          {$t("lighting.mvr.review.edits", {
            values: { count: edited.length + editedFocus.length },
          })}
        </h4>
        <p class="muted">{$t("lighting.mvr.review.editsHint")}</p>
        <ul class="list list--edits" data-testid="mvr-review-edits">
          {#each edited as f (f.name)}
            {#each f.overwrites ?? [] as e (e.field)}
              <li data-testid="mvr-edit-{f.name}-{e.field}">
                <label class="check">
                  <input
                    type="checkbox"
                    bind:checked={keeping[editKey(f.name, e.field)]}
                    data-testid="mvr-keep-{f.name}-{e.field}"
                  />
                  <span>
                    <strong>{f.name}</strong>
                    {$t("lighting.mvr.review.editRow", {
                      values: {
                        field: fieldLabel(e),
                        mine: e.mine,
                        mvr: e.mvr,
                      },
                    })}
                    <em>{$t("lighting.mvr.review.keepEdit")}</em>
                  </span>
                </label>
              </li>
            {/each}
          {/each}
          {#each editedFocus as f (f.name)}
            {#each f.overwrites ?? [] as e (e.field)}
              <li data-testid="mvr-edit-focus-{f.name}">
                <label class="check">
                  <input
                    type="checkbox"
                    bind:checked={keeping[focusKey(f.name)]}
                    data-testid="mvr-keep-focus-{f.name}"
                  />
                  <span>
                    <strong>{f.name}</strong>
                    {$t("lighting.mvr.review.editRow", {
                      values: {
                        field: fieldLabel(e),
                        mine: e.mine,
                        mvr: e.mvr,
                      },
                    })}
                    <em>{$t("lighting.mvr.review.keepEdit")}</em>
                  </span>
                </label>
              </li>
            {/each}
          {/each}
        </ul>
      {/if}

      {#if plan.warnings.length > 0}
        <h4 class="panel__sub">{$t("lighting.mvr.review.warnings")}</h4>
        <ul class="list list--warn" data-testid="mvr-review-warnings">
          {#each plan.warnings as w, i (i)}
            <li>{w}</li>
          {/each}
        </ul>
      {/if}

      <div class="actions">
        <button class="btn" onclick={() => (step = "origin")}
          >{$t("lighting.mvr.back")}</button
        >
        <button
          class="btn btn-primary"
          onclick={doImport}
          disabled={busy}
          data-testid="mvr-do-import"
        >
          {$t("lighting.mvr.review.import")}
        </button>
      </div>
    </section>
  {:else if step === "import"}
    <section class="panel" aria-labelledby="mvr-done-title">
      <h3 id="mvr-done-title" class="panel__title">
        {$t("lighting.mvr.step.import")}
      </h3>
      {#if busy}
        <p class="muted" data-testid="mvr-importing">
          {$t("lighting.mvr.working")}
        </p>
      {:else if report}
        <p class="banner banner--ok" data-testid="mvr-done">
          {$t("lighting.mvr.done.title", {
            values: {
              name: report.venue_name,
              fixtures: report.fixtures.filter((f) => !f.todo).length,
            },
          })}
        </p>
        <h4 class="panel__sub">
          {$t("lighting.mvr.done.written", {
            values: { count: report.written.length },
          })}
        </h4>
        <ul class="list" data-testid="mvr-done-written">
          {#each report.written as w (w)}
            <li><code>{w}</code></li>
          {/each}
        </ul>
        {#each Object.entries(report.distillation_warnings).filter(([, w]) => w.length > 0) as [type, warns] (type)}
          <ul class="list list--warn">
            {#each warns as w, i (i)}
              <li>{type}: {w}</li>
            {/each}
          </ul>
        {/each}
        {#if report.warnings.length > 0}
          <ul class="list list--warn">
            {#each report.warnings as w, i (i)}
              <li>{w}</li>
            {/each}
          </ul>
        {/if}
        <p class="hint" data-testid="mvr-not-current">
          {$t("lighting.mvr.done.notCurrent")}
        </p>
        <div class="actions">
          <a
            class="btn btn-primary"
            href="#/lighting/fit"
            data-testid="mvr-go-fit">{$t("lighting.mvr.done.fit")}</a
          >
          <a class="btn" href="#/lighting/venues" data-testid="mvr-go-venues"
            >{$t("lighting.mvr.done.venues")}</a
          >
          <button class="btn btn-ghost" onclick={restart}
            >{$t("lighting.mvr.done.another")}</button
          >
        </div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .wizard {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 860px;
  }
  .wizard__title {
    margin: 0;
    font-family: var(--nc-font-display);
    font-size: 18px;
    font-weight: 700;
  }
  .wizard__lede,
  .panel__lede {
    margin: 4px 0 0;
    color: var(--text-muted);
  }
  .wizard__error {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    align-items: center;
    padding: 12px 16px;
    border: 1px solid var(--border-danger);
    background: var(--bg-danger);
    border-radius: var(--nc-radius-md);
  }

  .stepper {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
  }
  .stepper__item {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
    font-size: 14px;
  }
  .stepper__num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 1px solid var(--border);
    font-size: 12px;
    font-weight: 700;
  }
  .stepper__item--current {
    color: var(--text);
    font-weight: 600;
  }
  .stepper__item--current .stepper__num {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--nc-ink);
  }
  .stepper__item--done .stepper__num {
    border-color: var(--green);
    color: var(--green);
  }

  .panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
  }
  .panel__title,
  .panel__sub {
    margin: 0;
    font-family: var(--nc-font-display);
    font-weight: 700;
  }
  .panel__title {
    font-size: 16px;
  }
  .panel__sub {
    font-size: 14px;
    margin-top: 4px;
  }
  .muted,
  .hint {
    margin: 0;
    color: var(--text-dim);
    font-size: 13px;
  }

  .drop {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    padding: 20px;
    border: 2px dashed var(--border);
    border-radius: var(--nc-radius-md);
  }
  .drop--over {
    border-color: var(--accent);
    background: var(--bg-hover, transparent);
  }
  .drop__choose {
    position: relative;
    cursor: pointer;
  }
  .drop__input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
    width: 100%;
    height: 100%;
  }
  .drop__choose:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .drop__file {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }

  .facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 8px;
    margin: 0;
  }
  .facts div {
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--nc-radius-md);
  }
  .facts dt {
    font-size: 12px;
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    font-size: 18px;
    font-weight: 700;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: flex-end;
  }
  a.btn {
    text-decoration: none;
    display: inline-flex;
    align-items: center;
  }

  .plan {
    width: 100%;
    max-height: 420px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--nc-radius-md);
    cursor: crosshair;
    touch-action: manipulation;
  }
  .plan:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .plan__deck {
    fill: var(--text-dim);
    fill-opacity: 0.25;
    stroke: var(--text-dim);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  .plan__scenery {
    fill: none;
    stroke: var(--text-dim);
    stroke-width: 1;
    stroke-dasharray: 4 3;
    vector-effect: non-scaling-stroke;
  }
  .plan__fixture {
    fill: var(--accent);
  }
  .plan__focus {
    fill: var(--yellow);
  }
  .plan__suggest {
    fill: none;
    stroke: var(--green);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .plan__origin line {
    stroke: var(--red);
    stroke-width: 2.5;
    vector-effect: non-scaling-stroke;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 12px;
    color: var(--text-dim);
  }
  .legend li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .key {
    width: 10px;
    height: 10px;
    display: inline-block;
    border-radius: 50%;
  }
  .key--fixture {
    background: var(--accent);
  }
  .key--focus {
    background: var(--yellow);
    border-radius: 0;
    transform: rotate(45deg);
  }
  .key--deck {
    background: var(--text-dim);
    opacity: 0.4;
    border-radius: 0;
  }
  .key--suggest {
    border: 2px solid var(--green);
  }
  .key--origin {
    background: var(--red);
  }

  .suggest {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border: 1px solid var(--green);
    border-radius: var(--nc-radius-md);
  }
  .origin {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
    border: 1px solid var(--border);
    border-radius: var(--nc-radius-md);
    padding: 8px 12px 12px;
    margin: 0;
  }
  .origin legend {
    font-size: 12px;
    font-weight: 600;
    padding: 0 4px;
    color: var(--text-muted);
  }

  .banner {
    margin: 0;
    padding: 10px 14px;
    border-radius: var(--nc-radius-md);
    border: 1px solid var(--border);
    background: var(--bg);
  }
  .banner--ok {
    border-color: var(--green);
  }
  .list {
    margin: 0;
    padding-left: 20px;
    font-size: 14px;
  }
  .list--warn {
    color: var(--yellow);
  }
  .list--edits {
    list-style: none;
    padding-left: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .check {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .check em {
    font-style: normal;
    color: var(--text-muted);
    margin-left: 6px;
  }

  @media (max-width: 600px) {
    .origin {
      grid-template-columns: 1fr;
    }
    .actions .btn {
      flex: 1 1 auto;
      justify-content: center;
    }
  }
</style>
