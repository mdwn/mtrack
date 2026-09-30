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
  const allTabs = [
    "tracks",
    "midi",
    "samples",
    "sections",
    "lighting",
    "notifications",
    "config",
  ];

  // The address's path, without a `?t=` (a time to open the lighting
  // editor at, in seconds: the 3D preview's "Open this cue in the timeline").
  let hashPath = $derived(currentHash.split("?", 1)[0]);
  let initialTimeMs = $derived.by(() => {
    const raw = new URLSearchParams(currentHash.split("?", 2)[1] ?? "").get(
      "t",
    );
    const seconds = raw === null ? NaN : Number(raw);
    return Number.isFinite(seconds) && seconds >= 0 ? seconds * 1000 : null;
  });

  let songName = $derived.by(() => {
    const prefix = "#/songs/";
    if (hashPath.startsWith(prefix) && hashPath.length > prefix.length) {
      const rest = decodeURIComponent(hashPath.slice(prefix.length));
      for (const tab of allTabs) {
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
    if (!hashPath.startsWith(prefix)) return undefined;
    const segments = hashPath.slice(prefix.length).split("/");
    const last = segments[segments.length - 1];
    if (allTabs.includes(last) && segments.length > 1) {
      return last as
        | "tracks"
        | "midi"
        | "samples"
        | "sections"
        | "lighting"
        | "notifications"
        | "config";
    }
    return undefined;
  });

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
