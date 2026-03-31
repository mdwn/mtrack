// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

import express from "express";
import { createServer } from "http";
import { WebSocketServer, WebSocket } from "ws";
import {
  SONGS,
  PLAYLISTS,
  PLAYLIST_DETAILS,
  CONFIG_STORE,
  STATUS,
  AUDIO_DEVICES,
  MIDI_DEVICES,
  PROFILE_FILES,
  PROFILE_FILE_DETAIL,
  PLAYBACK_STATE,
  METADATA_STATE,
  FIXTURE_STATE,
  WAVEFORM_DATA,
  LOG_LINES,
} from "./test-data.js";

const app = express();
app.use(express.json());
app.use(express.text({ type: "text/*" }));

// --- Songs ---

app.get("/api/songs", (_req, res) => {
  res.json(SONGS);
});

app.get("/api/songs/:name", (req, res) => {
  const song = SONGS.songs.find((s) => s.name === req.params.name);
  if (!song) return res.status(404).json({ error: "Song not found" });
  let yaml = `name: ${song.name}\ntracks:\n  - kick\n  - snare\n  - bass\n`;
  if (song.loop_playback) {
    yaml += `loop_playback: true\n`;
  }
  if (song.sections && song.sections.length > 0) {
    yaml += `sections:\n`;
    for (const s of song.sections) {
      yaml += `  - name: ${s.name}\n    start_measure: ${s.start_measure}\n    end_measure: ${s.end_measure}\n`;
    }
  }
  res.type("text/yaml").send(yaml);
});

app.post("/api/songs/:name", (_req, res) => {
  res.json({ status: "created" });
});

app.put("/api/songs/:name", (_req, res) => {
  res.json({ status: "updated" });
});

app.delete("/api/songs/:name", (_req, res) => {
  res.json({ status: "deleted" });
});

app.get("/api/songs/:name/waveform", (req, res) => {
  res.json({
    song_name: req.params.name,
    tracks: [
      { name: "kick", peaks: [0.5, 0.8, 0.3, 0.9, 0.2] },
      { name: "snare", peaks: [0.1, 0.4, 0.7, 0.2, 0.6] },
    ],
  });
});

app.get("/api/songs/:name/files", (_req, res) => {
  res.json({
    files: [
      { name: "kick.wav", type: "audio" },
      { name: "snare.wav", type: "audio" },
      { name: "bass.wav", type: "audio" },
    ],
  });
});

app.put("/api/songs/:name/tracks/:filename", (_req, res) => {
  res.json({ status: "uploaded" });
});

app.post("/api/songs/:name/tracks", (_req, res) => {
  res.json({ status: "uploaded" });
});

app.post("/api/songs/:name/import", (_req, res) => {
  res.json({ status: "imported" });
});

// --- Playlists ---

app.get("/api/playlists", (_req, res) => {
  res.json(PLAYLISTS);
});

app.get("/api/playlists/:name", (req, res) => {
  const detail = PLAYLIST_DETAILS[req.params.name];
  if (!detail) return res.status(404).json({ error: "Playlist not found" });
  res.json(detail);
});

app.put("/api/playlists/:name", (req, res) => {
  res.json({ status: "saved", name: req.params.name });
});

app.delete("/api/playlists/:name", (req, res) => {
  res.json({ status: "deleted", name: req.params.name });
});

app.post("/api/playlists/:name/activate", (req, res) => {
  res.json({ status: "activated", name: req.params.name });
});

// --- Config ---

app.get("/api/config/store", (_req, res) => {
  res.json(CONFIG_STORE);
});

// --- Devices ---

app.get("/api/devices/audio", (_req, res) => {
  res.json(AUDIO_DEVICES);
});

app.get("/api/devices/midi", (_req, res) => {
  res.json(MIDI_DEVICES);
});

// --- Profiles ---

app.post("/api/config/profiles", (_req, res) => {
  res.json({ ...CONFIG_STORE, checksum: "new-checksum-1" });
});

app.put("/api/config/profiles/:index", (_req, res) => {
  res.json({ ...CONFIG_STORE, checksum: "new-checksum-2" });
});

app.delete("/api/config/profiles/:index", (_req, res) => {
  res.json({ ...CONFIG_STORE, checksum: "new-checksum-3" });
});

app.get("/api/profiles", (_req, res) => {
  res.json(PROFILE_FILES);
});

app.get("/api/profiles/:filename", (_req, res) => {
  res.json(PROFILE_FILE_DETAIL);
});

app.put("/api/profiles/:filename", (_req, res) => {
  res.json({ status: "saved" });
});

app.delete("/api/profiles/:filename", (_req, res) => {
  res.json({ status: "deleted" });
});

// --- Samples ---

app.put("/api/config/samples", (_req, res) => {
  res.json({ ...CONFIG_STORE, checksum: "new-checksum-4" });
});

app.put("/api/samples/upload/:filename", (_req, res) => {
  res.json({
    status: "uploaded",
    file: "sample.wav",
    path: "/samples/sample.wav",
  });
});

// --- Status ---

app.get("/api/status", (_req, res) => {
  res.json(STATUS);
});

app.post("/api/controllers/restart", (_req, res) => {
  res.json({ status: "restarted", controllers: STATUS.controllers });
});

// --- Lock ---

app.get("/api/lock", (_req, res) => {
  res.json({ locked: false });
});

app.put("/api/lock", (req, res) => {
  res.json({ locked: req.body?.locked ?? false });
});

// --- Lighting ---

app.get("/api/lighting", (_req, res) => {
  res.json({ files: [{ path: "show.light", name: "show.light" }] });
});

app.get("/api/lighting/validate", (_req, res) => {
  res.json({ valid: true });
});

