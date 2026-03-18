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
  interface SubsystemStatus {
    status: string;
    name: string | null;
  }

  interface ControllerStatus {
    kind: string;
    status: string;
    detail: string | null;
    error: string | null;
  }

  interface StatusResponse {
    build: {
      version: string;
      git_hash: string;
      build_time: string;
    };
    hardware: {
      init_done: boolean;
      hostname: string | null;
      profile: string | null;
      audio: SubsystemStatus;
      midi: SubsystemStatus;
      dmx: SubsystemStatus;
      trigger: SubsystemStatus;
    };
    controllers: ControllerStatus[];
  }

  let data = $state<StatusResponse | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function fetchStatus() {
    loading = true;
    error = null;
    try {
      const res = await fetch("/api/status");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      data = await res.json();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    fetchStatus();
  });

  function statusColor(status: string): string {
    if (status === "connected") return "var(--green)";
    if (status === "initializing") return "var(--yellow)";
    return "var(--text-dim)";
  }

  function statusLabel(status: string): string {
    if (status === "connected") return "Connected";
    if (status === "initializing") return "Initializing";
    return "Not Connected";
  }

  let restarting = $state(false);

  async function restartControllers() {
    restarting = true;
    try {
      const res = await fetch("/api/controllers/restart", { method: "POST" });
      if (!res.ok) {
        const body = await res.json().catch(() => null);
        error = body?.error ?? `Restart failed (${res.status})`;
      } else {
        await fetchStatus();
      }
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      restarting = false;
    }
  }

  const subsystems: { key: keyof StatusResponse["hardware"]; label: string }[] =
    [
      { key: "audio", label: "Audio" },
      { key: "midi", label: "MIDI" },
      { key: "dmx", label: "DMX" },
      { key: "trigger", label: "Trigger" },
    ];
</script>

<div class="status-page">
  <div class="status-header">
    <h2>Status</h2>
    <button class="refresh-btn" onclick={fetchStatus} disabled={loading}>
      {loading ? "Refreshing..." : "Refresh"}
    </button>
  </div>

  {#if error}
    <div class="error-banner">Failed to load status: {error}</div>
  {/if}

  {#if data}
    <div class="cards">
      <div class="card">
        <h3>Build Info</h3>
        <div class="info-grid">
          <span class="info-label">Version</span>
          <span class="info-value">{data.build.version}</span>
          <span class="info-label">Git Hash</span>
          <span class="info-value mono">{data.build.git_hash}</span>
          <span class="info-label">Build Time</span>
          <span class="info-value mono">{data.build.build_time}</span>
        </div>
      </div>

      <div class="card">
        <h3>Hardware</h3>
        {#if !data.hardware.init_done}
          <div class="init-banner">Hardware initialization in progress...</div>
        {/if}
        {#if data.hardware.hostname || data.hardware.profile}
          <div class="info-grid profile-info">
            {#if data.hardware.hostname}
              <span class="info-label">Hostname</span>
              <span class="info-value mono">{data.hardware.hostname}</span>
            {/if}
            {#if data.hardware.profile}
              <span class="info-label">Profile</span>
              <span class="info-value mono">{data.hardware.profile}</span>
            {/if}
          </div>
        {/if}
        <div class="subsystem-list">
          {#each subsystems as sub (sub.key)}
            {@const s = data.hardware[sub.key] as SubsystemStatus}
            <div class="subsystem-row">
              <div
                class="status-dot"
                style="background: {statusColor(s.status)}"
              ></div>
              <span class="subsystem-label">{sub.label}</span>
              <span class="subsystem-status">{statusLabel(s.status)}</span>
              {#if s.name}
                <span class="subsystem-name">{s.name}</span>
              {/if}
            </div>
          {/each}
        </div>
      </div>

      <div class="card">
        <div class="card-header-row">
          <h3>Controllers</h3>
          <button
            class="refresh-btn"
            onclick={restartControllers}
            disabled={restarting}
          >
            {restarting ? "Restarting..." : "Restart"}
          </button>
        </div>
        {#if data.controllers.length === 0}
          <div class="empty-note">No controllers configured</div>
        {:else}
          <div class="subsystem-list">
            {#each data.controllers as ctrl, i (`${i}:${ctrl.kind}`)}
              <div class="subsystem-row">
                <div
                  class="status-dot"
                  style="background: {ctrl.status === 'running'
                    ? 'var(--green)'
                    : 'var(--red)'}"
                ></div>
                <span class="subsystem-label">{ctrl.kind.toUpperCase()}</span>
                <span class="subsystem-status"
                  >{ctrl.status === "running" ? "Running" : "Error"}</span
                >
                {#if ctrl.detail}
                  <span class="subsystem-name">{ctrl.detail}</span>
                {/if}
              </div>
              {#if ctrl.error}
                <div class="controller-error">{ctrl.error}</div>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .status-page {
    max-width: 700px;
  }
  .status-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }
  .status-header h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .refresh-btn {
    padding: 6px 14px;
    font-size: 14px;
    font-weight: 500;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-card);
    color: var(--text);
    cursor: pointer;
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .refresh-btn:hover:not(:disabled) {
    background: var(--bg-card-hover, rgba(255, 255, 255, 0.05));
    border-color: var(--accent);
  }
  .refresh-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .error-banner {
    padding: 10px 14px;
    margin-bottom: 16px;
    border-radius: var(--radius);
    background: rgba(239, 68, 68, 0.12);
    color: var(--red);
    font-size: 14px;
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 16px 20px;
  }
  .card h3 {
    margin: 0 0 12px;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 16px;
    font-size: 14px;
  }
  .info-label {
    color: var(--text-muted);
  }
  .info-value {
    color: var(--text);
  }
  .info-value.mono {
    font-family: var(--mono, monospace);
  }
  .profile-info {
    margin-bottom: 12px;
  }
  .init-banner {
    padding: 8px 12px;
    margin-bottom: 12px;
    border-radius: var(--radius);
    background: rgba(234, 179, 8, 0.12);
    color: var(--yellow);
    font-size: 14px;
  }
  .subsystem-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .subsystem-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 14px;
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .subsystem-label {
    font-weight: 500;
    color: var(--text);
    min-width: 60px;
  }
  .subsystem-status {
    color: var(--text-muted);
  }
  .subsystem-name {
    color: var(--text-dim);
    font-size: 13px;
    margin-left: auto;
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 300px;
  }
  .card-header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  .card-header-row h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .empty-note {
    font-size: 14px;
    color: var(--text-dim);
  }
  .controller-error {
    font-size: 13px;
    color: var(--red);
    padding-left: 18px;
    margin-top: -4px;
    margin-bottom: 4px;
  }
</style>
