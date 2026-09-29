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
  import { t } from "svelte-i18n";
  import { fetchLightingReadiness } from "../../lib/api/config";
  import {
    evaluateReadiness,
    type Check,
    type Finding,
    type Readiness,
  } from "../../lib/lighting/readiness";
  import { metadataStore, reloadStore, venueStore } from "../../lib/ws/stores";
  import StageView from "../StageView.svelte";

  interface Props {
    /** The running profile's name, once known: the DMX settings link needs it. */
    profileName: string | null;
  }

  let { profileName }: Props = $props();

  let readiness = $state<Readiness | null>(null);
  let error = $state<string | null>(null);

  // A refresh that arrives while one is running is folded into a single
  // follow-up: a reload sends both a reload and a metadata message, and the
  // second answer is the one that matters.
  let inflight = false;
  let again = false;
  async function refresh() {
    if (inflight) {
      again = true;
      return;
    }
    inflight = true;
    try {
      readiness = await fetchLightingReadiness();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      inflight = false;
      if (again) {
        again = false;
        refresh();
      }
    }
  }

  // Re-check when the venue or the config reloads (the server broadcasts
  // metadata after either), not on a timer. The stores' first values, set on
  // connect, also land here, which is the initial load.
  $effect(() => {
    void $metadataStore;
    void $venueStore;
    void $reloadStore;
    untrack(refresh);
  });

  let checks = $derived<Check[]>(
    readiness ? evaluateReadiness(readiness, { profileName }) : [],
  );
  let withFindings = $derived(checks.filter((c) => c.findings.length > 0));

  /** A check's findings, grouped under their song where they have one. */
  function grouped(
    findings: Finding[],
  ): { song: string | null; items: Finding[] }[] {
    const out: { song: string | null; items: Finding[] }[] = [];
    for (const f of findings) {
      const song = f.song ?? null;
      let group = out.find((g) => g.song === song);
      if (!group) {
        group = { song, items: [] };
        out.push(group);
      }
      group.items.push(f);
    }
    return out;
  }
</script>

