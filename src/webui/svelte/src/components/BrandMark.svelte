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
<!--
  Brand mark — small pixel-EQ logo evoking a stylized 3x3 equalizer.
  Cells 1, 5, 9 are pink; 2, 6, 7 are cyan; the rest follow currentColor.
-->
<script lang="ts">
  interface Props {
    size?: "sm" | "md";
  }
  let { size = "md" }: Props = $props();
</script>

<span
  class="brand-mark"
  class:brand-mark--sm={size === "sm"}
  aria-hidden="true"
>
  <span class="brand-mark__cell brand-mark__cell--pink"></span>
  <span class="brand-mark__cell brand-mark__cell--cyan"></span>
  <span class="brand-mark__cell"></span>
  <span class="brand-mark__cell"></span>
  <span class="brand-mark__cell brand-mark__cell--pink"></span>
  <span class="brand-mark__cell brand-mark__cell--cyan"></span>
  <span class="brand-mark__cell brand-mark__cell--cyan"></span>
  <span class="brand-mark__cell"></span>
  <span class="brand-mark__cell brand-mark__cell--pink"></span>
</span>

<style>
  .brand-mark {
    display: inline-grid;
    grid-template-columns: repeat(3, 5px);
    grid-template-rows: repeat(3, 5px);
    gap: 1px;
  }
  .brand-mark--sm {
    grid-template-columns: repeat(3, 4px);
    grid-template-rows: repeat(3, 4px);
  }
  .brand-mark__cell {
    background: currentColor;
    border-radius: 1px;
  }
  .brand-mark__cell--pink {
    background: var(--nc-pink-400);
  }
  .brand-mark__cell--cyan {
    background: var(--nc-cyan-400);
  }
</style>
