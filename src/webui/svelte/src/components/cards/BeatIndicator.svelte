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
  import { rules } from "../../lib/rules";

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

  /** The beat the playhead is on, from the player's grid: one `grid_query`
   * per animation frame. Before the first beat nothing has struck yet --
   * told by the first beat's own time, not by the answer's `secs`: that is
   * the position turned back into a time, which rounds to a hair after the
   * time asked about often enough in the first bar to blink the dot off. */
  let beatState = $derived.by(() => {
    const grid = $playbackStore.beat_grid;
    if (!grid) return null;
    const elapsed = smoothElapsedMs / 1000;
    const at = rules.grid_query(grid, { secs: elapsed });
    if (!at) return null;
    if (elapsed < grid.beats[0]) {
      return {
        beatIdx: -1,
        beatInMeasure: -1,
        beatsInMeasure: at.beats_in_measure,
        measureStart: 0,
        level: 1,
      };
    }
    const beatInMeasure = Math.floor(at.beat) - 1;
    const measureStart = at.beat_index - beatInMeasure;
    return {
      beatIdx: at.beat_index,
      beatInMeasure,
      beatsInMeasure: at.beats_in_measure,
      measureStart,
      level: levelOf(at.beat_index, measureStart),
    };
  });

  /** Accent level of a beat: resolved levels from the backend when the song
   * has a metronome, plain downbeat accent otherwise. */
  function levelOf(beatIdx: number, measureStart: number): number {
    const levels = $playbackStore.beat_grid?.accent_levels;
    if (levels && beatIdx < levels.length) return levels[beatIdx];
    return beatIdx === measureStart ? 3 : 1;
  }
</script>

{#if beatState}
  <div class="beat-indicator" aria-hidden="true">
    {#key beatState.beatIdx}
      <span
        class="beat-flash"
        class:beat-flash--accent={beatState.level === 3}
        class:beat-flash--half={beatState.level === 2}
        class:beat-flash--off={!$playbackStore.is_playing ||
          beatState.beatIdx < 0 ||
          beatState.level === 0}
      ></span>
    {/key}
    <span class="beat-dots">
      {#each { length: Math.min(beatState.beatsInMeasure, 16) }, i (i)}
        {@const dotLevel = levelOf(
          beatState.measureStart + i,
          beatState.measureStart,
        )}
        <span
          class="beat-dot"
          class:beat-dot--accent={dotLevel === 3}
          class:beat-dot--half={dotLevel === 2}
          class:beat-dot--silent={dotLevel === 0}
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
  .beat-flash--half {
    background: var(--nc-amber-400);
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
  .beat-dot--half {
    width: 7px;
    height: 7px;
  }
  .beat-dot--silent {
    background: transparent;
    border: 1px solid var(--nc-bg-3);
  }
  .beat-dot--active {
    background: var(--nc-cyan-400);
  }
  .beat-dot--accent.beat-dot--active {
    background: var(--nc-pink-400);
  }
  .beat-dot--half.beat-dot--active {
    background: var(--nc-amber-400);
  }
  .beat-dot--silent.beat-dot--active {
    background: transparent;
    border-color: var(--nc-cyan-400);
  }
</style>
