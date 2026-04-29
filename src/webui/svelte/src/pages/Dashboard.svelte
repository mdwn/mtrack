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
  import { t } from "svelte-i18n";

  let hasPlaylist = $derived($playbackStore.playlist_songs.length > 0);
  let hasTracks = $derived($playbackStore.tracks.length > 0);
  let hasEffects = $derived($effectsStore.length > 0);
  let hasFixtures = $derived(Object.keys($metadataStore).length > 0);
</script>

<PlaybackCard />

{#if !hasPlaylist}
  <div class="empty-state card">
    <svg
      class="empty-icon"
      width="48"
      height="48"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      ><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle
        cx="18"
        cy="16"
        r="3"
      /></svg
    >
    <p class="empty-text">{$t("playlists.noPlaylists")}</p>
    <div class="empty-actions">
      <a href="#/playlists" class="btn btn-primary">{$t("nav.playlists")}</a>
      <a href="#/songs" class="btn">{$t("nav.songs")}</a>
    </div>
  </div>
{/if}

<div class="dashboard-row">
  <PlaylistCard />
  {#if hasTracks}
    <TracksCard />
  {/if}
</div>

<div class="dashboard-row">
  {#if hasFixtures}
    <StageView />
  {/if}
  <LogsCard />
</div>

{#if hasEffects}
  <EffectsCard />
{/if}

<style>
  .dashboard-row {
    display: grid;
    grid-template-columns: 1.2fr 1fr;
    gap: 24px;
    margin-top: 24px;
  }
  .dashboard-row > :global(*) {
    min-width: 0;
  }
  /* If a row only has one child, let it span the full width. */
  .dashboard-row:has(> :global(*:only-child)) {
    grid-template-columns: 1fr;
  }
  .empty-state {
    margin-top: 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    padding: 48px 20px;
    text-align: center;
  }
  .empty-icon {
    color: var(--nc-fg-4);
    opacity: 0.6;
  }
  .empty-text {
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 18px;
    color: var(--nc-fg-2);
  }
  .empty-actions {
    display: flex;
    gap: 8px;
  }
  @media (max-width: 900px) {
    .dashboard-row {
      grid-template-columns: 1fr;
      gap: 16px;
    }
  }
</style>
