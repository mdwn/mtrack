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

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();

  let songName = $derived.by(() => {
    const prefix = "#/songs/";
    if (currentHash.startsWith(prefix) && currentHash.length > prefix.length) {
      return decodeURIComponent(currentHash.slice(prefix.length));
    }
    return null;
  });
</script>

{#if songName}
  <SongDetail {songName} />
{:else}
  <SongList />
{/if}
