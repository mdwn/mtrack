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

  const WAVEFORM_HEIGHT = 32;

  // Map of track name -> canvas element, managed via bind:this
  let canvasRefs: Record<string, HTMLCanvasElement> = $state({});

  function formatChannels(track: TrackInfo): string {
    if (track.output_channels.length === 0) return "(unmapped)";
    return "ch " + track.output_channels.join(", ");
  }

  // Draw waveforms whenever reactive deps change
  $effect(() => {
    // Read all reactive deps here
    const currentTracks = $playbackStore.tracks;
    const currentWaveform = $waveformStore;
    const currentSongName = $playbackStore.song_name;
    const currentElapsed = $playbackStore.elapsed_ms;
    const currentDuration = $playbackStore.song_duration_ms;
    const refs = canvasRefs;

    // Use rAF for a single paint
    const id = requestAnimationFrame(() => {
      for (const track of currentTracks) {
        const canvas = refs[track.name];
        if (!canvas) continue;

        const ctx = canvas.getContext("2d");
        if (!ctx) continue;

        const w = canvas.clientWidth;
        if (w === 0) continue;
        const h = WAVEFORM_HEIGHT;

        // Set canvas resolution to match display size
        if (canvas.width !== w || canvas.height !== h) {
          canvas.width = w;
          canvas.height = h;
        }

        ctx.clearRect(0, 0, w, h);

        // Find peaks for this track
        const waveformTrack =
          currentWaveform.song_name === currentSongName
            ? currentWaveform.tracks.find((t) => t.name === track.name)
            : undefined;

        if (waveformTrack && waveformTrack.peaks.length > 0) {
          // Draw waveform bars
          const peaks = waveformTrack.peaks;
          const barWidth = w / peaks.length;

          ctx.fillStyle = "rgba(91, 91, 214, 0.4)";
          for (let i = 0; i < peaks.length; i++) {
            const barHeight = peaks[i] * h;
            const x = i * barWidth;
            const y = (h - barHeight) / 2;
            ctx.fillRect(x, y, Math.max(barWidth - 0.5, 1), barHeight);
          }
        } else {
          // No waveform data yet — draw a flat line
          ctx.strokeStyle = "rgba(91, 91, 214, 0.2)";
          ctx.beginPath();
          ctx.moveTo(0, h / 2);
          ctx.lineTo(w, h / 2);
          ctx.stroke();
        }

        // Draw playhead
        if (currentDuration > 0) {
          const progress = currentElapsed / currentDuration;
          const x = Math.round(progress * w);
          ctx.strokeStyle = "rgba(228, 228, 231, 0.7)";
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

<div class="card tracks-card">
  <div class="card-header">
    <span class="card-title">Tracks</span>
    <span class="track-count">{$playbackStore.tracks.length} tracks</span>
  </div>
  {#if $playbackStore.tracks.length === 0}
    <div class="empty">No tracks</div>
  {:else}
    <div class="tracks-list">
      {#each $playbackStore.tracks as track (track.name)}
        <div class="track-row">
          <div class="track-info">
            <span class="track-name">{track.name}</span>
            <span class="track-channels">{formatChannels(track)}</span>
          </div>
          <canvas
            class="track-waveform"
            bind:this={canvasRefs[track.name]}
            height={WAVEFORM_HEIGHT}
          ></canvas>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .tracks-card {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .track-count {
    font-size: 11px;
    color: var(--text-dim);
  }
  .empty {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 0;
  }
  .tracks-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .track-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.02);
  }
  .track-info {
    flex: 0 0 140px;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .track-name {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .track-channels {
    font-size: 10px;
    color: var(--text-dim);
  }
  .track-waveform {
    flex: 1;
    height: 32px;
    min-width: 0;
  }
</style>
