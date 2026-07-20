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
  interface Props {
    /** Note value denominator: 1 whole, 2 half, 4 quarter, 8 eighth, ... */
    den: number;
    /** Number of notes drawn; groups of flagged values are beamed. */
    count?: number;
    /** Tuplet number rendered above the group. */
    tuplet?: number;
  }

  let { den, count = 1, tuplet }: Props = $props();

  const HEAD_Y = 24;

  let layout = $derived.by(() => {
    const filled = den >= 4;
    const hasStem = den >= 2;
    const flags = den >= 8 ? Math.round(Math.log2(den)) - 2 : 0;
    const beamed = count > 1 && flags > 0;
    const spacing = beamed ? (count > 4 ? 9.5 : 12) : 16;
    const xs = Array.from({ length: count }, (_, i) => 5 + i * spacing);
    const width = xs[xs.length - 1] + 7 + (!beamed && flags > 0 ? 6 : 0);
    const stemTop = tuplet !== undefined ? 10 : 5;
    return { filled, hasStem, flags, beamed, xs, width, stemTop };
  });

  let stemX = (x: number) => x + 3;
</script>

<svg class="note-icon" viewBox="0 0 {layout.width} 30" aria-hidden="true">
  {#if tuplet !== undefined}
    <text
      x={(layout.xs[0] + layout.xs[layout.xs.length - 1] + 6) / 2}
      y="7"
      text-anchor="middle"
      font-size="9"
      font-style="italic"
      font-weight="600"
      fill="currentColor">{tuplet}</text
    >
  {/if}

  {#each layout.xs as x (x)}
    <ellipse
      cx={x}
      cy={HEAD_Y}
      rx={den === 1 ? 4.4 : 3.6}
      ry="2.7"
      transform="rotate(-20 {x} {HEAD_Y})"
      fill={layout.filled ? "currentColor" : "none"}
      stroke="currentColor"
      stroke-width={layout.filled ? 0 : 1.7}
    />
    {#if layout.hasStem}
      <line
        x1={stemX(x)}
        y1={HEAD_Y - 1}
        x2={stemX(x)}
        y2={layout.stemTop}
        stroke="currentColor"
        stroke-width="1.4"
      />
    {/if}
    {#if !layout.beamed && layout.flags > 0}
      {#each { length: layout.flags }, flag (flag)}
        <path
          d="M {stemX(x)} {layout.stemTop + flag * 4.5} c 4.5 1.5 5.5 5 2.5 9"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        />
      {/each}
    {/if}
  {/each}

  {#if layout.beamed}
    {#each { length: layout.flags }, beam (beam)}
      <rect
        x={stemX(layout.xs[0]) - 0.7}
        y={layout.stemTop + beam * 4.5}
        width={layout.xs[layout.xs.length - 1] - layout.xs[0] + 1.4}
        height="2.7"
        fill="currentColor"
      />
    {/each}
  {/if}
</svg>

<style>
  .note-icon {
    height: 28px;
    width: auto;
    display: block;
  }
</style>
