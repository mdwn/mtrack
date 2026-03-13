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
  interface Props {
    profile: any;
    index: number;
    onclick: () => void;
  }

  let { profile, index, onclick }: Props = $props();

  let hostname = $derived(profile.hostname || "Default Profile");
  let hasAudio = $derived(!!profile.audio);
  let hasMidi = $derived(!!profile.midi);
  let hasDmx = $derived(!!profile.dmx);
  let hasControllers = $derived(
    profile.controllers && profile.controllers.length > 0,
  );

  let audioDevice = $derived(profile.audio?.device || "");
  let midiDevice = $derived(profile.midi?.device || "");
</script>

<button class="profile-card" {onclick}>
  <div class="card-top">
    <span class="card-index">#{index}</span>
    <span class="card-hostname">{hostname}</span>
  </div>
  <div class="badges">
    {#if hasAudio}<span class="badge badge-audio">AUDIO</span>{/if}
    {#if hasMidi}<span class="badge badge-midi">MIDI</span>{/if}
    {#if hasDmx}<span class="badge badge-dmx">DMX</span>{/if}
    {#if hasControllers}<span class="badge badge-ctrl">CTRL</span>{/if}
  </div>
  {#if audioDevice || midiDevice}
    <div class="device-meta">
      {#if audioDevice}<span>{audioDevice}</span>{/if}
      {#if midiDevice}<span>{midiDevice}</span>{/if}
    </div>
  {/if}
</button>

<style>
  .profile-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 16px;
    cursor: pointer;
    text-align: left;
    transition:
      background 0.15s,
      border-color 0.15s;
    width: 100%;
    font-family: var(--sans);
  }
  .profile-card:hover {
    background: var(--bg-card-hover);
    border-color: var(--text-dim);
  }
  .card-top {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .card-index {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-dim);
  }
  .card-hostname {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .badges {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 6px;
  }
  .badge {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.5px;
    padding: 2px 6px;
    border-radius: 3px;
  }
  .badge-audio {
    background: var(--accent);
    color: #fff;
  }
  .badge-midi {
    background: var(--green-dim);
    color: var(--green);
  }
  .badge-dmx {
    background: var(--yellow-dim);
    color: var(--yellow);
  }
  .badge-ctrl {
    background: var(--red-dim);
    color: var(--red);
  }
  .device-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 11px;
    color: var(--text-dim);
    font-family: var(--mono);
  }
</style>
