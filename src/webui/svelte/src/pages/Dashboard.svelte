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
  import PlaybackCard from "../components/cards/PlaybackCard.svelte";
  import PlaylistCard from "../components/cards/PlaylistCard.svelte";
  import TracksCard from "../components/cards/TracksCard.svelte";
  import EffectsCard from "../components/cards/EffectsCard.svelte";
  import LogsCard from "../components/cards/LogsCard.svelte";
  import StageView from "../components/StageView.svelte";
  import { playbackStore } from "../lib/ws/stores";
  import { effectsStore } from "../lib/ws/stores";
  import { metadataStore } from "../lib/ws/stores";

  let hasPlaylist = $derived($playbackStore.playlist_songs.length > 0);
  let hasTracks = $derived($playbackStore.tracks.length > 0);
  let hasEffects = $derived($effectsStore.length > 0);
  let hasFixtures = $derived(Object.keys($metadataStore).length > 0);
</script>

<div class="dashboard-grid">
  <PlaybackCard />
  {#if !hasPlaylist}
    <div class="empty-state card">
      <p class="empty-text">No playlist loaded</p>
      <div class="empty-actions">
        <a href="#/playlists" class="btn btn-primary">Go to Playlists</a>
        <a href="#/songs" class="btn">Browse Songs</a>
      </div>
    </div>
  {/if}
  <div class="card-pair">
    <PlaylistCard />
    {#if hasTracks}
      <div class="card-pair-follower">
        <TracksCard />
      </div>
    {/if}
  </div>
  {#if hasFixtures}
    <StageView />
  {/if}
  {#if hasEffects}
    <div class="card-pair-bottom">
      <EffectsCard />
      <LogsCard />
    </div>
  {:else}
    <LogsCard />
  {/if}
</div>

<style>
  .dashboard-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  .card-pair {
    grid-column: 1 / -1;
    display: flex;
    gap: 16px;
    height: 280px;
  }
  .card-pair > :global(:first-child) {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .card-pair-follower {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .card-pair-bottom {
    grid-column: 1 / -1;
    display: flex;
    gap: 16px;
  }
  .card-pair-bottom > :global(:first-child) {
    width: 280px;
    flex-shrink: 0;
  }
  .card-pair-bottom > :global(:last-child) {
    flex: 1;
    min-width: 0;
  }
  .empty-state {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 32px 20px;
    text-align: center;
  }
  .empty-text {
    font-size: 15px;
    color: var(--text-dim);
  }
  .empty-actions {
    display: flex;
    gap: 8px;
  }
  @media (max-width: 768px) {
    .dashboard-grid {
      grid-template-columns: 1fr;
    }
    .card-pair {
      flex-direction: column;
    }
    .card-pair-follower {
      min-height: 200px;
    }
    .card-pair-bottom {
      flex-direction: column;
    }
    .card-pair-bottom > :global(:first-child) {
      width: 100%;
    }
  }
</style>
