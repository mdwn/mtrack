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
  import { get } from "svelte/store";
  import { createSong } from "../../lib/api/songs";
  import { ApiError } from "../../lib/api/errors";

  interface Props {
    oncreated: (name: string) => void;
    oncancel: () => void;
  }

  let { oncreated, oncancel }: Props = $props();
  let name = $state("");
  let error = $state("");
  let saving = $state(false);

  async function submit() {
    const trimmed = name.trim().replace(/\/+$/, "");
    if (!trimmed) return;

    // The API path can include subdirectories (e.g. "Artist/Album/Song").
    // The song name in the YAML is the last segment.
    const songName = trimmed.split("/").filter(Boolean).pop() ?? trimmed;

    saving = true;
    error = "";
    try {
      const yaml = `name: "${songName}"\ntracks: []\n`;
      await createSong(trimmed, yaml);
      oncreated(songName);
    } catch (e) {
      if (e instanceof ApiError && e.kind === "exists") {
        error = get(t)("songs.create.alreadyExists");
        return;
      }
      error = e instanceof Error ? e.message : get(t)("songs.create.failed");
    } finally {
      saving = false;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter") submit();
    if (e.key === "Escape") oncancel();
  }
</script>

<div class="card create-form">
  <div class="form-row">
    <input
      class="input name-input"
      type="text"
      placeholder={$t("songs.create.placeholder")}
      bind:value={name}
      {onkeydown}
      disabled={saving}
    />
    <button
      class="btn btn-primary"
      onclick={submit}
      disabled={saving || !name.trim()}
    >
      {saving ? $t("common.creating") : $t("common.create")}
    </button>
    <button class="btn" onclick={oncancel} disabled={saving}
      >{$t("common.cancel")}</button
    >
  </div>
  {#if error}
    <div class="form-error">{error}</div>
  {/if}
</div>

<style>
  .create-form {
    margin-bottom: 16px;
  }
  .form-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .name-input {
    flex: 1;
  }
  .form-error {
    margin-top: 8px;
    font-size: 13px;
    color: var(--red);
  }
</style>
