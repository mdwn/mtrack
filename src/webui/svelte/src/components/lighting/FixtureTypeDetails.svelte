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
   * What a GDTF-referential fixture type's archive holds, above its text:
   * the fixture in 3D beside the facts the archive states, then every mode
   * of the archive with the one the `.fixture` pins marked as in use.
   * Choosing another mode shows what a show could do in it, in the mode
   * picker's words, its channels, and which venue fixtures use it.
   * Read-only — the mode is changed in the definition.
   */
  import { t } from "svelte-i18n";
  import {
    fetchFixtureTypeGdtf,
    type FixtureTypeGdtf,
  } from "../../lib/api/config";
  import {
    abilities,
    extras,
    filterModes,
    isRefused,
    missingHere,
  } from "../../lib/lighting/modes";
  import {
    beamFacts,
    outputFacts,
    powerFact,
    venueUse,
  } from "../../lib/lighting/fixtureFacts";
  import FixtureViewer from "./FixtureViewer.svelte";

  interface Props {
    name: string;
    dir?: string;
    venuesDir?: string;
  }

  let { name, dir, venuesDir }: Props = $props();

  /** Past this many characters the archive's description is clamped. */
  const LONG_ABOUT = 240;

  let data = $state<FixtureTypeGdtf | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let selected = $state("");
  let filter = $state("");
  let aboutOpen = $state(false);

  // Each name (or directory) is its own fetch; a late answer for an earlier
  // one is dropped.
  let request = 0;
  $effect(() => {
    const ask = ++request;
    data = null;
    error = null;
    loading = true;
    fetchFixtureTypeGdtf(name, dir, venuesDir)
      .then((answer) => {
        if (ask !== request) return;
        data = answer;
        filter = "";
        aboutOpen = false;
        const offered = answer.inspection.modes.filter((m) => !isRefused(m));
        selected =
          offered.find((m) => m.name === answer.matched_mode)?.name ??
          offered[0]?.name ??
          "";
      })
      .catch((e: unknown) => {
        if (ask !== request) return;
        error = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        if (ask === request) loading = false;
      });
  });

  const modes = $derived(data?.inspection.modes ?? []);
  const visible = $derived(filterModes(modes, filter));
  const offered = $derived(visible.filter((m) => !isRefused(m)));
  const mode = $derived(
    modes.find((m) => m.name === selected && !isRefused(m)) ?? null,
  );
  const words = $derived(mode ? abilities(mode) : []);
  const more = $derived(mode ? extras(mode) : []);
  const missing = $derived(mode ? missingHere(mode, modes) : []);
  const inUse = $derived(!!mode && mode.name === data?.matched_mode);

  const beam = $derived(beamFacts(data?.beam ?? null));
  const output = $derived(outputFacts(data?.beam ?? null));
  const power = $derived(powerFact(data?.beam ?? null));
  // Every venue fixture of the type is in the pinned mode today.
  const uses = $derived(venueUse(data?.venues ?? []));
  const longAbout = $derived((data?.about?.length ?? 0) > LONG_ABOUT);

  // A filter that hides the selection moves it to the first mode left.
  $effect(() => {
    if (offered.length > 0 && !offered.some((m) => m.name === selected)) {
      selected = offered[0].name;
    }
  });

  function move(delta: number) {
    if (offered.length === 0) return;
    const at = offered.findIndex((m) => m.name === selected);
    const next = Math.min(offered.length - 1, Math.max(0, at + delta));
    selected = offered[next].name;
  }

  function onListKey(e: KeyboardEvent) {
    switch (e.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Home":
        selected = offered[0]?.name ?? selected;
        break;
      case "End":
        selected = offered[offered.length - 1]?.name ?? selected;
        break;
      default:
        return;
    }
    e.preventDefault();
    queueMicrotask(() =>
      document
        .getElementById(optionId(selected))
        ?.scrollIntoView({ block: "nearest" }),
    );
  }

  function onListClick(e: MouseEvent) {
    const option = (e.target as HTMLElement).closest<HTMLElement>(
      "[data-mode]",
    );
    const chosen = modes.find((m) => m.name === option?.dataset.mode);
    if (chosen && !isRefused(chosen)) selected = chosen.name;
  }

  const optionId = (modeName: string) =>
    `ft-details-mode-${modes.findIndex((m) => m.name === modeName)}`;

  const nameOf = (what: string) => $t(`lighting.gdtf.missing.${what}`);
