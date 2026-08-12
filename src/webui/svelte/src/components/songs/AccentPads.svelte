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

  interface Props {
    /** Per-beat accent levels: 0 silent, 1 normal, 2 half accent, 3 accent. */
    levels: number[];
    onchange: (levels: number[]) => void;
  }

  let { levels, onchange }: Props = $props();

  /** One-way cycle: silent → normal → half accent → accent → silent. */
  function cycle(index: number) {
    const next = [...levels];
    next[index] = (next[index] + 1) % 4;
    onchange(next);
  }
</script>

<div class="accent-pads">
  {#each levels as level, i (i)}
    <button
      type="button"
      class="accent-pad"
      class:silent={level === 0}
      title={$t(`metronome.level.${level}`)}
      aria-label="{$t('metronome.beat')} {i + 1}: {$t(
        `metronome.level.${level}`,
      )}"
      onclick={() => cycle(i)}
    >
      {#each [3, 2, 1] as segment (segment)}
        <span class="pad-seg" class:filled={level >= segment}></span>
      {/each}
      <span class="pad-num">{i + 1}</span>
    </button>
  {/each}
</div>

<style>
  .accent-pads {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .accent-pad {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 46px;
    height: 72px;
    padding: 5px 5px 2px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 9px;
    cursor: pointer;
    touch-action: manipulation;
    user-select: none;
    -webkit-user-select: none;
    transition: transform 0.06s;
  }
  .accent-pad:active {
    transform: scale(0.94);
  }
  .accent-pad.silent .pad-seg {
    opacity: 0.4;
  }
  .accent-pad.silent .pad-num {
    opacity: 0.4;
  }
  .pad-seg {
    flex: 1;
    border-radius: 3px;
    border: 1px solid
      color-mix(in srgb, var(--accent, #5ecaea) 45%, transparent);
    background: transparent;
    transition: background 0.08s;
  }
  .pad-seg.filled {
    background: var(--accent, #5ecaea);
    border-color: var(--accent, #5ecaea);
  }
  .pad-num {
    font-size: 10px;
    color: var(--text-dim);
    text-align: center;
    line-height: 1.4;
    font-variant-numeric: tabular-nums;
  }
</style>
