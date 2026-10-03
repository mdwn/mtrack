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
   * "Your settings for this fixture" (lighting UI design §12.4): the
   * things about a fixture from a GDTF that are the user's — the name
   * venues and shows call it by, its movement limits and its strobe
   * curve — as a form. (Its
   * mode is each venue fixture's own choice.) A save is planned first: a
   * rename says which venue lines it rewrites, and the user confirms before
   * anything is written. mtrack keeps these in a record of its own; the user never sees
   * or edits it as a file.
   */
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { showConfirm } from "../../lib/dialog.svelte";
  import { guardUnsaved } from "../../lib/dirtyGuard";
  import {
    ConflictError,
    fetchFixtureSettings,
    postFixtureSettings,
    type FixtureSettingsData,
    type FixtureSettingsResult,
    type GdtfMode,
    type StrobeCurve,
  } from "../../lib/api/config";

  interface Props {
    name: string;
    dir?: string;
    venuesDir?: string;
    /** The archive's modes: whether any can pan or tilt. */
    modes: GdtfMode[];
    /** After a save, with the type's (possibly new) name. */
    onsaved?: (name: string, notice: { ok: boolean; text: string }) => void;
    /** What the last save said, kept by the page across the reload a rename
     *  makes (this form is rebuilt for the new name). */
    notice?: { ok: boolean; text: string } | null;
  }

  let { name, dir, venuesDir, modes, onsaved, notice = null }: Props = $props();

  let saved = $state<FixtureSettingsData | null>(null);
  let loadError = $state<string | null>(null);
  let typeName = $state("");
  /** Bound to number inputs: a number, or null/undefined when empty. */
  let pan = $state<number | null | undefined>(null);
  let tilt = $state<number | null | undefined>(null);
  /** The stated strobe curve; "" is automatic. */
  let curve = $state<"" | StrobeCurve>("");
  let saving = $state(false);
  let message = $state<{ ok: boolean; text: string } | null>(null);

  const asNumber = (v: number | string | null | undefined) =>
    v === null || v === undefined || v === "" ? null : Number(v);

  function reset(from: FixtureSettingsData) {
    saved = from;
    typeName = from.name;
    pan = from.movement.max_pan_speed;
    tilt = from.movement.max_tilt_speed;
    curve = from.strobe_curve ?? "";
  }

  let ask = 0;
  async function load() {
    const mine = ++ask;
    loadError = null;
    try {
      const answer = await fetchFixtureSettings(name, dir);
      if (mine === ask) reset(answer);
    } catch (e) {
      if (mine === ask) loadError = e instanceof Error ? e.message : String(e);
    }
  }
  $effect(() => {
    void name;
    void dir;
    void load();
  });

  /** Whether the fixture can pan or tilt in any mode. */
  let canMove = $derived(
    modes.some((m) => (m.capabilities ?? []).includes("pan_tilt")),
  );

  // Leaving with unsaved settings asks first (the app's guard, and the
  // browser's for a reload or a closed tab).
  $effect(() =>
    guardUnsaved(
      () => dirty,
      get(t)("lighting.discard.fixture", { values: { name } }),
    ),
  );

  let dirty = $derived(
    !!saved &&
      (typeName.trim() !== saved.name ||
        asNumber(pan) !== saved.movement.max_pan_speed ||
        asNumber(tilt) !== saved.movement.max_tilt_speed ||
        (curve || null) !== saved.strobe_curve),
  );

  let speedsValid = $derived(
    [pan, tilt].every((v) => {
      const n = asNumber(v);
      return n === null || (Number.isFinite(n) && n > 0);
    }),
  );

  function body(write: boolean, versions?: Record<string, string>) {
    return {
      name: typeName.trim(),
      movement: canMove
        ? { max_pan_speed: asNumber(pan), max_tilt_speed: asNumber(tilt) }
        : (saved?.movement ?? { max_pan_speed: null, max_tilt_speed: null }),
      strobe_curve: saved?.strobe
        ? curve || null
        : (saved?.strobe_curve ?? null),
      write,
      ...(versions ? { venue_versions: versions } : {}),
    };
  }

  /** What the save will do, in words, for the confirm; null when there is
   *  nothing to ask about. */
  function consequences(plan: FixtureSettingsResult): string | null {
    const tr = get(t);
    const lines: string[] = [];
    if (plan.rename && plan.rename.lines > 0) {
      lines.push(
        tr("lighting.settings.confirmRename", {
          values: {
            from: plan.rename.from,
            to: plan.rename.to,
            count: plan.rename.lines,
            venues: plan.rename.venues
              .map((v) => `${v.venue} (${v.lines})`)
              .join(", "),
          },
        }),
      );
    }
    if (plan.config_references.length > 0) {
      lines.push(
        tr("lighting.settings.confirmConfig", {
          values: { names: plan.config_references.join(", ") },
        }),
      );
    }
    return lines.length > 0 ? lines.join("\n\n") : null;
  }

  async function save() {
    if (!saved || !dirty || saving) return;
    notice = null;
    const tr = get(t);
    if (!typeName.trim()) {
      message = { ok: false, text: tr("lighting.settings.nameRequired") };
      return;
    }
    if (!speedsValid) {
      message = { ok: false, text: tr("lighting.settings.speedInvalid") };
      return;
    }
    saving = true;
    message = null;
    const dirs = { dir, venuesDir };
    try {
      const plan = await postFixtureSettings(
        name,
        body(false),
        dirs,
        saved.version,
      );
      const asked = consequences(plan);
      if (asked) {
        const go = await showConfirm(
          `${asked}\n\n${tr("lighting.settings.confirmQuestion")}`,
          { confirmLabel: tr("lighting.settings.saveAnyway") },
        );
        if (!go) {
          message = { ok: false, text: tr("lighting.settings.notSaved") };
          return;
        }
      }
      const done = await postFixtureSettings(
        name,
        body(true, plan.venue_versions),
        dirs,
        saved.version,
      );
      const said: string[] = [tr("common.saved")];
      if (done.rename && done.rename.lines > 0) {
        said.push(
          tr("lighting.settings.renamed", {
            values: {
              count: done.rename.lines,
              venues: done.rename.venues
                .map((v) => `${v.venue} (${v.lines})`)
                .join(", "),
            },
          }),
        );
      }
      if (done.venue_error) {
        message = {
          ok: false,
          text: tr("lighting.venueError.saved", {
            values: { ...done.venue_error },
          }),
        };
      } else {
        message = { ok: true, text: said.join(" ") };
      }
      const next = done.rename?.to ?? typeName.trim();
      // Saved: the form is clean before anything navigates (a rename moves
      // the page to the new name's address, which must not ask).
      const sent = body(true);
      saved = {
        ...saved,
        name: next,
        movement: sent.movement,
        strobe_curve: sent.strobe_curve,
        version: done.version,
      };
      onsaved?.(next, message);
      if (next === name) await load();
    } catch (e) {
      if (e instanceof ConflictError) await load();
      message = {
        ok: false,
        text: e instanceof Error ? e.message : String(e),
      };
    } finally {
      saving = false;
    }
  }

  function discard() {
    if (saved) reset(saved);
    message = null;
  }

  let locked = $derived(saving);
</script>

<section
  class="settings"
  aria-labelledby="ft-settings-title"
  data-testid="ft-settings"
>
  <h5 class="settings__title" id="ft-settings-title">
    {$t("lighting.settings.title")}
  </h5>
  {#if loadError}
    <p class="settings__error">{loadError}</p>
  {:else if saved}
    <div class="settings__grid">
      <div class="settings__field">
        <label for="ft-set-name">{$t("lighting.settings.name")}</label>
        <input
          id="ft-set-name"
          class="input"
          bind:value={typeName}
          disabled={locked}
          data-testid="ft-set-name"
        />
        <span class="field-hint">{$t("lighting.settings.nameHint")}</span>
      </div>
      <div class="settings__field">
        <span class="settings__label">{$t("lighting.settings.movement")}</span>
        {#if canMove}
          <div class="settings__speeds">
            <label>
              <span>{$t("lighting.settings.maxPan")}</span>
              <input
                class="input"
                type="number"
                min="0"
                step="any"
                bind:value={pan}
                disabled={locked}
                data-testid="ft-set-pan"
              />
            </label>
            <label>
              <span>{$t("lighting.settings.maxTilt")}</span>
              <input
                class="input"
                type="number"
                min="0"
                step="any"
                bind:value={tilt}
                disabled={locked}
                data-testid="ft-set-tilt"
              />
            </label>
          </div>
          <span class="field-hint">{$t("lighting.settings.movementHint")}</span>
        {:else}
          <div class="settings__na" data-testid="ft-set-not-moving">
            {$t("lighting.settings.notMoving")}
          </div>
        {/if}
      </div>
      {#if saved.strobe}
        {@const steps = saved.strobe.steps}
        <div class="settings__field">
          <label for="ft-set-strobe"
            >{$t("lighting.settings.strobeCurve")}</label
          >
          <select
            id="ft-set-strobe"
            class="input"
            bind:value={curve}
            disabled={locked}
            data-testid="ft-set-strobe"
          >
            <option value=""
              >{steps > 0
                ? $t("lighting.settings.strobeAutoTable", {
                    values: { steps },
                  })
                : $t("lighting.settings.strobeAutoLinear")}</option
            >
            <option value="period"
              >{$t("lighting.settings.strobePeriod")}</option
            >
            <option value="linear"
              >{$t("lighting.settings.strobeLinear")}</option
            >
            {#if steps > 0}
              <option value="declared"
                >{$t("lighting.settings.strobeDeclared", {
                  values: { steps },
                })}</option
              >
            {/if}
          </select>
          <span class="field-hint" data-testid="ft-set-strobe-hint"
            >{$t("lighting.settings.strobeHint")}</span
          >
        </div>
      {/if}
    </div>
    <div class="settings__actions">
      <button
        class="btn btn-primary"
        type="button"
        disabled={!dirty || locked}
        onclick={save}
        data-testid="ft-set-save"
      >
        {saving ? $t("common.saving") : $t("lighting.settings.save")}
      </button>
      {#if dirty}
        <button
          class="btn"
          type="button"
          disabled={saving}
          onclick={discard}
          data-testid="ft-set-discard">{$t("common.discard")}</button
        >
      {/if}
      {#if message ?? notice}
        {@const said = (message ?? notice)!}
        <span
          class="settings__msg"
          class:settings__msg--error={!said.ok}
          role="status"
          data-testid="ft-set-msg">{said.text}</span
        >
      {/if}
    </div>
  {/if}
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border-top: 1px solid var(--border);
    padding-top: 14px;
  }
  .settings__title,
  .settings__label,
  .settings__field > label {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--text-dim);
  }
  .settings__field > label,
  .settings__label {
    letter-spacing: 0.5px;
  }
  .settings__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 12px;
  }
  .settings__field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .settings__speeds {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .settings__speeds label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .settings__na {
    font-size: 13px;
    color: var(--text-dim);
    border: 1px dashed var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
  }
  .settings__actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .settings__msg {
    font-size: 13px;
    color: var(--green);
  }
  .settings__msg--error,
  .settings__error {
    color: var(--red);
    font-size: 13px;
    margin: 0;
  }
  .field-hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
  }
</style>