app.post("/api/lighting/validate", (_req, res) => {
  res.json({ valid: true });
});

app.get("/api/lighting/fixture-types", (_req, res) => {
  res.json({
    fixture_types: {
      par: {
        name: "par",
        channels: { red: 0, green: 1, blue: 2, dimmer: 3 },
        max_strobe_frequency: null,
        min_strobe_frequency: null,
        strobe_dmx_offset: null,
      },
    },
  });
});

app.get("/api/lighting/fixture-types/:name", (_req, res) => {
  res.json({
    fixture_type: {
      name: "par",
      channels: { red: 0, green: 1, blue: 2, dimmer: 3 },
      max_strobe_frequency: null,
      min_strobe_frequency: null,
      strobe_dmx_offset: null,
    },
    dsl: "fixture_type par { red: 0, green: 1, blue: 2, dimmer: 3 }",
  });
});

app.put("/api/lighting/fixture-types/:name", (_req, res) => {
  res.json({ status: "saved" });
});

app.delete("/api/lighting/fixture-types/:name", (_req, res) => {
  res.json({ status: "deleted" });
});

app.get("/api/lighting/venues", (_req, res) => {
  res.json({
    venues: {
      "test-venue": {
        name: "test-venue",
        fixtures: {
          "front-left": {
            name: "front-left",
            fixture_type: "par",
            universe: 1,
            start_channel: 1,
            tags: ["front", "left"],
          },
        },
        groups: {},
      },
    },
  });
});

app.get("/api/lighting/venues/:name", (_req, res) => {
  res.json({
    venue: {
      name: "test-venue",
      fixtures: {
        "front-left": {
          fixture_type: "par",
          universe: 1,
          start_channel: 1,
          tags: ["front", "left"],
        },
      },
      groups: {},
    },
    dsl: "venue test-venue {\n  fixture front-left par@1:1 [front, left]\n}",
  });
});

app.put("/api/lighting/venues/:name", (_req, res) => {
  res.json({ status: "saved" });
});

app.delete("/api/lighting/venues/:name", (_req, res) => {
  res.json({ status: "deleted" });
});

app.get("/api/lighting/:name", (_req, res) => {
  res.type("text/plain").send("// Lighting show\n");
});

app.put("/api/lighting/:name", (_req, res) => {
  res.json({ status: "saved" });
});

// --- Browse ---

app.get("/api/browse", (_req, res) => {
  res.json({ path: "/songs", root: "/songs", entries: [] });
});

app.post("/api/browse/create-song", (_req, res) => {
  res.json({ status: "created" });
});

app.post("/api/browse/bulk-import", (_req, res) => {
  res.json({ created: [], skipped: [], failed: [] });
});

// --- Calibration ---

app.post("/api/calibrate/start", (_req, res) => {
  res.json({
    peak: 0.01,
    rms: 0.005,
    low_freq_energy: 0.002,
    channel: 0,
    sample_rate: 48000,
    device_channels: 2,
  });
});

app.post("/api/calibrate/capture", (_req, res) => {
  res.json({ status: "capturing" });
});

app.post("/api/calibrate/stop", (_req, res) => {
  res.json({
    channel: 0,
    threshold: 0.1,
    gain: 1.0,
    scan_time_ms: 50,
    retrigger_time_ms: 200,
    num_hits_detected: 5,
    noise_floor_peak: 0.01,
    max_hit_amplitude: 0.8,
  });
});

app.delete("/api/calibrate", (_req, res) => {
  res.json({ status: "cancelled" });
});

// --- Test Control ---
// POST /test/send-ws — broadcast a WebSocket message to all connected clients.
// Used by Playwright tests to simulate state changes (e.g., playback starting).
// WebSocket connections indexed by wsId query parameter.
// Tests navigate to /?wsId=xxx, causing the app to connect with /ws?wsId=xxx.
// sendWsMessage targets a specific wsId so messages don't leak between tests.
const wsConnections = new Map<string, import("ws").WebSocket>();

// Send a WebSocket message to the connection identified by wsId.
app.post("/test/send-ws", (req, res) => {
  const { _wsId, ...payload } = req.body;
  const wsId = _wsId as string;
  const msg = JSON.stringify(payload);

  if (!wsId) {
    // No wsId: broadcast to all (backward compat for tests that don't use namespacing).
    let sent = 0;
    for (const client of wss.clients) {
      if (client.readyState === WebSocket.OPEN) {
        client.send(msg);
        sent++;
      }
    }
    res.json({ sent });
    return;
  }

  const ws = wsConnections.get(wsId);
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(msg);
    res.json({ sent: 1, wsId });
  } else {
    res.json({ sent: 0, wsId });
  }
});

// --- HTTP + WebSocket server ---

const server = createServer(app);
const wss = new WebSocketServer({ server, path: "/ws" });

wss.on("connection", (ws, req) => {
  // Extract wsId from query parameter for test isolation.
  const url = new URL(req.url ?? "", "http://localhost");
  const wsId = url.searchParams.get("wsId");
  if (wsId) {
    wsConnections.set(wsId, ws);
    ws.on("close", () => wsConnections.delete(wsId));
  }
  // Send initial state in the same order as the real server.
  ws.send(JSON.stringify(METADATA_STATE));

  setTimeout(() => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(PLAYBACK_STATE));
    }
  }, 50);

  setTimeout(() => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(FIXTURE_STATE));
    }
  }, 100);

  setTimeout(() => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(WAVEFORM_DATA));
    }
  }, 150);

  setTimeout(() => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(LOG_LINES));
    }
  }, 200);
});

const PORT = 3111;
server.listen(PORT, "127.0.0.1", () => {
  console.log(`Mock server listening on http://127.0.0.1:${PORT}`);
});
