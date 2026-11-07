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
  import SongList from "../components/songs/SongList.svelte";
  import SongDetail from "../components/songs/SongDetail.svelte";
  import { songRoute } from "../lib/songRoute";

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();

  // A `?t=` is a time to open the lighting editor at, in seconds: the 3D
  // preview's "Open this cue in the timeline".
  let initialTimeMs = $derived.by(() => {
    const raw = new URLSearchParams(currentHash.split("?", 2)[1] ?? "").get(
      "t",
    );
    const seconds = raw === null ? NaN : Number(raw);
    return Number.isFinite(seconds) && seconds >= 0 ? seconds * 1000 : null;
  });

  // `#/songs/<name>[/<tab>]`, the name one encoded segment (`songRoute`).
  let route = $derived(songRoute(currentHash));
  let songName = $derived(route.name);
  let initialTab = $derived(route.tab);

  let savedSearch = $state("");
</script>

{#if songName}
  <SongDetail {songName} {initialTab} {initialTimeMs} />
{:else}
  <SongList
    initialSearch={savedSearch}
    onSearchChange={(q) => (savedSearch = q)}
  />
{/if}
