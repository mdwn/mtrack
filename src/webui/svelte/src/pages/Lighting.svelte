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
  import { t } from "svelte-i18n";
  import { loadRunningLighting } from "../lib/profileStore";
  import { lightingRoute } from "../lib/lightingRoute";
  import LightingOverview from "../components/lighting/LightingOverview.svelte";
  import FixtureTypesPanel from "../components/lighting/FixtureTypesPanel.svelte";
  import VenuesPanel from "../components/lighting/VenuesPanel.svelte";
  import GroupsPage from "../components/lighting/GroupsPage.svelte";
  import FitPage from "../components/lighting/FitPage.svelte";
  import MvrImportWizard from "../components/lighting/MvrImportWizard.svelte";
  import StageView from "../components/StageView.svelte";
  import Stage3D from "./Stage3D.svelte";

  interface Props {
    currentHash: string;
  }

  let { currentHash }: Props = $props();

  const tabs = [
    { key: "overview", href: "#/lighting", labelKey: "lighting.area.overview" },
    {
      key: "fixtures",
      href: "#/lighting/fixtures",
      labelKey: "lighting.fixtureTypes",
    },
    { key: "venues", href: "#/lighting/venues", labelKey: "lighting.venues" },
    {
      key: "groups",
      href: "#/lighting/groups",
      labelKey: "lighting.area.groups",
    },
    { key: "fit", href: "#/lighting/fit", labelKey: "lighting.area.fit" },
    { key: "stage", href: "#/lighting/stage", labelKey: "lighting.area.stage" },
  ] as const;

  let route = $derived(lightingRoute(currentHash));
  // The import wizard has no tab of its own: it is part of Venues.
  let activeTab = $derived(route.sub === "import" ? "venues" : route.sub);

  // The fixture type and venue directories are a profile setting, so the
  // project-file pages read them from the profile the player is running.
  let running = $state<Awaited<ReturnType<typeof loadRunningLighting>>>(null);
  $effect(() => {
    loadRunningLighting().then((r) => (running = r));
  });
  let ftDir = $derived<string>(
    running?.lighting?.directories?.fixture_types ?? "",
  );
  let venueDir = $derived<string>(running?.lighting?.directories?.venues ?? "");
  // The venue picked on the Venues page: the plot shows it (its file, unless
  // it is the current venue). Null leaves the plot on the current venue.
  let plotVenue = $state<string | null>(null);
</script>

<div class="lighting page">
  <div class="page__head">
    <div>
      <h1 class="page__title">{$t("nav.lighting")}</h1>
    </div>
  </div>

  <nav class="lighting__tabs" aria-label={$t("lighting.area.tabsLabel")}>
    {#each tabs as tab (tab.key)}
      <a
        class="lighting__tab"
        class:lighting__tab--active={activeTab === tab.key}
        href={tab.href}
        aria-current={activeTab === tab.key ? "page" : undefined}
        >{$t(tab.labelKey)}</a
      >
    {/each}
  </nav>

  {#if route.sub === "overview"}
    <LightingOverview profileName={running?.profileName ?? null} />
  {:else if route.sub === "fixtures"}
    <FixtureTypesPanel dir={ftDir} venuesDir={venueDir} />
  {:else if route.sub === "venues"}
    <VenuesPanel
      fixtureTypesDir={ftDir}
      venuesDir={venueDir}
      bind:selected={plotVenue}
    />
    <div class="lighting__stage">
      <StageView
        editable
        fixtureTypesDir={ftDir}
        venuesDir={venueDir}
        fileVenue={plotVenue}
      />
    </div>
  {:else if route.sub === "groups"}
    <GroupsPage profileParam={route.profile} />
  {:else if route.sub === "fit"}
    <FitPage
      groupParam={route.group}
      profileName={running?.profileName ?? null}
    />
  {:else if route.sub === "import"}
    <MvrImportWizard fixtureTypesDir={ftDir} venuesDir={venueDir} />
  {:else if route.sub === "stage"}
    <Stage3D heading="h2" />
  {/if}
</div>

<style>
  .lighting__tabs {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border);
    margin-bottom: 20px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .lighting__tabs::-webkit-scrollbar {
    display: none;
  }
  .lighting__tab {
    padding: 10px 16px;
    font-size: 14px;
    font-weight: 500;
    color: var(--text-muted);
    text-decoration: none;
    white-space: nowrap;
    border-bottom: 2px solid transparent;
    transition:
      color 0.15s,
      border-color 0.15s;
  }
  .lighting__tab:hover {
    color: var(--text);
  }
  .lighting__tab--active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .lighting__stage {
    margin-top: 24px;
  }
  @media (max-width: 600px) {
    .lighting__tab {
      padding: 8px 12px;
      font-size: 13px;
    }
  }
</style>
