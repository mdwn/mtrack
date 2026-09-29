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
  import { fetchFixtureTypes, fetchVenues } from "../../lib/api/config";

  interface Props {
    /** The running profile's name, once known. */
    profileName: string | null;
    /** The running profile's current venue, if it selects one. */
    currentVenue: string | null;
    fixtureTypesDir: string;
    venuesDir: string;
  }

  let { profileName, currentVenue, fixtureTypesDir, venuesDir }: Props =
    $props();

  let typeCount = $state<number | null>(null);
  let venueCount = $state<number | null>(null);

  $effect(() => {
    const dir = fixtureTypesDir;
    fetchFixtureTypes(dir || undefined)
      .then((r) => (typeCount = Object.keys(r.fixtureTypes).length))
      .catch(() => (typeCount = null));
  });

  $effect(() => {
    const dir = venuesDir;
    fetchVenues(dir || undefined)
      .then((r) => (venueCount = Object.keys(r.venues).length))
      .catch(() => (venueCount = null));
  });

  let groupsHref = $derived(
    profileName
      ? `#/lighting/groups?profile=${encodeURIComponent(profileName)}`
      : "#/lighting/groups",
  );
</script>

<div class="overview" data-testid="lighting-overview">
  <dl class="overview__facts">
    <div>
      <dt>{$t("lighting.overview.currentVenue")}</dt>
      <dd data-testid="overview-venue">
        {currentVenue ?? $t("lighting.overview.noVenue")}
      </dd>
    </div>
    <div>
      <dt>{$t("lighting.fixtureTypes")}</dt>
      <dd data-testid="overview-types">{typeCount ?? "-"}</dd>
    </div>
    <div>
      <dt>{$t("lighting.venues")}</dt>
      <dd data-testid="overview-venues">{venueCount ?? "-"}</dd>
    </div>
  </dl>

  <ul class="overview__links">
    <li>
      <a href="#/lighting/fixtures">{$t("lighting.fixtureTypes")}</a>
      <span>{$t("lighting.overview.fixturesHint")}</span>
    </li>
    <li>
      <a href="#/lighting/venues">{$t("lighting.venues")}</a>
      <span>{$t("lighting.overview.venuesHint")}</span>
    </li>
    <li>
      <a href={groupsHref}>{$t("lighting.area.groups")}</a>
      <span>{$t("lighting.overview.groupsHint")}</span>
    </li>
    <li>
      <a href="#/lighting/stage">{$t("lighting.area.stage")}</a>
      <span>{$t("lighting.overview.stageHint")}</span>
    </li>
  </ul>
</div>

<style>
  .overview {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .overview__facts {
    display: flex;
    flex-wrap: wrap;
    gap: 12px 32px;
    padding: 16px 20px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
    margin: 0;
  }
  .overview__facts dt {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .overview__facts dd {
    margin: 2px 0 0;
    font-size: 18px;
    font-family: var(--nc-font-display);
    font-weight: 700;
  }
  .overview__links {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .overview__links li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 20px;
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--nc-radius-md);
  }
  .overview__links a {
    font-weight: 600;
    color: var(--accent);
  }
  .overview__links span {
    font-size: 13px;
    color: var(--text-dim);
  }
</style>
