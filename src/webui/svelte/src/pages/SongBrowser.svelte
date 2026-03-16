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

  // Parse: #/songs/SongName or #/songs/SongName/tab
  let songName = $derived.by(() => {
    const prefix = "#/songs/";
    if (currentHash.startsWith(prefix) && currentHash.length > prefix.length) {
      const rest = decodeURIComponent(currentHash.slice(prefix.length));
      // Strip trailing tab segment if present
      const tabs = ["tracks", "midi", "lighting", "config"];
      for (const tab of tabs) {
        if (rest.endsWith("/" + tab)) {
          return rest.slice(0, -(tab.length + 1));
        }
      }
      return rest;
    }
    return null;
  });

  let initialTab = $derived.by(() => {
    const prefix = "#/songs/";
    if (!currentHash.startsWith(prefix)) return undefined;
    const segments = currentHash.slice(prefix.length).split("/");
    const last = segments[segments.length - 1];
    const tabs = ["tracks", "midi", "lighting", "config"];
    if (tabs.includes(last) && segments.length > 1) {
      return last as "tracks" | "midi" | "lighting" | "config";
    }
    return undefined;
  });
</script>

{#if songName}
  <SongDetail {songName} {initialTab} />
{:else}
  <SongList />
{/if}
