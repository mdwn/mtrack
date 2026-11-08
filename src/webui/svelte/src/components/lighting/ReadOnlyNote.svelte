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
<script lang="ts" module>
  import type { LightingOwner } from "../../gen/rules/mtrack_rules";

  /** An owner the UI may not write, or null: none listed (a new name, or a
   *  library GDTF) is not read-only. */
  export function readOnly(
    owner: LightingOwner | null | undefined,
  ): LightingOwner | null {
    return owner && !owner.editable ? owner : null;
  }
</script>

<script lang="ts">
  /* A venue or fixture type the UI may not write: its file is not that
     one thing's alone (shared, mixed or declared twice). Says which file
     and why, so it can be edited there by hand. */
  import { t } from "svelte-i18n";

  interface Props {
    owner: LightingOwner;
    /** `venue` or `ft`: the test ids are `<prefix>-read-only…`. */
    prefix: string;
  }

  let { owner, prefix }: Props = $props();
</script>

<div class="read-only-note" role="note" data-testid="{prefix}-read-only">
  <strong>{$t("lighting.readOnly.title")}</strong>
  <span data-testid="{prefix}-read-only-file">{owner.file}</span>
  <span data-testid="{prefix}-read-only-reason">{owner.reason}</span>
</div>

<style>
  .read-only-note {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--yellow);
    background: var(--yellow-subtle);
    border-radius: 4px;
    font-size: 13px;
    color: var(--text-muted);
  }

  .read-only-note strong {
    color: var(--text);
  }
</style>
