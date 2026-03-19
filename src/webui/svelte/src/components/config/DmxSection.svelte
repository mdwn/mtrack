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
  /* eslint-disable @typescript-eslint/no-explicit-any */
  import { t } from "svelte-i18n";
  import Tooltip from "./Tooltip.svelte";

  interface Props {
    dmx: any;
    onchange: () => void;
  }

  let { dmx = $bindable(), onchange }: Props = $props();

  function setOrDelete(key: string, value: any) {
    if (value === undefined || value === "") {
      delete dmx[key];
    } else {
      dmx[key] = value;
    }
    onchange();
  }

  // Universe helpers
  let universes: { universe: number; name: string }[] = $derived(
    dmx.universes || [],
  );

  function addUniverse() {
    if (!dmx.universes) dmx.universes = [];
    const next =
      dmx.universes.length > 0
        ? Math.max(...dmx.universes.map((u: any) => u.universe)) + 1
        : 1;
    dmx.universes.push({ universe: next, name: "" });
    onchange();
  }

  function removeUniverse(i: number) {
    dmx.universes.splice(i, 1);
    if (dmx.universes.length === 0) delete dmx.universes;
    onchange();
  }

  function updateUniverse(i: number, key: string, value: any) {
    dmx.universes[i][key] = value;
    onchange();
  }
</script>

<div class="section-fields">
  <div class="field-row-2">
    <div class="field">
      <label for="dmx-ola-port"
        >{$t("dmx.olaPort")}
        <Tooltip text={$t("tooltips.dmx.olaPort")} /></label
      >
      <input
        id="dmx-ola-port"
        type="number"
        class="input"
        placeholder="9010"
        value={dmx.ola_port ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("ola_port", v ? parseInt(v) : undefined);
        }}
      />
    </div>

    <div class="field">
      <label for="dmx-dim-speed"
        >{$t("dmx.dimSpeedModifier")}
        <Tooltip text={$t("tooltips.dmx.dimSpeedModifier")} /></label
      >
      <input
        id="dmx-dim-speed"
        type="number"
        step="0.1"
        class="input"
        placeholder="1.0"
        value={dmx.dim_speed_modifier ?? ""}
        onchange={(e) => {
          const v = (e.target as HTMLInputElement).value;
          setOrDelete("dim_speed_modifier", v ? parseFloat(v) : undefined);
        }}
      />
    </div>
  </div>

  <div class="field-row-2">
    <div class="field">
      <label for="dmx-delay"
        >{$t("dmx.playbackDelay")}
        <Tooltip text={$t("tooltips.dmx.playbackDelay")} /></label
      >
      <input
        id="dmx-delay"
        type="text"
        class="input"
        placeholder={$t("dmx.playbackDelayPlaceholder")}
        value={dmx.playback_delay ?? ""}
        onchange={(e) =>
          setOrDelete(
            "playback_delay",
            (e.target as HTMLInputElement).value.trim() || undefined,
          )}
      />
    </div>

    <div class="field">
      <label for="dmx-null-client">
        <input
          id="dmx-null-client"
          type="checkbox"
          checked={dmx.null_client ?? false}
          onchange={(e) => {
            const checked = (e.target as HTMLInputElement).checked;
            setOrDelete("null_client", checked || undefined);
          }}
        />
        {$t("dmx.nullClient")}
        <Tooltip text={$t("tooltips.dmx.nullClient")} />
      </label>
    </div>
  </div>

  <div class="field">
    <div class="field-header">
      <span class="field-label"
        >{$t("dmx.universes")}
        <Tooltip text={$t("tooltips.dmx.universes")} /></span
      >
      <button class="btn" onclick={addUniverse}>{$t("common.add")}</button>
    </div>
    {#each universes as u, i (u.universe)}
      <div class="universe-row">
        <input
          type="number"
          class="input universe-num"
          placeholder={$t("dmx.universePlaceholder")}
          value={u.universe}
          onchange={(e) =>
            updateUniverse(
              i,
              "universe",
              parseInt((e.target as HTMLInputElement).value) || 1,
            )}
        />
        <input
          class="input universe-name"
          placeholder={$t("dmx.namePlaceholder")}
          value={u.name}
          onchange={(e) =>
            updateUniverse(
              i,
              "name",
              (e.target as HTMLInputElement).value.trim(),
            )}
        />
        <button class="btn btn-danger" onclick={() => removeUniverse(i)}
          >X</button
        >
      </div>
    {/each}
  </div>
</div>

<style>
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label,
  .field-label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field label:has(input[type="checkbox"]) {
    display: flex;
    align-items: center;
    gap: 8px;
    text-transform: none;
    font-size: 14px;
    cursor: pointer;
  }
  .field-row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .universe-row {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .universe-num {
    width: 100px;
    flex-shrink: 0;
  }
  .universe-name {
    flex: 1;
  }
  @media (max-width: 600px) {
    .field-row-2 {
      grid-template-columns: 1fr;
    }
  }
</style>
