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
  /**
   * Pairing phones with the Bluetooth controller, in its card: whether the
   * controller runs, the pairing window and its countdown, and the phones
   * paired. The server is asked every 2 seconds while the window is open
   * and every 10 otherwise, and the countdown runs on the clock between.
   */
  import type { BluetoothStatus } from "../../gen/rules/mtrack_rules";
  import { onDestroy, onMount } from "svelte";
  import { t } from "svelte-i18n";
  import { get } from "svelte/store";
  import { playbackStore } from "../../lib/ws/stores";
  import { showAlert, showConfirm } from "../../lib/dialog.svelte";
  import {
    fetchBluetooth,
    forgetBluetoothPhone,
    setBluetoothPairing,
  } from "../../lib/api/bluetooth";

  /** How long "Pair a phone" keeps the window open. */
  const PAIRING_SECONDS = 120;
  const POLL_OPEN_MS = 2000;
  const POLL_CLOSED_MS = 10000;

  let status = $state<BluetoothStatus | null>(null);
  /** Why the last read failed, when it did. */
  let readError = $state<string | null>(null);
  let busy = $state(false);
  /** When the open window closes, by the local clock. */
  let deadline = $state(0);
  let now = $state(Date.now());

  let timer: ReturnType<typeof setTimeout> | undefined;
  let ticker: ReturnType<typeof setInterval> | undefined;
  let destroyed = false;
  let abort: AbortController | undefined;

  let open = $derived(status?.pairing.open ?? false);
  let secondsLeft = $derived(
    open ? Math.max(0, Math.ceil((deadline - now) / 1000)) : 0,
  );
  let locked = $derived($playbackStore.locked);

  function apply(next: BluetoothStatus) {
    status = next;
    readError = null;
    deadline = Date.now() + next.pairing.seconds_left * 1000;
    now = Date.now();
  }

  function clock(seconds: number): string {
    const s = String(seconds % 60).padStart(2, "0");
    return `${Math.floor(seconds / 60)}:${s}`;
  }

  async function refresh() {
    if (destroyed || busy) return;
    const mine = new AbortController();
    abort = mine;
    try {
      apply(await fetchBluetooth(mine.signal));
    } catch (e) {
      // Cancelled by a change or by leaving: not a failure to read.
      if (destroyed || mine.signal.aborted) return;
      readError = e instanceof Error ? e.message : String(e);
    }
  }

  async function poll() {
    if (!document.hidden) await refresh();
    if (destroyed) return;
    timer = setTimeout(poll, open ? POLL_OPEN_MS : POLL_CLOSED_MS);
  }

  /** Runs a change: its answer is the new state, and a refusal is shown. */
  async function change(action: () => Promise<BluetoothStatus>) {
    busy = true;
    abort?.abort();
    try {
      apply(await action());
    } catch (e) {
      await showAlert(e instanceof Error ? e.message : String(e));
      await refresh();
    } finally {
      busy = false;
      // The next poll is paced by the state the change left.
      clearTimeout(timer);
      if (!destroyed)
        timer = setTimeout(poll, open ? POLL_OPEN_MS : POLL_CLOSED_MS);
    }
  }

  async function forget(address: string, name: string | null) {
    const label = name || address;
    const sure = await showConfirm(
      get(t)("bluetooth.confirmForget", { values: { name: label } }),
      { danger: true, confirmLabel: get(t)("bluetooth.forget") },
    );
    if (sure) await change(() => forgetBluetoothPhone(address));
  }

  onMount(() => {
    poll();
    ticker = setInterval(() => {
      now = Date.now();
      // The window ran out here: ask, rather than wait for the next poll.
      if (open && deadline <= now && !busy) refresh();
    }, 1000);
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(timer);
    clearInterval(ticker);
    abort?.abort();
  });
</script>

<div class="bluetooth" data-testid="bluetooth-pairing">
  {#if status === null}
    {#if readError}
      <p class="status status--bad">
        {$t("bluetooth.unreadable", { values: { error: readError } })}
      </p>
    {/if}
  {:else if !status.configured}
    <p class="status">{$t("bluetooth.notSaved")}</p>
  {:else if !status.running}
    <p class="status status--bad">
      {$t("bluetooth.notRunning")}{status.error ? `: ${status.error}` : ""}
    </p>
  {:else}
    <p class="status" class:status--open={open} aria-live="polite">
      {#if open}
        {$t("bluetooth.pairingOpen", {
          values: { left: clock(secondsLeft) },
        })}
      {:else}
        {$t("bluetooth.pairingClosed")}
      {/if}
    </p>
    {#if status.error}
      <p class="status status--bad">{status.error}</p>
    {/if}
    <div class="actions">
      {#if open}
        <button
          class="btn"
          disabled={busy || locked}
          title={locked ? $t("common.locked") : null}
          onclick={() => change(() => setBluetoothPairing(false))}
          >{$t("bluetooth.stopPairing")}</button
        >
      {:else}
        <button
          class="btn btn-primary"
          disabled={busy || locked}
          title={locked ? $t("common.locked") : null}
          onclick={() =>
            change(() => setBluetoothPairing(true, PAIRING_SECONDS))}
          >{$t("bluetooth.pairPhone")}</button
        >
      {/if}
    </div>
    {#if open}
      <p class="hint-text">{$t("bluetooth.pairingHint")}</p>
    {/if}
    <div class="phones">
      <span class="phones-label">{$t("bluetooth.phones")}</span>
      {#if status.devices.length === 0}
        <p class="hint-text">{$t("bluetooth.noPhones")}</p>
      {:else}
        <ul class="phone-list">
          {#each status.devices as phone (phone.address)}
            <li class="phone">
              <span class="phone-name">{phone.name || phone.address}</span>
              {#if phone.connected}
                <span class="badge badge--active"
                  >{$t("bluetooth.connected")}</span
                >
              {/if}
              <button
                class="btn btn-danger btn-sm phone-forget"
                disabled={busy || locked}
                title={locked ? $t("common.locked") : null}
                onclick={() => forget(phone.address, phone.name)}
                >{$t("bluetooth.forget")}</button
              >
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</div>

<style>
  .bluetooth {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .status {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .status--open {
    color: var(--text);
    font-weight: 600;
  }
  .status--bad {
    color: var(--red);
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .hint-text {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
  }
  .phones {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .phones-label {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .phone-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .phone {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .phone-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .btn-sm {
    padding: 3px 8px;
    font-size: 12px;
  }
</style>
