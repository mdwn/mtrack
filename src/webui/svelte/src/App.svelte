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
  import Nav from "./components/Nav.svelte";
  import Dashboard from "./pages/Dashboard.svelte";
  import ConfigEditor from "./pages/ConfigEditor.svelte";
  import SongBrowser from "./pages/SongBrowser.svelte";
  import PlaylistEditor from "./pages/PlaylistEditor.svelte";
  import StatusPage from "./pages/StatusPage.svelte";
  import NotFound from "./pages/NotFound.svelte";
  import { playbackStore } from "./lib/ws/stores";

  let currentHash = $state(window.location.hash || "#/");

  function onHashChange() {
    currentHash = window.location.hash || "#/";
  }

  $effect(() => {
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  });

  $effect(() => {
    const titles: Record<string, string> = {
      "#/": "Dashboard",
      "#/config": "Config",
      "#/songs": "Songs",
      "#/playlists": "Playlists",
      "#/status": "Status",
    };
    const base = "mtrack";
    const route = Object.keys(titles).find((k) =>
      k === "#/"
        ? currentHash === "#/" || currentHash === ""
        : currentHash.startsWith(k),
    );
    const pageTitle = route ? titles[route] : "";

    const song = $playbackStore.song_name;
    const playing = $playbackStore.is_playing;

    if (playing && song) {
      document.title = `\u25B6 ${song} - ${base}`;
    } else {
      document.title = pageTitle ? `${pageTitle} - ${base}` : base;
    }
  });
</script>

<Nav {currentHash} />

<main class="app-main">
  {#if currentHash === "#/" || currentHash === ""}
    <Dashboard />
  {:else if currentHash.startsWith("#/config")}
    <ConfigEditor />
  {:else if currentHash.startsWith("#/songs")}
    <SongBrowser {currentHash} />
  {:else if currentHash.startsWith("#/playlists")}
    <PlaylistEditor />
  {:else if currentHash.startsWith("#/status")}
    <StatusPage />
  {:else}
    <NotFound />
  {/if}
</main>

<style>
  .app-main {
    max-width: 1600px;
    margin: 0 auto;
    padding: 24px 20px;
  }
  @media (max-width: 600px) {
    .app-main {
      padding: 12px 10px;
    }
  }
</style>
