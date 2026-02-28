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
</script>

<div class="dashboard-grid">
  <PlaybackCard />
  <div class="card-pair">
    <PlaylistCard />
    <div class="card-pair-follower">
      <TracksCard />
    </div>
  </div>
  <StageView />
  <EffectsCard />
  <LogsCard />
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
  }
  .card-pair > :global(:first-child) {
    flex: 1;
    min-width: 0;
  }
  .card-pair-follower {
    flex: 1;
    min-width: 0;
    position: relative;
  }
  .card-pair-follower > :global(*) {
    position: absolute;
    inset: 0;
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
  }
</style>
