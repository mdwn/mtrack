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
  let hasTrigger = $derived(!!profile.trigger);
  let hasControllers = $derived(
    profile.controllers && profile.controllers.length > 0,
  );

  let audioDevice = $derived(profile.audio?.device || "");
  let midiDevice = $derived(profile.midi?.device || "");
</script>

<button class="profile-row" {onclick}>
  <span class="row-index">#{index}</span>
  <span class="row-hostname">{hostname}</span>
  <div class="row-badges">
    {#if hasAudio}<span class="badge badge-audio">AUDIO</span>{/if}
    {#if hasMidi}<span class="badge badge-midi">MIDI</span>{/if}
    {#if hasDmx}<span class="badge badge-dmx">DMX</span>{/if}
    {#if hasTrigger}<span class="badge badge-trigger">TRIGGER</span>{/if}
    {#if hasControllers}<span class="badge badge-ctrl">CTRL</span>{/if}
  </div>
  {#if audioDevice || midiDevice}
    <span class="row-devices">
      {#if audioDevice}{audioDevice}{/if}
      {#if audioDevice && midiDevice}
        /
      {/if}
      {#if midiDevice}{midiDevice}{/if}
    </span>
  {/if}
</button>

<style>
  .profile-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    text-align: left;
    width: 100%;
    font-family: var(--sans);
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .profile-row:hover {
    background: var(--bg-card-hover);
    border-color: var(--text-dim);
  }
  .row-index {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .row-hostname {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    flex-shrink: 0;
  }
  .row-badges {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }
  .badge {
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.5px;
    padding: 2px 5px;
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
  .badge-trigger {
    background: rgba(168, 85, 247, 0.15);
    color: #a855f7;
  }
  .badge-ctrl {
    background: var(--red-dim);
    color: var(--red);
  }
  .row-devices {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-dim);
    font-family: var(--mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
  }
</style>
