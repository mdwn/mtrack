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
  import { playbackStore } from "../../lib/ws/stores";

  // Never extrapolate more than this past the last frame — if frames stop
  // arriving (disconnect), the indicator freezes instead of running away.
  const MAX_EXTRAPOLATION_MS = 600;

  // Smoothly-extrapolated elapsed time, resynced on every 5Hz frame.
  let smoothElapsedMs = $state(0);
  let raf = 0;

  $effect(() => {
    if (!$playbackStore.is_playing) {
      smoothElapsedMs = $playbackStore.elapsed_ms;
      return;
    }
    const tick = () => {
      const state = $playbackStore;
      const since = Math.min(
        performance.now() - state.received_at,
        MAX_EXTRAPOLATION_MS,
      );
      smoothElapsedMs = Math.min(
        state.elapsed_ms + since,
        state.song_duration_ms || Infinity,
      );
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  let beatState = $derived.by(() => {
    const grid = $playbackStore.beat_grid;
    if (!grid || grid.beats.length === 0 || grid.measure_starts.length === 0) {
      return null;
    }
    const elapsed = smoothElapsedMs / 1000;

    // The last beat at or before the playhead (binary search).
    let lo = 0;
    let hi = grid.beats.length - 1;
    if (elapsed < grid.beats[0]) {
      return {
        beatIdx: -1,
        beatInMeasure: -1,
        beatsInMeasure: countBeats(grid, 0),
        accent: false,
      };
    }
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (grid.beats[mid] <= elapsed) lo = mid;
      else hi = mid - 1;
    }
    const beatIdx = lo;

    // The measure containing this beat.
    let measure = 0;
    for (let i = grid.measure_starts.length - 1; i >= 0; i--) {
      if (grid.measure_starts[i] <= beatIdx) {
        measure = i;
        break;
      }
    }
    return {
      beatIdx,
      beatInMeasure: beatIdx - grid.measure_starts[measure],
      beatsInMeasure: countBeats(grid, measure),
      accent: beatIdx === grid.measure_starts[measure],
    };
  });

  function countBeats(
    grid: { beats: number[]; measure_starts: number[] },
    measure: number,
  ): number {
    const start = grid.measure_starts[measure];
    const end =
      measure + 1 < grid.measure_starts.length
        ? grid.measure_starts[measure + 1]
        : grid.beats.length;
    return Math.max(end - start, 1);
  }
</script>

{#if beatState}
  <div class="beat-indicator" aria-hidden="true">
    {#key beatState.beatIdx}
      <span
        class="beat-flash"
        class:beat-flash--accent={beatState.accent}
        class:beat-flash--off={!$playbackStore.is_playing ||
          beatState.beatIdx < 0}
      ></span>
    {/key}
    <span class="beat-dots">
      {#each { length: Math.min(beatState.beatsInMeasure, 16) }, i (i)}
        <span
          class="beat-dot"
          class:beat-dot--accent={i === 0}
          class:beat-dot--active={i === beatState.beatInMeasure}
        ></span>
      {/each}
    </span>
  </div>
{/if}

<style>
  .beat-indicator {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .beat-flash {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--nc-cyan-400);
    opacity: 0.15;
    animation: beat-pulse 180ms ease-out;
  }
  .beat-flash--accent {
    background: var(--nc-pink-400);
  }
  .beat-flash--off {
    animation: none;
    opacity: 0.15;
  }
  @keyframes beat-pulse {
    0% {
      opacity: 1;
      transform: scale(1.25);
    }
    100% {
      opacity: 0.15;
      transform: scale(1);
    }
  }
  .beat-dots {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .beat-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--nc-bg-3);
    transition: background 60ms linear;
  }
  .beat-dot--accent {
    width: 8px;
    height: 8px;
  }
  .beat-dot--active {
    background: var(--nc-cyan-400);
  }
  .beat-dot--accent.beat-dot--active {
    background: var(--nc-pink-400);
  }
</style>