</script>

<section
  class="ftd"
  data-testid="ft-details"
  aria-label={$t("lighting.gdtfDetails.title")}
>
  {#if loading}
    <p class="ftd__quiet">{$t("lighting.gdtfDetails.loading")}</p>
  {:else if error}
    <p class="ftd__error" data-testid="ft-details-error">
      {$t("lighting.gdtfDetails.error", { values: { error } })}
    </p>
  {:else if data}
    <div class="ftd__top">
      {#key data}
        <div class="ftd__viewer">
          <FixtureViewer
            typeName={name}
            rig={data.rig}
            thumbnail={data.thumbnail}
            modeLabel={mode?.name ?? ""}
          />
        </div>
      {/key}

      <div class="ftd__about">
        <dl class="ftd__facts" data-testid="ft-details-facts">
          <dt>{$t("lighting.gdtfDetails.facts.archive")}</dt>
          <dd class="ftd__mono" data-testid="ft-details-archive">
            {data.archive}
          </dd>
          <dt>{$t("lighting.gdtfDetails.facts.modes")}</dt>
          <dd data-testid="ft-fact-modes">{modes.length}</dd>
          {#if beam}
            <dt>{$t("lighting.gdtfDetails.facts.beam")}</dt>
            <dd data-testid="ft-fact-beam">
              {[
                beam.type,
                [
                  beam.beam
                    ? $t("lighting.gdtfDetails.beamAngle", {
                        values: { angle: beam.beam },
                      })
                    : null,
                  beam.field
                    ? $t("lighting.gdtfDetails.fieldAngle", {
                        values: { angle: beam.field },
                      })
                    : null,
                ]
                  .filter(Boolean)
                  .join(", "),
              ]
                .filter(Boolean)
                .join(" · ")}
            </dd>
          {/if}
          {#if output}
            <dt>{$t("lighting.gdtfDetails.facts.output")}</dt>
            <dd data-testid="ft-fact-output">
              {[
                output.flux
                  ? $t("lighting.gdtfDetails.lumens", {
                      values: { value: output.flux },
                    })
                  : null,
                output.cct
                  ? $t("lighting.gdtfDetails.kelvin", {
                      values: { value: output.cct },
                    })
                  : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </dd>
          {/if}
          {#if power}
            <dt>{$t("lighting.gdtfDetails.facts.power")}</dt>
            <dd data-testid="ft-fact-power">
              {$t("lighting.gdtfDetails.watts", { values: { value: power } })}
            </dd>
          {/if}
          {#if uses.length > 0}
            <dt>{$t("lighting.gdtfDetails.facts.venues")}</dt>
            <dd data-testid="ft-fact-venues">
              {uses
                .map((u) =>
                  $t("lighting.gdtfDetails.venueCount", {
                    values: { venue: u.venue, count: u.count },
                  }),
                )
                .join(", ")}
            </dd>
          {/if}
        </dl>
        {#if !data.matched_mode}
          <p class="ftd__error" data-testid="ft-details-mode-unmatched">
            {$t("lighting.gdtfDetails.modeUnmatched", {
              values: { mode: data.mode },
            })}
          </p>
        {/if}
        {#if data.about}
          <!-- The manufacturer's words: shown as text, never as markup. -->
          <p
            class="ftd__about-text"
            class:ftd__about-text--clamped={longAbout && !aboutOpen}
            data-testid="ft-details-about"
          >
            {data.about}
          </p>
          {#if longAbout}
            <button
              class="ftd__more"
              type="button"
              aria-expanded={aboutOpen}
              onclick={() => (aboutOpen = !aboutOpen)}
              >{aboutOpen
                ? $t("lighting.gdtfDetails.less")
                : $t("lighting.gdtfDetails.more")}</button
            >
          {/if}
        {/if}
      </div>
    </div>

    <div class="ftd__modes">
      <div class="ftd__pick">
        <span class="ftd__label" id="ft-details-modes-label"
          >{$t("lighting.gdtfDetails.modes")}</span
        >
        <input
          class="input"
          type="search"
          bind:value={filter}
          placeholder={$t("lighting.gdtfDetails.filterPlaceholder", {
            values: { count: modes.length },
          })}
          aria-label={$t("lighting.gdtf.filter")}
          data-testid="ft-details-filter"
        />
        <!-- A listbox: one tab stop, arrows move the choice. -->
        <ul
          class="ftd__list"
          role="listbox"
          tabindex="0"
          aria-labelledby="ft-details-modes-label"
          aria-activedescendant={selected ? optionId(selected) : undefined}
          onkeydown={onListKey}
          onclick={onListClick}
          data-testid="ft-details-modes"
        >
          {#each visible as m (m.name)}
            {@const refused = isRefused(m)}
            <li
              id={optionId(m.name)}
              role="option"
              class="ftd__mode"
              class:ftd__mode--selected={m.name === selected}
              class:ftd__mode--refused={refused}
              aria-selected={m.name === selected}
              aria-disabled={refused}
              data-mode={m.name}
            >
              <span class="ftd__mode-name">{m.name}</span>
              {#if m.name === data.matched_mode}
                <span class="ftd__pill" data-testid="ft-details-mode-in-use"
                  >{$t("lighting.gdtfDetails.inUse")}</span
                >
              {:else}
                <span></span>
              {/if}
              <span class="ftd__mode-ch"
                >{$t("lighting.gdtfDetails.footprint", {
                  values: { count: m.footprint },
                })}</span
              >
              {#if refused}
                <span class="ftd__mode-reason"
                  >{$t("lighting.gdtf.refused", {
                    values: { reason: m.refused ?? "" },
                  })}</span
                >
              {/if}
            </li>
          {:else}
            <li class="ftd__none" role="presentation">
              {$t("lighting.gdtf.noMatch")}
            </li>
          {/each}
        </ul>
      </div>

      <div
        class="ftd__detail"
        aria-live="polite"
        data-testid="ft-details-mode-detail"
      >
        {#if mode}
          <div class="ftd__detail-head">
            <h5 class="ftd__detail-name">{mode.name}</h5>
            <span class="ftd__badge"
              >{$t("lighting.gdtf.addresses", {
                values: { count: mode.footprint },
              })}</span
            >
            {#if inUse}
              <span class="ftd__pill">{$t("lighting.gdtfDetails.inUse")}</span>
            {/if}
          </div>

          <div class="ftd__label">{$t("lighting.gdtfDetails.canUse")}</div>
          {#if words.length > 0}
            <ul class="ftd__chips" data-testid="ft-details-can">
              {#each words as w (w.key)}
                <li class="ftd__chip">
                  {$t(`lighting.gdtf.can.${w.key}`, { values: w.params })}
                </li>
              {/each}
            </ul>
          {:else}
            <p class="ftd__quiet">{$t("lighting.gdtf.canNothing")}</p>
          {/if}
          {#if more.length > 0}
            <p class="ftd__quiet">
              {$t("lighting.gdtf.extras", {
                values: { names: more.join(", ") },
              })}
            </p>
          {/if}
          {#if missing.length > 0}
            <div class="ftd__label">{$t("lighting.gdtf.notHere")}</div>
            <ul class="ftd__missing">
              {#each missing as m (m.what)}
                <li>
                  {$t("lighting.gdtf.missingIn", {
                    values: { what: nameOf(m.what), mode: m.mode },
                  })}
                </li>
              {/each}
            </ul>
          {/if}

          <div class="ftd__label">{$t("lighting.gdtfDetails.channels")}</div>
          <div class="ftd__scroll">
            <table class="ftd__table" data-testid="ft-details-channels">
              <thead>
                <tr>
                  <th scope="col">{$t("lighting.gdtfDetails.address")}</th>
                  <th scope="col">{$t("lighting.gdtfDetails.channel")}</th>
                </tr>
              </thead>
              <tbody>
                {#each mode.channels ?? [] as [offset, channel] (offset)}
                  <tr>
                    <td class="ftd__addr">{offset}</td>
                    <td>{channel}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>

          <p class="ftd__usedby" data-testid="ft-details-used-by">
            {#if inUse && uses.length > 0}
              {uses
                .map((u) =>
                  $t("lighting.gdtfDetails.usedBy", {
                    values: {
                      who: u.names
                        ? u.names.join(", ")
                        : $t("lighting.gdtfDetails.fixtureCount", {
                            values: { count: u.count },
                          }),
                      venue: u.venue,
                    },
                  }),
                )
                .join(" ")}
            {:else}
              {$t("lighting.gdtfDetails.usedByNone")}
            {/if}
          </p>
        {/if}
        <p class="field-hint">{$t("lighting.gdtfDetails.readOnly")}</p>
      </div>
    </div>
  {/if}
</section>

<style>
  .ftd {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .ftd__top {
    display: grid;
    grid-template-columns: minmax(0, 5fr) minmax(0, 4fr);
    gap: 16px;
    align-items: start;
  }
  .ftd__viewer,
  .ftd__about {
    min-width: 0;
  }
  .ftd__facts {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 6px 14px;
    align-content: start;
    margin: 0;
  }
  .ftd__facts dt {
    font-size: 12px;
    color: var(--text-dim);
  }
  .ftd__facts dd {
    margin: 0;
    font-size: 14px;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .ftd__mono {
    font-family: var(--mono);
    font-size: 12px;
  }
  .ftd__about-text {
    margin: 10px 0 0;
    font-size: 13px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
    white-space: pre-line;
  }
  .ftd__about-text--clamped {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    overflow: hidden;
  }
  .ftd__more {
    align-self: flex-start;
    margin-top: 2px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
  }
  .ftd__modes {
    display: grid;
    grid-template-columns: minmax(0, 5fr) minmax(0, 6fr);
    gap: 16px;
    align-items: start;
  }
  .ftd__pick {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .ftd__label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--text-dim);
  }
  .ftd__list {
    list-style: none;
    margin: 2px 0 0;
    padding: 0;
    max-height: 330px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .ftd__list:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .ftd__mode {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: 2px 8px;
    align-items: center;
    padding: 7px 10px;
    cursor: pointer;
    border-bottom: 1px solid var(--border);
  }
  .ftd__mode:last-child {
    border-bottom: 0;
  }
  .ftd__mode:hover {
    background: var(--bg);
  }
  .ftd__mode--selected,
  .ftd__mode--selected:hover {
    background: var(--accent-subtle);
  }
  .ftd__mode--refused {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .ftd__mode-name {
    font-family: var(--mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ftd__mode-ch {
    font-size: 12px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    text-align: right;
  }
  .ftd__mode-reason {
    grid-column: 1 / -1;
    font-size: 12px;
    color: var(--text-dim);
  }
  .ftd__none {
    padding: 10px;
    font-size: 13px;
    color: var(--text-dim);
  }
  .ftd__pill {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--accent-subtle);
    color: var(--accent);
    white-space: nowrap;
  }
  .ftd__badge {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1px 6px;
    white-space: nowrap;
  }
  .ftd__detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .ftd__detail-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .ftd__detail-name {
    margin: 0 auto 0 0;
    font-family: var(--mono);
    font-weight: 500;
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  .ftd__chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .ftd__chip {
    font-size: 12px;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--green) 16%, transparent);
    color: var(--green);
  }
  .ftd__missing {
    margin: 0;
    padding-left: 18px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .ftd__scroll {
    overflow-x: auto;
  }
  .ftd__table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .ftd__table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text-dim);
    padding: 0 8px 4px 0;
  }
  .ftd__table td {
    padding: 4px 8px 4px 0;
    border-top: 1px solid var(--border);
    font-variant-numeric: tabular-nums;
  }
  .ftd__addr {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-dim);
    width: 1%;
    white-space: nowrap;
  }
  .ftd__usedby {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .ftd__quiet {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .ftd__error {
    margin: 0;
    font-size: 13px;
    color: var(--red);
  }
  .field-hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
  }
  @media (max-width: 720px) {
    .ftd__top,
    .ftd__modes {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
