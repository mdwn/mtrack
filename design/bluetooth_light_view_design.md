# Bluetooth remote: the light view

*Design doc, draft 1 — 2026-11-10. Phase C3 of
`design/bluetooth_remote_phase2_design.md`, which left this to be decided.
Decided with Mike since: mtrack keeps one lighting visualization, the web UI's,
and the phone reuses it; the phone gets the plot, with movers, and 3D after it;
the feed is a gRPC stream. A spike the same day ran the web UI's stage view with
no server behind it (§3). It has since run on a Pixel (§3).*

## 1. What it is for

Seeing from the stage that the rig is doing what the show says, without a
laptop. It is a monitor: nothing on it controls a light.

## 2. One visualization

The web UI's stage view is the only one. The app does not redraw it: the Lights
tab is a web view running the web UI's own components, from a copy shipped in
the app, and the data comes over Bluetooth instead of a websocket.

What stays single:

- **The drawing**: `StageView.svelte` (the plot) and `Stage3DView.svelte` with
  `scene3d.ts` (3D).
- **The lighting rules**: a fixture's colour and whether it is lit are the
  player's own functions, which the web UI already calls through WebAssembly.
- **The data**: one set of messages, defined once (§4).

The terminal UI's text display is a separate, older thing and is not touched.

## 3. What the spike showed

A stage-only build of the web UI, fed a recording from the Pi 4 rig, in desktop
Chromium with no backend (branch `stage-spike`, not for merging):

- The live read-only view makes no REST calls. The 3D view fetches the rig
  description and one model per fixture type from `/api/lighting/assets/`:
  25.7 kB for the rig's eight fixtures.
- Two existing web UI files changed: the websocket's message dispatch became
  callable, and the socket no longer opens as a side effect of an import.
- Size, compressed: about 90 kB for the plot, 260 kB with 3D (three.js is most
  of it), and a 470 kB WebAssembly module that carries the whole rules crate.
- The rig's feed today: 1.4 kB of JSON per update, 20 a second. More than half
  is poses for fixtures that do not move.

Then on a Pixel, in the app's web view, from the same recording (L0,
2026-11-10): the plot and the 3D view both draw, 3D turns under a finger, and
the page renders at about 100 frames a second while taking 20 updates a
second. Mike: "pretty smooth". So the reuse in §2 stands.

One thing it turned up: the 3D view rebuilds its whole scene when a layout
arrives, which shows as a blank for a fraction of a second. The spike sent
the layout at every loop of its recording; sending it once removed the blank.
The player's stream can send a layout that says nothing new (a venue reload
that changed nothing), so the page keeps its scene when the layout is the one
it has (§8).

Not shown by the spike: movers, per-cell fixtures and scenery (the rig has
none); iOS. The phone run reported two files not found and one page error
that were not read; L3 serves its own files and looks at them then.

Measured separately on a Pixel against the rig: the link carries about
50 kB/s, player to phone, in 512-byte packets. One phone, one position.

## 4. The feed is a gRPC stream

```proto
// A stream of what the stage looks like: the layout first, then a frame
// whenever something changes, and the layout again if the venue does.
rpc WatchStage(WatchStageRequest) returns (stream StageUpdate);

message WatchStageRequest {
  // The most frames a second the caller wants; 0 is the player's own rate.
  uint32 max_rate = 1;
}

message StageUpdate {
  oneof update {
    StageLayout layout = 1;
    StageFrame frame = 2;
  }
}
```

`StageLayout` is what the websocket's `metadata` message carries: per fixture
its name, type, tags, position, rotation, beam angle, rig asset, the names of
its channels, in order, and where it points when the layout is sent; and the
venue's name, focus points and scenery.

`StageFrame` is what the `state` message carries, without the names: for each
fixture, in the layout's order, its channel values as bytes in the layout's
channel order; strobe rates; a pose for each fixture that points somewhere
other than the layout says; per-cell values while a per-cell effect runs; the
active effects and fixtures under test.

**Every frame stands alone.** A frame is read against the layout and nothing
else, never against the frame before it, because a frame may be dropped (§5).
That is why poses are sent as a difference from the layout, not from the last
frame: a fixture that never moves costs nothing, and losing a frame loses
nothing a later one does not say again.
Each layout has an id and each frame names its layout's; a client drops a
frame for a layout it does not hold.

