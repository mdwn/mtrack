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
  import { onMount, untrack } from "svelte";
  import { t } from "svelte-i18n";
  import { showConfirm } from "../../lib/dialog.svelte";
  import {
    ConflictError,
    addAimPoints,
    downloadMvrExport,
    keepMvrExport,
    fetchMvrExportSummary,
    fetchVenue,
    type MvrExportSummary,
  } from "../../lib/api/config";
  import { fileStem, validExportName } from "../../lib/lighting/mvrPlan";

  interface Props {
    venue: string;
    /** Directory overrides (a profile's `lighting.directories`). */
    fixtureTypesDir?: string;
    venuesDir?: string;
    onclose: () => void;
    /** Called after aim points were added to the venue file. */
    onchanged?: () => void;
  }

  let {
    venue,
    fixtureTypesDir = "",
    venuesDir = "",
    onclose,
    onchanged,
  }: Props = $props();
  let dirs = $derived({
    fixtureTypesDir: fixtureTypesDir || undefined,
    venuesDir: venuesDir || undefined,
  });

  let dialogEl: HTMLDialogElement | undefined = $state();
  let fileName = $state(untrack(() => `${fileStem(venue)}.mvr`));
  let layersFromTags = $state(false);
  let keep = $state(false);
  let summary = $state<MvrExportSummary | null>(null);
  let error = $state("");
  let working = $state(false);
  let addedNote = $state("");
  let keptNote = $state("");
  /** True while the dialog steps out of modal mode to ask a question: the
   *  native close event that causes must not count as the user closing. */
  let asking = false;

  let fileValid = $derived(validExportName(fileName));
  let unlinked = $derived(
    summary?.fixed_fixtures.filter((f) => f.focus === null) ?? [],
  );
  let linkedCount = $derived(
    (summary?.fixed_fixtures.length ?? 0) - unlinked.length,
  );

  /** The venue file's version the summary was read at; adding aim points
   *  is refused if the file has changed since. */
  let venueVersion: string | undefined;

  async function loadSummary() {
    error = "";
    try {
      summary = await fetchMvrExportSummary(venue, dirs);
      venueVersion = await fetchVenue(venue, venuesDir || undefined)
        .then((r) => r.version)
        .catch(() => undefined);
    } catch (e) {
      summary = null;
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(() => {
    dialogEl?.showModal();
    loadSummary();
  });

  async function addAim() {
    working = true;
    error = "";
    addedNote = "";
    try {
      const result = await addAimPoints(venue, dirs, venueVersion);
      addedNote = $t("lighting.mvr.export.added", {
        values: {
          count: result.created.length,
          names: result.created.map((c) => c.name).join(", "),
        },
      });
      if (result.venue_error) {
        // Saved, but the venue no longer loads: said where it happened.
        error = $t("lighting.venueError.saved", {
          values: { ...result.venue_error },
        });
      }
      onchanged?.();
      await loadSummary();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      if (e instanceof ConflictError) {
        // The venue file changed: show the summary as it is now, and let
        // the user press the button again.
        onchanged?.();
        await loadSummary();
        error = $t("lighting.mvr.export.changedElsewhere");
      }
    } finally {
      working = false;
    }
  }

  /** Keeps a copy in the project after the download; a file already there is
   *  replaced only when the user says so. */
  async function keepCopy() {
    const options = { file: fileName.trim(), layersFromTags };
    let result = await keepMvrExport(venue, options, dirs);
    if (result.status === "exists") {
      const path = `lighting/export/${result.existing}`;
      // The app's confirm is a fixed overlay, and a modal <dialog> sits in
      // the browser's top layer above every z-index, so the question would
      // render behind this dialog and could not be answered. Leave modal
      // mode for the question and come back to it after.
      asking = true;
      dialogEl?.close();
      let replace: boolean;
      try {
        replace = await showConfirm(
          $t("lighting.mvr.export.replace", { values: { path } }),
        );
      } finally {
        dialogEl?.showModal();
        asking = false;
      }
      if (!replace) {
        return;
      }
      result = await keepMvrExport(
        venue,
        { ...options, overwrite: true },
        dirs,
      );
    }
    if (result.status === "kept") {
      keptNote = $t("lighting.mvr.export.kept", {
        values: { path: result.path },
      });
    }
  }

  async function download() {
    if (!fileValid) return;
    working = true;
    error = "";
    keptNote = "";
    try {
      const result = await downloadMvrExport(
        venue,
        { file: fileName.trim(), layersFromTags },
        dirs,
      );
      const url = URL.createObjectURL(result.blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = result.fileName;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      if (keep) await keepCopy();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      working = false;
    }
  }
</script>

<dialog
  bind:this={dialogEl}
  class="export"
  aria-labelledby="mvr-export-title"
  onclose={() => {
    if (!asking) onclose();
  }}
  data-testid="mvr-export-dialog"
>
  <h3 id="mvr-export-title" class="export__title">
    {$t("lighting.mvr.export.title", { values: { venue } })}
  </h3>

  <div class="field">
    <label for="mvr-export-file">{$t("lighting.mvr.export.file")}</label>
    <input
      id="mvr-export-file"
      class="input"
      bind:value={fileName}
      aria-invalid={!fileValid}
      aria-describedby={fileValid ? undefined : "mvr-export-file-hint"}
      data-testid="mvr-export-file"
    />
    {#if !fileValid}
      <span id="mvr-export-file-hint" class="hint hint--error"
        >{$t("lighting.mvr.export.fileInvalid")}</span
      >
    {/if}
  </div>

  <label class="check">
    <input
      type="checkbox"
      bind:checked={layersFromTags}
      data-testid="mvr-export-layers"
    />
    {$t("lighting.mvr.export.layers")}
  </label>

  {#if summary}
    <dl class="facts" data-testid="mvr-export-summary">
      <div>
        <dt>{$t("lighting.mvr.export.fixtures")}</dt>
        <dd data-testid="mvr-export-fixtures">
          {$t("lighting.mvr.export.fixturesValue", {
            values: {
              count: summary.fixtures,
              positioned: summary.positioned_fixtures,
            },
          })}
        </dd>
      </div>
      <div>
        <dt>{$t("lighting.mvr.export.gdtfs")}</dt>
        <dd data-testid="mvr-export-gdtfs">
          {$t("lighting.mvr.export.gdtfsValue", {
            values: {
              embedded: summary.embedded_gdtfs.length,
              generated: Object.keys(summary.generated_gdtfs).length,
            },
          })}
        </dd>
      </div>
      <div>
        <dt>{$t("lighting.mvr.export.focusPoints")}</dt>
        <dd>{summary.focus_points}</dd>
      </div>
      <div>
        <dt>{$t("lighting.mvr.export.linked")}</dt>
        <dd data-testid="mvr-export-linked">
          {$t("lighting.mvr.export.linkedValue", {
            values: {
              linked: linkedCount,
              total: summary.fixed_fixtures.length,
            },
          })}
        </dd>
      </div>
    </dl>

    {#if summary.warnings.length > 0}
      <ul class="list" data-testid="mvr-export-warnings">
        {#each summary.warnings as w, i (i)}
          <li>{w}</li>
        {/each}
      </ul>
    {/if}

    {#if unlinked.length > 0}
      <div class="warn" role="alert" data-testid="mvr-export-unlinked">
        <p>
          {$t("lighting.mvr.export.unlinked", {
            values: { count: unlinked.length },
          })}
        </p>
        <p class="hint">
          {unlinked.map((f) => f.name).join(", ")}
        </p>
        <div class="warn__actions">
          <button
            class="btn btn-primary"
            onclick={addAim}
            disabled={working}
            data-testid="mvr-export-add-aim"
          >
            {$t("lighting.mvr.export.addAim")}
          </button>
          <button
            class="btn"
            onclick={download}
            disabled={working || !fileValid}
            data-testid="mvr-export-as-is"
          >
            {$t("lighting.mvr.export.asIs")}
          </button>
        </div>
      </div>
    {/if}
  {:else if !error}
    <p class="muted" data-testid="mvr-export-loading">
      {$t("common.loading")}
    </p>
  {/if}

  {#if addedNote}
    <p class="note" role="status" data-testid="mvr-export-added">
      {addedNote}
    </p>
  {/if}

  <label class="check">
    <input type="checkbox" bind:checked={keep} data-testid="mvr-export-keep" />
    {$t("lighting.mvr.export.keep")}
  </label>

  {#if error}
    <p class="error" role="alert" data-testid="mvr-export-error">{error}</p>
  {/if}
  {#if keptNote}
    <p class="note" role="status" data-testid="mvr-export-kept">{keptNote}</p>
  {/if}

  <div class="actions">
    <button
      class="btn"
      onclick={() => dialogEl?.close()}
      data-testid="mvr-export-close"
    >
      {$t("lighting.mvr.export.close")}
    </button>
    {#if error && !summary}
      <button class="btn" onclick={loadSummary}>{$t("common.retry")}</button>
    {/if}
    {#if summary && unlinked.length === 0}
      <button
        class="btn btn-primary"
        onclick={download}
        disabled={working || !fileValid}
        data-testid="mvr-export-download"
      >
        {$t("lighting.mvr.export.download")}
      </button>
    {/if}
  </div>
</dialog>

<style>
  .export {
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    overflow: auto;
    padding: 20px;
    background: var(--card-bg);
    color: var(--text);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    box-sizing: border-box;
  }
  .export::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  .export[open] {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .export__title {
    margin: 0;
    font-family: var(--nc-font-display);
    font-size: 16px;
    font-weight: 700;
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
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
  }
  .hint,
  .muted {
    margin: 0;
    color: var(--text-dim);
    font-size: 13px;
  }
  .hint--error,
  .error {
    color: var(--red);
  }
  .error,
  .note {
    margin: 0;
    font-size: 14px;
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
    font-size: 15px;
    font-weight: 700;
  }
  .list {
    margin: 0;
    padding-left: 20px;
    font-size: 13px;
    color: var(--yellow);
  }
  .warn {
    padding: 12px;
    border: 1px solid var(--yellow);
    border-radius: var(--nc-radius-md);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .warn p {
    margin: 0;
  }
  .warn__actions,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .actions {
    justify-content: flex-end;
  }
  @media (max-width: 600px) {
    .warn__actions .btn,
    .actions .btn {
      flex: 1 1 auto;
    }
  }
</style>
