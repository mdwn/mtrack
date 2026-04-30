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
  import MiniPlayer from "./components/MiniPlayer.svelte";
  import Dashboard from "./pages/Dashboard.svelte";
  import ConfigEditor from "./pages/ConfigEditor.svelte";
  import SongBrowser from "./pages/SongBrowser.svelte";
  import PlaylistEditor from "./pages/PlaylistEditor.svelte";
  import StatusPage from "./pages/StatusPage.svelte";
  import NotFound from "./pages/NotFound.svelte";
  import ConfirmDialog from "./components/ConfirmDialog.svelte";
  import { playbackStore } from "./lib/ws/stores";
  import { confirmNavigation, hasDirty } from "./lib/dirtyGuard";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";

  let currentHash = $state(window.location.hash || "#/");

  /**
   * Returns the routing "scope" for a hash — the portion that drives which
   * page-level component stays mounted. Hash changes within the same scope
   * (e.g. song-detail tab switches, config profile sections) don't
   * remount the editor, so they don't lose unsaved state and don't need
   * a discard prompt.
   */
  function pageScope(hash: string): string {
    const parts = hash.replace(/^#\/?/, "").split("/");
    const page = parts[0] || "dashboard";
    if (page === "songs" && parts[1]) return `songs/${parts[1]}`;
    if (page === "playlists" && parts[1]) return `playlists/${parts[1]}`;
    if (page === "config" && parts[1]) return `config/${parts[1]}`;
    return page;
  }

  async function onHashChange() {
    const next = window.location.hash || "#/";
    if (next === currentHash) return;

    const sameScope = pageScope(next) === pageScope(currentHash);
    if (!sameScope && hasDirty()) {
      const previous = currentHash;
      const ok = await confirmNavigation();
      if (!ok) {
        // Restore the URL to the page the user is still editing. The
        // resulting hashchange event re-enters this handler, but `next`
        // will equal `currentHash` so it exits early.
        window.location.hash = previous;
        return;
      }
    }
    currentHash = next;
  }

  $effect(() => {
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  });

  $effect(() => {
    const base = "mtrack";
    let pageTitle = "";

    if (currentHash === "#/" || currentHash === "") {
      pageTitle = get(t)("nav.dashboard");
    } else if (currentHash.startsWith("#/config")) {
      const rest = decodeURIComponent(currentHash.slice("#/config/".length));
      pageTitle = rest
        ? `${get(t)("nav.config")} - ${rest.split("/")[0]}`
        : get(t)("nav.config");
    } else if (currentHash.startsWith("#/songs")) {
      const rest = decodeURIComponent(currentHash.slice("#/songs/".length));
      pageTitle = rest
        ? `${get(t)("nav.songs")} - ${rest.split("/")[0]}`
        : get(t)("nav.songs");
    } else if (currentHash.startsWith("#/playlists")) {
      const rest = decodeURIComponent(currentHash.slice("#/playlists/".length));
      pageTitle = rest
        ? `${get(t)("nav.playlists")} - ${rest}`
        : get(t)("nav.playlists");
    } else if (currentHash.startsWith("#/status")) {
      pageTitle = get(t)("nav.status");
    }

    const song = $playbackStore.song_name;
    const playing = $playbackStore.is_playing;

    if (playing && song) {
      document.title = `▶ ${song} - ${base}`;
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
    <ConfigEditor {currentHash} />
  {:else if currentHash.startsWith("#/songs")}
    <SongBrowser {currentHash} />
  {:else if currentHash.startsWith("#/playlists")}
    <PlaylistEditor {currentHash} />
  {:else if currentHash.startsWith("#/status")}
    <StatusPage />
  {:else}
    <NotFound />
  {/if}
</main>

<MiniPlayer />
<ConfirmDialog />

<style>
  .app-main {
    max-width: 1280px;
    margin: 0 auto;
    padding: 32px 24px 80px;
  }
  @media (max-width: 720px) {
    .app-main {
      padding: 18px 14px 110px;
    }
  }
</style>
