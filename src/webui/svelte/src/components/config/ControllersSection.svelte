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
    controllers: any[];
    onchange: () => void;
  }

  let { controllers = $bindable(), onchange }: Props = $props();

  let showOscAdvanced: Record<number, boolean> = $state({});
  let nextId = $state(controllers.length);

  // Assign stable keys to controllers for Svelte's keyed each block.
  // Initialize synchronously to avoid undefined keys on first render.
  let controllerKeys = $state<number[]>(
    Array.from({ length: controllers.length }, (_, i) => i),
  );
  $effect(() => {
    while (controllerKeys.length < controllers.length) {
      controllerKeys.push(nextId++);
    }
    if (controllerKeys.length > controllers.length) {
      controllerKeys.length = controllers.length;
    }
  });

  function addController(kind: string) {
    if (kind === "grpc") {
      controllers.push({ kind: "grpc" });
    } else if (kind === "osc") {
      controllers.push({ kind: "osc" });
    }
    controllerKeys.push(nextId++);
    onchange();
  }

  function removeController(i: number) {
    controllers.splice(i, 1);
    controllerKeys.splice(i, 1);
    onchange();
  }

  function updateField(i: number, key: string, value: any) {
    if (value === undefined || value === "") {
      delete controllers[i][key];
    } else {
      controllers[i][key] = value;
    }
    onchange();
  }

  // OSC broadcast address helpers
  function addBroadcastAddr(i: number) {
    if (!controllers[i].broadcast_addresses)
      controllers[i].broadcast_addresses = [];
    controllers[i].broadcast_addresses.push("");
    onchange();
  }

  function removeBroadcastAddr(ci: number, ai: number) {
    controllers[ci].broadcast_addresses.splice(ai, 1);
    if (controllers[ci].broadcast_addresses.length === 0) {
      delete controllers[ci].broadcast_addresses;
    }
    onchange();
  }

  function updateBroadcastAddr(ci: number, ai: number, value: string) {
    controllers[ci].broadcast_addresses[ai] = value;
    onchange();
  }

  const oscPaths = [
    ["play", "/mtrack/play"],
    ["prev", "/mtrack/prev"],
    ["next", "/mtrack/next"],
    ["stop", "/mtrack/stop"],
    ["all_songs", "/mtrack/all_songs"],
    ["playlist", "/mtrack/playlist"],
    ["stop_samples", "/mtrack/samples/stop"],
    ["status", "/mtrack/status"],
    ["playlist_current", "/mtrack/playlist/current"],
    ["playlist_current_song", "/mtrack/playlist/current_song"],
    ["playlist_current_song_elapsed", "/mtrack/playlist/current_song/elapsed"],
  ];

  function toggleOscAdvanced(i: number) {
    showOscAdvanced[i] = !showOscAdvanced[i];
  }
</script>

<div class="section-fields">
  {#each controllers as ctrl, i (controllerKeys[i])}
    <div class="controller-card">
      <div class="controller-header">
        <span class="controller-kind">{ctrl.kind?.toUpperCase()}</span>
        <button
          class="btn btn-danger btn-sm"
          onclick={() => removeController(i)}>Remove</button
        >
      </div>

      {#if ctrl.kind === "grpc"}
        <div class="field">
          <label for="ctrl-grpc-port-{i}">Port</label>
          <input
            id="ctrl-grpc-port-{i}"
            type="number"
            class="input"
            placeholder="43234"
            value={ctrl.port ?? ""}
            onchange={(e) => {
              const v = (e.target as HTMLInputElement).value;
              updateField(i, "port", v ? parseInt(v) : undefined);
            }}
          />
        </div>
      {:else if ctrl.kind === "osc"}
        <div class="field">
          <label for="ctrl-osc-port-{i}">Port</label>
          <input
            id="ctrl-osc-port-{i}"
            type="number"
            class="input"
            placeholder="43235"
            value={ctrl.port ?? ""}
            onchange={(e) => {
              const v = (e.target as HTMLInputElement).value;
              updateField(i, "port", v ? parseInt(v) : undefined);
            }}
          />
        </div>

        <div class="field">
          <div class="field-header">
            <span class="field-label">Broadcast Addresses</span>
            <button class="btn" onclick={() => addBroadcastAddr(i)}>Add</button>
          </div>
          {#each ctrl.broadcast_addresses || [] as addr, ai (ai)}
            <div class="addr-row">
              <input
                class="input"
                value={addr}
                placeholder="192.168.1.255:43235"
                onchange={(e) =>
                  updateBroadcastAddr(
                    i,
                    ai,
                    (e.target as HTMLInputElement).value.trim(),
                  )}
              />
              <button
                class="btn btn-danger"
                onclick={() => removeBroadcastAddr(i, ai)}>X</button
              >
            </div>
          {/each}
        </div>

        <button class="btn btn-expand" onclick={() => toggleOscAdvanced(i)}>
          {showOscAdvanced[i] ? "Hide" : "Show"} OSC Path Overrides
        </button>

        {#if showOscAdvanced[i]}
          <div class="osc-paths">
            {#each oscPaths as [key, defaultPath] (key)}
              <div class="field">
                <label for="osc-{key}-{i}">{key}</label>
                <input
                  id="osc-{key}-{i}"
                  class="input"
                  placeholder={defaultPath}
                  value={ctrl[key] ?? ""}
                  onchange={(e) =>
                    updateField(
                      i,
                      key,
                      (e.target as HTMLInputElement).value.trim() || undefined,
                    )}
                />
              </div>
            {/each}
          </div>
        {/if}
      {:else if ctrl.kind === "midi"}
        <div class="note">Edit MIDI controllers in raw YAML.</div>
      {/if}
    </div>
  {/each}

  <div class="add-buttons">
    <button class="btn" onclick={() => addController("grpc")}>Add gRPC</button>
    <button class="btn" onclick={() => addController("osc")}>Add OSC</button>
  </div>
</div>

<style>
  .section-fields {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .controller-card {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .controller-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .controller-kind {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label,
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
  }
  .field-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .addr-row {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .addr-row .input {
    flex: 1;
  }
  .btn-sm {
    padding: 3px 8px;
    font-size: 11px;
  }
  .btn-expand {
    font-size: 12px;
    align-self: flex-start;
  }
  .osc-paths {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .add-buttons {
    display: flex;
    gap: 8px;
  }
  .note {
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
  }
  @media (max-width: 600px) {
    .osc-paths {
      grid-template-columns: 1fr;
    }
  }
</style>
