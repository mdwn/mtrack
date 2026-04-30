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
  import { playbackStore, waveformStore } from "../../lib/ws/stores";
  import type { TrackInfo } from "../../lib/ws/stores";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

  const WAVEFORM_HEIGHT = 28;

  let canvasRefs: Record<string, HTMLCanvasElement> = $state({});

  function formatChannels(track: TrackInfo): string {
    if (track.output_channels.length === 0) return get(t)("tracks.unmapped");
    return "ch " + track.output_channels.join(", ");
  }

  function isUnmapped(track: TrackInfo): boolean {
    return track.output_channels.length === 0;
  }

  $effect(() => {
    const currentTracks = $playbackStore.tracks;
    const currentWaveform = $waveformStore;
    const currentSongName = $playbackStore.song_name;
    const currentElapsed = $playbackStore.elapsed_ms;
    const currentDuration = $playbackStore.song_duration_ms;
    const refs = canvasRefs;
    const isDark = document.documentElement.classList.contains("nc--dark");

    const id = requestAnimationFrame(() => {
      for (const track of currentTracks) {
        const canvas = refs[track.name];
        if (!canvas) continue;

        const ctx = canvas.getContext("2d");
        if (!ctx) continue;

        const w = canvas.clientWidth;
        if (w === 0) continue;
        const h = WAVEFORM_HEIGHT;

        if (canvas.width !== w || canvas.height !== h) {
          canvas.width = w;
          canvas.height = h;
        }

        ctx.clearRect(0, 0, w, h);

        const waveformTrack =
          currentWaveform.song_name === currentSongName
            ? currentWaveform.tracks.find((t) => t.name === track.name)
            : undefined;

        if (waveformTrack && waveformTrack.peaks.length > 0) {
          const peaks = waveformTrack.peaks;
          const barWidth = w / peaks.length;
          ctx.fillStyle = "rgba(94, 202, 234, 0.85)";
          for (let i = 0; i < peaks.length; i++) {
            const barHeight = peaks[i] * h;
            const x = i * barWidth;
            const y = (h - barHeight) / 2;
            ctx.fillRect(x, y, Math.max(barWidth - 0.5, 1), barHeight);
          }
        } else {
          ctx.strokeStyle = "rgba(94, 202, 234, 0.25)";
          ctx.beginPath();
          ctx.moveTo(0, h / 2);
          ctx.lineTo(w, h / 2);
          ctx.stroke();
        }

        if (currentDuration > 0) {
          const progress = currentElapsed / currentDuration;
          const x = Math.round(progress * w);
          ctx.strokeStyle = isDark
            ? "rgba(244, 241, 240, 0.7)"
            : "rgba(35, 31, 32, 0.7)";
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x, h);
          ctx.stroke();
        }
      }
    });

    return () => cancelAnimationFrame(id);
  });
</script>

<section class="card tracks-card">
  <header class="tracks-card__head">
    <div>
      <div class="overline">{$t("tracks.title")}</div>
      <div class="tracks-card__title">
        {$t("tracks.count", {
          values: { count: $playbackStore.tracks.length },
        })}
      </div>
    </div>
  </header>

  {#if $playbackStore.tracks.length === 0}
    <div class="tracks-card__empty">{$t("tracks.noTracks")}</div>
  {:else}
    <div class="tracks-card__list">
      {#each $playbackStore.tracks as track, i (`${i}:${track.name}`)}
        <div class="tracks-card__row">
          <div class="tracks-card__info">
            <div class="tracks-card__name">{track.name}</div>
            <div
              class="mono tracks-card__channels"
              class:tracks-card__channels--unmapped={isUnmapped(track)}
            >
              {formatChannels(track)}
            </div>
          </div>
          <!-- svelte-ignore a11y_no_interactive_element_to_noninteractive_role -->
          <canvas
            class="tracks-card__waveform"
            bind:this={canvasRefs[track.name]}
            height={WAVEFORM_HEIGHT}
            role="img"
            aria-label="Waveform for {track.name}"
          ></canvas>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .tracks-card {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .tracks-card__head {
    padding: 16px 20px;
    border-bottom: 1px solid var(--card-border);
  }
  .tracks-card__title {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 16px;
    margin-top: 4px;
    color: var(--nc-fg-1);
  }
  .tracks-card__empty {
    padding: 24px;
    font-size: 13px;
    color: var(--nc-fg-3);
    text-align: center;
  }
  .tracks-card__list {
    flex: 1;
    overflow-y: auto;
    max-height: 360px;
  }
  .tracks-card__row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 20px;
    border-bottom: 1px solid var(--card-border);
  }
  .tracks-card__row:last-child {
    border-bottom: none;
  }
  .tracks-card__info {
    flex: 0 0 132px;
    min-width: 0;
  }
  .tracks-card__name {
    font-family: var(--nc-font-sans);
    font-weight: 500;
    font-size: 13px;
    line-height: 1.1;
    color: var(--nc-fg-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tracks-card__channels {
    color: var(--nc-fg-3);
    font-size: 11px;
    margin-top: 4px;
  }
  .tracks-card__channels--unmapped {
    color: var(--nc-pink-600);
  }
  :global(.nc--dark) .tracks-card__channels--unmapped {
    color: var(--nc-pink-300);
  }
  .tracks-card__waveform {
    flex: 1;
    height: 28px;
    min-width: 0;
  }
</style>
