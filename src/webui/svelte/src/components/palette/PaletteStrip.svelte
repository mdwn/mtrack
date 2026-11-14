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
  import type { StripEntry } from "../../lib/palette/resolve";

  interface Props {
    /** Every entry of the playlist with the colours it plays in. */
    entries: StripEntry[];
    /** Opens the entry's settings. */
    onselect?: (index: number) => void;
  }

  let { entries, onselect }: Props = $props();

  function describe(entry: StripEntry): string {
    if (entry.palette.slots.length === 0) {
      return $t("palette.strip.noPalette", {
        values: { position: String(entry.position), song: entry.song },
      });
    }
    const colors = entry.palette.slots
      .map((s) => `${s.name} ${s.color}`)
      .join(", ");
    return $t("palette.strip.entry", {
      values: {
        position: String(entry.position),
        song: entry.song,
        colors,
      },
    });
  }
</script>

<section class="strip" data-testid="palette-strip">
  <h4>{$t("palette.strip.title")}</h4>
  <ol class="tiles" aria-label={$t("palette.strip.title")}>
    {#each entries as entry, i (i)}
      <li>
        <button
          type="button"
          class="tile"
          class:tile--empty={entry.palette.slots.length === 0}
          data-testid={`strip-tile-${entry.position}`}
          title={describe(entry)}
          aria-label={describe(entry)}
          onclick={() => onselect?.(i)}
        >
          <span class="bands" aria-hidden="true">
            {#each entry.palette.slots as slot (slot.name)}
              <span
                class="band"
                class:band--changed={slot.source !== "show"}
                data-testid="strip-band"
                data-slot={slot.name}
                data-color={slot.color}
                data-source={slot.source}
                style:background={slot.color}
              ></span>
            {/each}
          </span>
          <span class="num" aria-hidden="true">{entry.position}</span>
          {#if entry.palette.unknown.length > 0}
            <span
              class="tile-warn"
              data-testid="strip-warn"
              aria-hidden="true"
              title={$t("palette.unknownSlotHint")}>!</span
            >
          {/if}
        </button>
      </li>
    {/each}
  </ol>
  <p class="legend">{$t("palette.strip.legend")}</p>
</section>

<style>
  .strip {
    padding: 12px 16px;
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    background: var(--inset-bg);
  }
  h4 {
    margin: 0 0 8px;
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--nc-fg-3);
  }
  .tiles {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .tile {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    width: 30px;
    padding: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--nc-fg-2);
  }
  .tile:focus-visible {
    outline: 2px solid var(--nc-cyan-400);
    outline-offset: 2px;
  }
  .bands {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 40px;
    border: 1px solid var(--card-border);
    border-radius: 4px;
    overflow: hidden;
    background: var(--nc-bg-2);
  }
  .tile--empty .bands {
    border-style: dashed;
  }
  .band {
    flex: 1;
    min-height: 4px;
  }
  /* A colour an override set rather than the show: a notch on its left. */
  .band--changed {
    box-shadow: inset 4px 0 0 var(--nc-ink, #0b1020);
  }
  .num {
    font-family: var(--nc-font-mono);
    font-size: 10px;
  }
  .tile-warn {
    position: absolute;
    top: -4px;
    right: -4px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--nc-amber-400);
    color: var(--nc-ink, #0b1020);
    font-size: 10px;
    font-weight: 700;
    line-height: 14px;
    text-align: center;
  }
  .legend {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--nc-fg-2);
  }
</style>
