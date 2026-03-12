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
  import LightingEditor from "./pages/LightingEditor.svelte";
  import NotFound from "./pages/NotFound.svelte";

  let currentHash = $state(window.location.hash || "#/");

  function onHashChange() {
    currentHash = window.location.hash || "#/";
  }

  $effect(() => {
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
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
  {:else if currentHash.startsWith("#/lighting")}
    <LightingEditor />
  {:else}
    <NotFound />
  {/if}
</main>

<style>
  .app-main {
    max-width: 1200px;
    margin: 0 auto;
    padding: 24px 20px;
  }
  @media (max-width: 600px) {
    .app-main {
      padding: 12px 10px;
    }
  }
</style>
