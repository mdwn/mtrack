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
</script>

<div class="page-placeholder">
  <h2>{$t("notFound.title")}</h2>
  <p>{$t("notFound.message")}</p>
  <a href="#/" class="btn back-btn">Back to Dashboard</a>
</div>

<style>
  .back-btn {
    display: inline-block;
    margin-top: 16px;
    padding: 8px 16px;
    font-size: 14px;
    font-weight: 500;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-card);
    color: var(--text);
    text-decoration: none;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
  }
  .back-btn:hover {
    background: var(--bg-card-hover, rgba(255, 255, 255, 0.05));
    border-color: var(--accent);
  }
</style>
