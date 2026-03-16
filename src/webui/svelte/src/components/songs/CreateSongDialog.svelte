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
  import { createSong } from "../../lib/api/songs";

  interface Props {
    oncreated: (name: string) => void;
    oncancel: () => void;
  }

  let { oncreated, oncancel }: Props = $props();
  let name = $state("");
  let error = $state("");
  let saving = $state(false);

  async function submit() {
    const trimmed = name.trim();
    if (!trimmed) return;

    saving = true;
    error = "";
    try {
      const yaml = `name: "${trimmed}"\ntracks: []\n`;
      const res = await createSong(trimmed, yaml);
      if (res.status === 409) {
        error = "Song already exists";
        return;
      }
      if (!res.ok) {
        const data = await res.json().catch(() => null);
        error = data?.error ?? `Failed to create song (${res.status})`;
        return;
      }
      oncreated(trimmed);
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to create song";
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
      placeholder="Song name"
      bind:value={name}
      {onkeydown}
      disabled={saving}
    />
    <button
      class="btn btn-primary"
      onclick={submit}
      disabled={saving || !name.trim()}
    >
      {saving ? "Creating..." : "Create"}
    </button>
    <button class="btn" onclick={oncancel} disabled={saving}>Cancel</button>
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