Measured on the implementation (#718), with nothing moving: 44 bytes for eight
fixtures of five channels, against 827 of JSON; 405 bytes for 50 fixtures of
eight channels, against 7 kB. Each mover pointing away from where the layout
has it adds 43 bytes, so eight movers mid-sweep is about 390 bytes, and a
large rig with moving heads takes more than one packet a frame. At ten frames
a second that is still a few kB/s.

Why a stream and not a format of its own:

- The data is defined once, for every client. A gRPC client on the network
  gets the same feed.
- The web UI already has generated code for the player's messages, so the page
  decodes the frames itself. The app passes bytes from Bluetooth into the web
  view and never learns what a fixture is.
- It goes through the same dispatch, allowlist and lock policy as every call.
  Opening the stream subscribes; cancelling it unsubscribes.

It does not help with bandwidth, and bandwidth is not short: this needs a few
kB/s of the 50 measured.

**Compression.** Frames are not compressed: they are small and mostly bytes
that change, where gzip's own header costs more than it saves. Leaving out
what the layout already says (poses) and what is not running (cells) does
more, and the messages do that. The layout
and the 3D assets (§6) are text and models, compress well, and are sent
gzipped.

**The websocket.** The web UI's stage view keeps reading the websocket's JSON
for now. These messages are designed so it can read the stream instead, and
then the JSON `metadata` and `state` messages are the ones to retire. Until
then there are two encodings of one thing, and that is a debt this doc names
rather than hides.

## 5. Carrying a stream over Bluetooth

Today a request has one answer. A stream needs three things added to the
envelope, and a channel of its own.

- **Envelope.** `Response` gains `more` (frames follow) and the last one
  carries the status. `Request` gains a cancel for a stream's id. An old app
  never asks for a stream, so nothing changes for it.
- **Its own characteristic.** Stream frames are notified on a fourth
  characteristic, `stream`. On the `response` characteristic a frame could sit
  in front of a Stop's answer.
- **Newest wins.** A gRPC stream is ordered and loses nothing; a lighting
  frame that is late is worthless. The player keeps one pending frame per
  stream: while it waits its turn, a newer frame replaces it. This is safe
  because every frame stands alone (§4). A layout is never dropped.
- **One packet at a time.** The stream pump sends one packet, then lets
  anything waiting on `response` or `status` go first.
- **A fixed pace, because the player cannot see the link.** The first draft
  assumed the player would feel a slow phone. It does not: BlueZ takes each
  packet at once and queues it itself, and says nothing back. So the pump
  sends at a fixed 16 kB/s, under what the Pixel's link carried (§3), and
  frames coalesce in the player while the pace holds it. On a link slower than
  the stream, a backlog builds inside BlueZ where the player can neither see
  nor shorten it: frames arrive late, and so do answers to calls. If that
  shows on real links, the remedy is the phone acknowledging frames; it is not
  built. The pace is a guess until it is measured (§11).
- **Rate.** The app asks for 10 frames a second; the player samples at 20.
- **Limits.** Two streams a phone; a cancel by the id the stream was opened
  with.

The dispatcher calls the gRPC service in-process and collects the whole body
today. For a stream it reads the body message by message; the service side is
ordinary tonic server streaming.

## 6. 3D

After the plot. It needs the files the 3D view fetches: the rig description,
a model per fixture type, and scenery if the venue has any.

```proto
rpc GetLightingAsset(GetLightingAssetRequest) returns (stream AssetChunk);
```

The player already stores these under a content hash, which is in the path.
The app caches by that path and fetches only what it lacks; the page's own
requests for `/api/lighting/assets/...` are answered from the cache. Models
are small (22 kB on the rig); scenery may not be, so the app shows progress
and the plot works meanwhile.

## 7. The app

- **Lights**, a fifth tab. The ambient control moves here from the settings
  sheet.
- The tab is a web view. The app serves the bundled page to it from a server
  on the phone's own loopback address, which is what lets the page load its
  modules and WebAssembly, and works the same way on iOS.
- The stream is open only while the tab is showing.
- The stage card loses what means nothing here: the Edit link and the expand
  button. The plot fills the width.
- Against an older player the tab says so, as the settings sheet does.
- The app talks to the page through `window.mtrackStage` and hears from it on
  a JavaScript channel named `MtrackHost` (§8.1). It passes each `StageUpdate`
  of the stream through as it arrived, and calls `reset()` when the stream
  ends or the player changes.

## 8. What changes in the web UI

- A stage-only entry point and build, beside the normal one.
- The websocket starts when asked to, not when a file is imported.
- The stage stores take a layout and frames from either source.
- The stage view gets a mode with no editing chrome.
- The 3D view keeps its scene when a layout arrives that matches the one it
  was built from.
- The stage build should not carry the config API or the rules parser. The
  WebAssembly module wants splitting for that; until then it ships whole.

The app's build then depends on the web UI's. App CI builds the stage bundle
first, and a web UI change runs the app's checks.

As built (L3, web UI side):

- `make build-stage-ui` builds `stage.html` and `src/stage-main.ts` into
  `src/webui/svelte/dist-stage/` (`vite.stage.config.ts`). The page is
  `dist-stage/stage.html`, with relative paths to its files in `assets/`.
  The frontend checks in CI build it, so it cannot rot.
- `lib/stage/feed.ts` reads a layout as the `metadata` message and a frame as
  the `state` message; `applyStageMetadata` and `applyStageState` in
  `lib/ws/stores.ts` are the two writes both sources end in. It is tested
  against a stream the player's encoder wrote
  (`e2e/fixtures/stage-stream.json`, held to the encoder by a test in
  `src/controller/grpc/stage.rs`).
- The websocket opens when `main.ts` calls `connect()`.
- `StageView` has `embedded` (no Edit link, no maximize, nothing dragged or
  kept, the card to the edges) and `threeD` (whether it offers 3D).
- 3D is off in the stage build, and its code is left out of it. A build with
  `MTRACK_STAGE_3D=1` has the switch and the code: that is L4's to turn on,
  once the app answers `/api/lighting/assets/`.
- The 3D view rebuilds only for metadata that differs from what its scene was
  built from. The comparison is in the view, not the stores: two pages of the
  web UI re-read from the server whenever metadata arrives, changed or not.
- Size, gzipped: about 80 kB of script, 27 kB of strings (the web UI's whole
  English table, of which the page uses a handful), 6 kB of styles and the
  462 kB rules module, which still ships whole.

### 8.1 The host contract

What the app is written against. The page's side is the comment at the top of
`src/webui/svelte/src/stage-main.ts`, and the two are kept the same.

```ts
window.mtrackStage = {
  /** True once modules and wasm are loaded and the page can take updates. */
  ready: boolean,
  /** One serialized player.v1.StageUpdate, base64 (standard alphabet,
   *  padded). Updates given before `ready` are queued and applied in order. */
  update(base64: string): void,
  /** Forget the layout and all state: the stream ended or the player
   *  changed. Shows the empty state. Drops anything still queued. */
  reset(): void,
  /** 'light' | 'dark' | 'system'; anything else is 'system'. */
  setTheme(theme: string): void,
  /** Counters for the host's diagnostics: updates applied, frames dropped
   *  for a wrong layout id, decode errors, rendered frames a second. */
  stats(): { applied: number, dropped: number, errors: number, fps: number },
};
```

- The page tells the host things by
  `window.MtrackHost?.postMessage(JSON.stringify({type, ...}))`:
  `{type: 'ready'}` once, and `{type: 'error', message}` for an update that
  does not decode and for uncaught errors and rejections. With no
  `MtrackHost` it works the same.
- `window.mtrackStage` exists once the page's script has run, which is before
  `ready`. The host waits for the page to load, or for `ready`, before its
  first call.
- A frame is read against the layout most recently given. A frame of any
  other layout, or given when the page holds none, is dropped and counted.
- An update that does not decode is counted, reported, and changes nothing
  on screen. `reset()` keeps the counters.
- The page starts as `system`, which follows `prefers-color-scheme`. For the
  right theme at first paint the host can load `stage.html?theme=dark` (or
  `light`). The page's background is the web UI's card colour, which is the
  app's `surface` in both themes (`#FFFFFF`, `#231F20`).
- With no layout, or a layout with no venue fixtures, or after `reset()`, the
  page says "No lights to show."
- The page asks the network for nothing but its own files: no fonts, no API,
  no websocket.

On a desktop, with no phone: `npm run dev` in `src/webui/svelte`, then
`/stage.html?dev=replay` plays the recorded stream and `/stage.html?dev=grpc`
plays a running player's `WatchStage` over gRPC-Web through the dev server's
proxy (`VITE_PROXY_TARGET`). Neither is in the built page.

## 9. If the web view does not work on a phone

Then there is no reuse, and the choice is a second, native plot or no light
view. That goes back to Mike; it is not decided here. The stream (§4, §5) is
the same either way.

## 10. Phasing

| Phase | What |
|---|---|
| L0 | The spike on a Pixel: plot and 3D in the app's web view, from the recording |
| L1 | `WatchStage` on gRPC, with tests against the websocket's messages |
| L2 | Streams over Bluetooth: envelope, `stream` characteristic, newest wins |
| L3 | The stage-only build, and the Lights tab with the plot |
| L4 | `GetLightingAsset`, the cache, and 3D |

## 11. Not verified

- Anything on iOS, where the link is assumed to carry a third of what the
  Pixel's does.
- Movers, per-cell fixtures and scenery: the rig has none.
- Streams over Bluetooth on hardware: the 16 kB/s pace, how late a Stop's
  answer is while a layout streams, and what a weak link does.
- Layouts are not yet gzipped (§4); one over 64 KiB ends the stream.
- The frame sizes in §4 are measured on made-up fixtures, not on the rig.

## 12. Decisions

1. **The feed is a gRPC stream** (§4). Decided.
2. **One visualization, reused in a web view** (§2). Decided; L0 passed.
3. **Plot with movers first, 3D after** (§10). Decided.
4. **Retire the websocket's JSON stage messages later, not now** (§4).
   Decided.
5. **A fifth tab, and ambient moves to it** (§7). Decided.
