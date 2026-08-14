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

  interface SubsystemStatus {
    status: string;
    name: string | null;
    /** Why init gave up. Present only for `failed`. */
    error?: string | null;
  }

  interface ControllerStatus {
    kind: string;
    status: string;
    detail: string | null;
    error: string | null;
  }

  /** Runtime state of the audio output callback, as distinct from whether the
   * device opened. `status` is the verdict; the rest is the evidence. */
  interface AudioOutputHealth {
    status: "healthy" | "recovering" | "stalled" | "never_started";
    callback_alive: boolean;
    writing_signal: boolean;
    since_last_callback_ms: number | null;
    since_last_signal_ms: number | null;
    callbacks: number;
    recoveries: number;
    underruns: number;
    last_error: string | null;
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
      /** Present once an audio device is open and able to report. */
      audio_output: AudioOutputHealth | null;
      midi: SubsystemStatus;
      dmx: SubsystemStatus;
      trigger: SubsystemStatus;
    };
    controllers: ControllerStatus[];
  }

  let data = $state<StatusResponse | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let lastFetchTime = $state<number>(Date.now());
  let secondsAgo = $state<number>(0);

  async function fetchStatus() {
    loading = true;
    error = null;
    try {
      const res = await fetch("/api/status");
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      data = await res.json();
      lastFetchTime = Date.now();
      secondsAgo = 0;
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    fetchStatus();
    const poll = setInterval(() => fetchStatus(), 5000);
    const tick = setInterval(() => {
      secondsAgo = Math.round((Date.now() - lastFetchTime) / 1000);
    }, 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  });

  function statusColor(status: string): string {
    if (status === "connected") return "var(--green)";
    if (status === "initializing") return "var(--yellow)";
    // Red rather than dim: `failed` means init stopped trying, which needs
    // someone to act. Dim reads as "not configured", which needs nobody.
    if (status === "failed") return "var(--red)";
    return "var(--text-dim)";
  }

  /** Audio health, but only once it is worth saying something about.
   *
   * `connected` on the audio subsystem means "a device handle exists", which
   * stays true while the interface is physically unplugged. Without this the
   * page shows a green dot at the exact moment someone is standing in a silent
   * room wondering what went wrong.
   *
   * `never_started` is deliberately not surfaced: it is the ordinary state for
   * the few milliseconds between opening a device and its first callback, and
   * flagging it would make every start look broken. */
  function audioFault(): AudioOutputHealth | null {
    const health = data?.hardware.audio_output;
    if (!health) return null;
    if (health.status === "healthy" || health.status === "never_started")
      return null;
    return health;
  }

  function audioDotColor(s: SubsystemStatus): string {
    const fault = audioFault();
    if (!fault) return statusColor(s.status);
    // Recovering is amber: something is wrong but it is being worked on.
    return fault.status === "recovering" ? "var(--yellow)" : "var(--red)";
  }

  /** The history worth showing under the audio row, or null if there is none.
   *
   * Neither of these is visible at any single instant, which is the whole point:
   * a rig that rebuilt its stream three times during the last set reads as
   * perfectly healthy right now, and the reason it did is the thing someone
   * wants after the show rather than during it. */
  function audioHistory(): string | null {
    const health = data?.hardware.audio_output;
    if (!health) return null;
    const parts: string[] = [];
    if (health.recoveries > 0)
      parts.push(
        get(t)("status.audioRecoveries", {
          values: { count: health.recoveries },
        }),
      );
    if (health.underruns > 0)
      parts.push(
        get(t)("status.audioUnderruns", {
          values: { count: health.underruns },
        }),
      );
    if (health.last_error) parts.push(health.last_error);
    return parts.length ? parts.join(" · ") : null;
  }

  function audioStatusLabel(s: SubsystemStatus): string {
    const fault = audioFault();
    if (!fault) return statusLabel(s);
    return fault.status === "recovering"
      ? get(t)("status.audioRecovering")
      : get(t)("status.audioStalled");
  }

  function statusLabel(s: SubsystemStatus): string {
    if (s.status === "connected") return get(t)("status.connected");
    if (s.status === "initializing") return get(t)("status.initializing");
    // Checked before the `name` fallback below: a failed subsystem has no name
    // to report, so that fallback would label it "not configured" — the one
    // reading that tells the operator there is nothing to fix.
    if (s.status === "failed") return get(t)("status.failed");
    if (!s.name && s.status !== "connected")
      return get(t)("status.notConfigured");
    return get(t)("status.notConnected");
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

  const subsystems: {
    key: keyof StatusResponse["hardware"];
    labelKey: string;
    /** Section slug within the profile editor URL. */
    section: string;
  }[] = [
    { key: "audio", labelKey: "status.audio", section: "audio" },
    { key: "midi", labelKey: "status.midi", section: "midi" },
    // DMX configuration lives inside the profile's Lighting section.
    { key: "dmx", labelKey: "status.dmx", section: "lighting" },
    { key: "trigger", labelKey: "status.trigger", section: "trigger" },
  ];

  /** Build a deep-link to the profile editor for a given section. */
  function configureLink(section: string): string {
    const profile = data?.hardware.profile;
    if (!profile) return "#/config";
    return `#/config/${encodeURIComponent(profile)}/${section}`;
  }
</script>

<div class="status-page">
  <div class="page__head">
    <div>
      <h1 class="page__title">{$t("status.title")}</h1>
      <p class="page__subtitle">
        {$t("status.lastUpdated", { values: { seconds: secondsAgo } })}
      </p>
    </div>
    <div class="refresh-group">
      <button class="btn" onclick={fetchStatus} disabled={loading}>
        {loading ? $t("common.refreshing") : $t("common.refresh")}
      </button>
    </div>
  </div>

  {#if error}
    <div class="error-banner">
      {$t("status.failedToLoad", { values: { error } })}
    </div>
  {/if}

  {#if data}
    <div class="cards">
      <div class="card">
        <h3>{$t("status.hardware")}</h3>
        {#if !data.hardware.init_done}
          <div class="init-banner">{$t("status.hardwareInit")}</div>
        {/if}
        {#if data.hardware.hostname || data.hardware.profile}
          <div class="info-grid profile-info">
            {#if data.hardware.hostname}
              <span class="info-label">{$t("status.hostname")}</span>
              <span class="info-value mono">{data.hardware.hostname}</span>
            {/if}
            {#if data.hardware.profile}
              <span class="info-label">{$t("status.profile")}</span>
              <span class="info-value mono">{data.hardware.profile}</span>
            {/if}
          </div>
        {/if}
        <div class="subsystem-list">
          {#each subsystems as sub (sub.key)}
            {@const s = data.hardware[sub.key] as SubsystemStatus}
            {@const needsConfig = s.status !== "connected"}
            {@const isAudio = sub.key === "audio"}
            {@const dotColor = isAudio
              ? audioDotColor(s)
              : statusColor(s.status)}
            {@const label = isAudio ? audioStatusLabel(s) : statusLabel(s)}
            <div class="subsystem-row">
              <div
                class="status-dot"
                style="background: {dotColor}"
                role="img"
                aria-label={label}
              ></div>
              <span class="subsystem-label">{$t(sub.labelKey)}</span>
              <span class="subsystem-status">{label}</span>
              {#if s.name}
                <span class="subsystem-name">{s.name}</span>
              {/if}
              {#if isAudio}
                {@const history = audioHistory()}
                {#if history}
                  <span class="subsystem-history" title={history}
                    >{history}</span
                  >
                {/if}
              {/if}
              {#if needsConfig}
                <a
                  class="subsystem-link"
                  href={configureLink(sub.section)}
                  aria-label={s.name
                    ? $t("status.fixSubsystem", {
                        values: { name: $t(sub.labelKey) },
                      })
                    : $t("status.configureSubsystem", {
                        values: { name: $t(sub.labelKey) },
                      })}
                >
                  {s.name ? $t("status.fix") : $t("status.configure")}
                  <span aria-hidden="true">→</span>
                </a>
              {/if}
            </div>
            {#if s.error}
              <div class="subsystem-error">{s.error}</div>
            {/if}
          {/each}
        </div>
      </div>

      <div class="card">
        <div class="card-header-row">
          <h3>{$t("status.controllers")}</h3>
          <button
            class="btn"
            onclick={restartControllers}
            disabled={restarting}
          >
            {restarting ? $t("status.restarting") : $t("status.restart")}
          </button>
        </div>
        {#if data.controllers.length === 0}
          <div class="empty-note">{$t("status.noControllers")}</div>
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
                  >{ctrl.status === "running"
                    ? $t("status.running")
                    : $t("status.error")}</span
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

      <div class="card">
        <h3>{$t("status.buildInfo")}</h3>
        <div class="info-grid">
          <span class="info-label">{$t("status.version")}</span>
          <span class="info-value">{data.build.version}</span>
          <span class="info-label">{$t("status.gitHash")}</span>
          <span class="info-value mono">{data.build.git_hash}</span>
          <span class="info-label">{$t("status.buildTime")}</span>
          <span class="info-value mono">{data.build.build_time}</span>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .status-page {
    max-width: 1100px;
  }
  .refresh-group {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .cards {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 24px;
  }
  .cards .card:last-child {
    grid-column: 1 / -1;
  }
  @media (max-width: 900px) {
    .cards {
      grid-template-columns: 1fr;
    }
    .cards .card:last-child {
      grid-column: auto;
    }
  }
  .card {
    padding: 20px 24px;
  }
  .card h3 {
    margin: 0 0 16px;
    font-family: var(--nc-font-display);
    font-weight: 700;
    font-size: 18px;
    color: var(--nc-fg-1);
  }
  .info-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 8px 20px;
    font-size: 14px;
  }
  .info-label {
    font-family: var(--nc-font-sans);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--nc-fg-3);
    align-self: center;
  }
  .info-value {
    color: var(--nc-fg-1);
  }
  .info-value.mono {
    font-family: var(--nc-font-mono);
    font-size: 13px;
  }
  .profile-info {
    margin-bottom: 16px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--card-border);
  }
  .init-banner {
    padding: 10px 14px;
    margin-bottom: 16px;
    border-radius: var(--nc-radius-sm);
    background: rgba(242, 181, 68, 0.14);
    color: #b47a1a;
    border: 1px solid rgba(242, 181, 68, 0.4);
    font-size: 13px;
  }
  :global(.nc--dark) .init-banner {
    color: var(--nc-warn);
  }
  .subsystem-history {
    color: var(--text-dim);
    font-size: 0.8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .subsystem-list {
    display: flex;
    flex-direction: column;
  }
  .subsystem-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 0;
    font-family: var(--nc-font-sans);
    font-size: 14px;
    border-bottom: 1px solid var(--card-border);
  }
  .subsystem-row:last-child {
    border-bottom: none;
  }
  .status-dot {
    width: 10px;
    height: 10px;
    border-radius: 999px;
    flex-shrink: 0;
    box-shadow: 0 0 6px currentColor;
  }
  .subsystem-label {
    font-family: var(--nc-font-display);
    font-weight: 700;
    color: var(--nc-fg-1);
    min-width: 80px;
  }
  .subsystem-status {
    font-family: var(--nc-font-mono);
    font-size: 13px;
    color: var(--nc-fg-2);
    text-transform: capitalize;
  }
  .subsystem-name {
    color: var(--nc-fg-3);
    font-size: 13px;
    margin-left: auto;
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 360px;
    font-family: var(--nc-font-mono);
  }
  .subsystem-link {
    margin-left: auto;
    font-family: var(--nc-font-display);
    font-weight: 600;
    font-size: 13px;
    color: var(--nc-cyan-600);
    text-decoration: none;
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid rgba(94, 202, 234, 0.45);
    background: rgba(94, 202, 234, 0.1);
    white-space: nowrap;
    transition:
      background var(--nc-dur-fast) var(--nc-ease),
      color var(--nc-dur-fast) var(--nc-ease);
  }
  :global(.nc--dark) .subsystem-link {
    color: var(--nc-cyan-300);
  }
  .subsystem-link:hover {
    background: rgba(94, 202, 234, 0.2);
    color: var(--nc-cyan-700);
  }
  /* Push the name back to its mid position when a fix link is also shown. */
  .subsystem-row:has(.subsystem-link) .subsystem-name {
    margin-left: 0;
    text-align: left;
    flex: 1;
  }
  .card-header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }
  .card-header-row h3 {
    margin: 0;
  }
  .empty-note {
    font-size: 14px;
    color: var(--nc-fg-3);
    font-style: italic;
  }
  /* Shares the controller-error shape: same question, same answer. */
  .subsystem-error,
  .controller-error {
    font-family: var(--nc-font-mono);
    font-size: 12px;
    color: var(--nc-error);
    padding-left: 22px;
    margin-bottom: 8px;
    margin-top: -4px;
  }
</style>