<div class="hub" data-testid="lighting-overview">
  <p class="hub__lede">{$t("lighting.hub.lede")}</p>

  {#if error}
    <p class="hub__error" role="alert" data-testid="hub-error">
      {$t("lighting.hub.error", { values: { error } })}
      <button class="btn btn--sm" onclick={refresh}>{$t("common.retry")}</button
      >
    </p>
  {/if}

  {#if !readiness && !error}
    <p class="hub__loading" data-testid="hub-loading">
      {$t("lighting.hub.loading")}
    </p>
  {/if}

  {#if readiness}
    <ol class="hub__strip" aria-label={$t("lighting.hub.checksLabel")}>
      {#each checks as check, i (check.key)}
        <li
          class="check check--{check.state}"
          data-testid="check-{check.key}"
          data-state={check.state}
        >
          <span class="check__num" aria-hidden="true">{i + 1}</span>
          <h2 class="check__title">{$t(`lighting.hub.check.${check.key}`)}</h2>
          <span class="check__state" data-testid="check-{check.key}-state">
            <span class="check__dot" aria-hidden="true"></span>
            {$t(`lighting.hub.state.${check.state}`)}
          </span>
          <p class="check__summary">
            {$t(check.summary.key, { values: check.summary.params })}
          </p>
        </li>
      {/each}
    </ol>

    <section class="hub__attention" aria-labelledby="hub-attention-title">
      <h2 id="hub-attention-title" class="hub__section-title">
        {$t("lighting.hub.attention.title")}
      </h2>
      {#if withFindings.length === 0}
        <p class="hub__none" data-testid="hub-none">
          {$t("lighting.hub.attention.none")}
        </p>
      {:else}
        {#each withFindings as check (check.key)}
          <div class="finding-group" data-testid="findings-{check.key}">
            <h3 class="finding-group__title">
              {$t(`lighting.hub.check.${check.key}`)}
            </h3>
            {#each grouped(check.findings) as group (group.song ?? "")}
              {#if group.song}
                <p class="finding-group__song">{group.song}</p>
              {/if}
              <ul class="findings">
                {#each group.items as finding, n (n)}
                  <li class="finding finding--{finding.severity}">
                    <span class="finding__text">
                      {#if finding.severity === "note"}
                        <span class="finding__tag"
                          >{$t("lighting.hub.note")}</span
                        >
                      {/if}
                      {$t(finding.msg.key, { values: finding.msg.params })}
                    </span>
                    <a class="finding__fix" href={finding.href}
                      >{$t(finding.linkKey)}</a
                    >
                  </li>
                {/each}
              </ul>
            {/each}
          </div>
        {/each}
      {/if}
    </section>
  {/if}

  <section class="hub__stage" aria-labelledby="hub-stage-title">
    <h2 id="hub-stage-title" class="hub__section-title">
      {$t("lighting.hub.live")}
    </h2>
    <StageView />
  </section>
</div>

<style>
  .hub {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .hub__lede {
    margin: 0;
    color: var(--text-muted);
  }
  .hub__error {
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
  .hub__loading {
    margin: 0;
    color: var(--text-dim);
  }
  .hub__section-title {
    margin: 0 0 8px;
    font-family: var(--nc-font-display);
    font-size: 16px;
    font-weight: 700;
  }

  /* The strip: five cards in a row, wrapping to fewer columns as the width
     drops and to one on a phone. */
  .hub__strip {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 12px;
  }
  .check {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-areas:
      "num title"
      "state state"
      "summary summary";
    gap: 4px 10px;
    align-items: center;
    padding: 14px 16px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-top: 4px solid var(--check-color, var(--border));
    border-radius: var(--nc-radius-md);
    min-width: 0;
  }
  .check--ready {
    --check-color: var(--green);
  }
  .check--attention {
    --check-color: var(--yellow);
  }
  .check--blocked {
    --check-color: var(--red);
  }
  .check--unknown {
    --check-color: var(--text-dim);
  }
  .check__num {
    grid-area: num;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--nc-radius-pill);
    background: var(--bg-surface);
    font-size: 12px;
    font-weight: 700;
  }
  .check__title {
    grid-area: title;
    margin: 0;
    font-family: var(--nc-font-display);
    font-size: 15px;
    font-weight: 700;
  }
  .check__state {
    grid-area: state;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .check__dot {
    width: 8px;
    height: 8px;
    border-radius: var(--nc-radius-pill);
    background: var(--check-color, var(--border));
  }
  .check__summary {
    grid-area: summary;
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .hub__attention,
  .hub__stage {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    padding: 16px 20px;
  }
  .hub__none {
    margin: 0;
    color: var(--text-muted);
  }
  .finding-group + .finding-group {
    margin-top: 16px;
  }
  .finding-group__title {
    margin: 0 0 6px;
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .finding-group__song {
    margin: 8px 0 4px;
    font-weight: 600;
  }
  .findings {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .finding {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    justify-content: space-between;
    align-items: baseline;
    padding: 8px 12px;
    background: var(--inset-bg);
    border-left: 3px solid var(--border);
    border-radius: var(--nc-radius-xs);
  }
  .finding--blocked {
    border-left-color: var(--red);
  }
  .finding--attention {
    border-left-color: var(--yellow);
  }
  .finding__text {
    flex: 1 1 260px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .finding__tag {
    display: inline-block;
    margin-right: 6px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--nc-radius-pill);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .finding__fix {
    font-weight: 600;
    color: var(--accent);
    white-space: nowrap;
  }

  @media (max-width: 600px) {
    .hub__strip {
      grid-template-columns: 1fr;
    }
    .hub__attention,
    .hub__stage {
      padding: 12px 14px;
    }
  }
</style>
