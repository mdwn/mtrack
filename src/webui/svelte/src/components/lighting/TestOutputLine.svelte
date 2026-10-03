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
  // "Test output is live on universe N, addresses a–b (type). Open · Stop":
  // the nav's banner on every page, and the stage card's line, so the
  // stage says why a light is doing what the show is not.
  import { t } from "svelte-i18n";
  import { statusStore, testOutput } from "../../lib/ws/status";
  import { releaseTest } from "../../lib/lighting/fixtureTest";

  interface Props {
    /** The Stop button's test id. */
    stopTestId?: string;
  }

  let { stopTestId = "test-output-stop" }: Props = $props();

  /** Stops a fixture test from anywhere; a page that is sending hears the
   *  event and stops its heartbeat. */
  async function stopTest() {
    window.dispatchEvent(new Event("mtrack:fixture-test-stop"));
    await releaseTest();
    statusStore.update((s) =>
      s ? { ...s, hardware: { ...s.hardware, test_output: null } } : s,
    );
  }
</script>

{#if $testOutput}
  <span
    >{$t("nav.testOutput", {
      values: {
        universe: $testOutput.universe,
        from: $testOutput.address,
        to: $testOutput.address + Math.max(1, $testOutput.footprint) - 1,
        fixture: $testOutput.fixture_type,
      },
    })}</span
  >
  <a
    class="test-output__open"
    href={`#/lighting/fixtures/${encodeURIComponent($testOutput.fixture_type)}`}
    >{$t("nav.testOutputOpen")}</a
  >
  <button
    class="btn btn-sm"
    type="button"
    data-testid={stopTestId}
    onclick={stopTest}>{$t("nav.testOutputStop")}</button
  >
{/if}

<style>
  .test-output__open {
    color: var(--accent);
    text-decoration: underline;
  }
</style>
