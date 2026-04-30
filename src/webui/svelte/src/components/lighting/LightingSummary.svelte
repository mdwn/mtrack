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
<!--
  Read-only lighting summary for phones.

  The timeline editor isn't usable below ~1000px viewport width, so on
  phones we replace it with this summary so users can still verify a
  show is configured (rather than seeing nothing or a broken UI).
-->
<script lang="ts">
  import type { LightFile } from "../../lib/lighting/types";
  import { t } from "svelte-i18n";

  interface Props {
    lightFile: LightFile;
  }

  let { lightFile }: Props = $props();

  /** Distinct effect types across all shows + sequences, in first-seen order. */
  let effectTypes = $derived.by(() => {
    const seen: string[] = [];
    const collect = (cues: { effects?: { effect: { type: string } }[] }[]) => {
      for (const cue of cues) {
        for (const e of cue.effects ?? []) {
          const t = e.effect?.type;
          if (t && !seen.includes(t)) seen.push(t);
        }
      }
    };
    for (const show of lightFile.shows) collect(show.cues);
    for (const seq of lightFile.sequences) collect(seq.cues);
    return seen;
  });
</script>

<section class="lighting-summary">
  <p class="lighting-summary__hint">
    {$t("lightingSummary.desktopOnly")}
  </p>

  {#if lightFile.tempo}
    <div class="lighting-summary__row">
      <span class="overline">{$t("timeline.tempo")}</span>
      <span class="mono"
        >{lightFile.tempo.bpm} BPM · {lightFile.tempo.time_signature}</span
      >
    </div>
  {/if}

  {#if lightFile.shows.length > 0}
    <div class="lighting-summary__group">
      <div class="overline">{$t("lightingSummary.shows")}</div>
      <ul>
        {#each lightFile.shows as show (show.name)}
          <li>
            <span class="lighting-summary__name">{show.name}</span>
            <span class="mono"
              >{$t("cue.cueCount", {
                values: { count: show.cues.length },
              })}</span
            >
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if lightFile.sequences.length > 0}
    <div class="lighting-summary__group">
      <div class="overline">{$t("lightingSummary.sequences")}</div>
      <ul>
        {#each lightFile.sequences as seq (seq.name)}
          <li>
            <span class="lighting-summary__name">{seq.name}</span>
            <span class="mono"
              >{$t("cue.cueCount", {
                values: { count: seq.cues.length },
              })}</span
            >
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if effectTypes.length > 0}
    <div class="lighting-summary__group">
      <div class="overline">{$t("lightingSummary.effects")}</div>
      <div class="lighting-summary__chips">
        {#each effectTypes as type (type)}
          <span class="badge badge--light">{type}</span>
        {/each}
      </div>
    </div>
  {/if}

  {#if lightFile.shows.length === 0 && lightFile.sequences.length === 0}
    <p class="lighting-summary__empty">{$t("lightingSummary.empty")}</p>
  {/if}
</section>

<style>
  .lighting-summary {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 20px;
    border: 1.5px dashed var(--card-border);
    border-radius: 14px;
    background: var(--inset-bg);
  }
  .lighting-summary__hint {
    font-family: var(--nc-font-sans);
    font-size: 13px;
    color: var(--nc-fg-3);
    margin: 0;
  }
  .lighting-summary__row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .lighting-summary__group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lighting-summary__group ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .lighting-summary__group li {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 0;
    border-bottom: 1px solid var(--card-border);
  }
  .lighting-summary__group li:last-child {
    border-bottom: none;
  }
  .lighting-summary__name {
    font-family: var(--nc-font-sans);
    font-weight: 500;
    font-size: 14px;
    color: var(--nc-fg-1);
  }
  .lighting-summary__chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .lighting-summary__empty {
    font-family: var(--nc-font-sans);
    font-size: 13px;
    color: var(--nc-fg-3);
    margin: 0;
    text-align: center;
    padding: 12px 0;
  }
</style>
