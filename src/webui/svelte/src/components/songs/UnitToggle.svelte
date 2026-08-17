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

  interface Props {
    unit: "beat" | "time";
    onchange: (unit: "beat" | "time") => void;
  }

  let { unit, onchange }: Props = $props();
</script>

<div class="unit-toggle">
  <button
    type="button"
    class="unit"
    class:unit--on={unit === "beat"}
    aria-pressed={unit === "beat"}
    onclick={() => onchange("beat")}>{$t("position.modeBeat")}</button
  >
  <button
    type="button"
    class="unit"
    class:unit--on={unit === "time"}
    aria-pressed={unit === "time"}
    onclick={() => onchange("time")}>{$t("position.modeTime")}</button
  >
</div>

<style>
  .unit-toggle {
    display: flex;
    align-self: flex-start;
    border: 1px solid var(--border);
    border-radius: 99px;
    overflow: hidden;
  }
  .unit {
    padding: 4px 14px;
    font-size: 12px;
    color: var(--text-dim);
    background: transparent;
    border: none;
    cursor: pointer;
  }
  .unit--on {
    color: var(--bg);
    background: var(--accent);
    font-weight: 600;
  }
</style>
