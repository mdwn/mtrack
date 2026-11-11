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

message GetLightingAssetRequest {
  string path = 1;  // under /api/lighting/assets/, as the layout gives it
  bool gzip = 2;    // send the file gzipped; the chunks join into a gzip stream
}

message AssetChunk {
  bytes data = 1;
  uint64 total_size = 2;  // first chunk only: the bytes all chunks carry
}
```

As built: the method reads the file the way the HTTP endpoint does (the
same function: extension allowlist, 64 MiB cap, containment in the store),
so a path that is not in the store, not a served type, or over the cap is
`NOT_FOUND`, and one that escapes the store is `INVALID_ARGUMENT`. With
`gzip` the whole file is compressed first and then cut, so `total_size` is
the size sent. Chunks hold at most 8 KiB, which gives a phone at 16 kB/s a
step every half second and sits far under the 64 KiB message cap. Open
while locked. Over Bluetooth it is a stream no message of which is
replaceable: each chunk waits in the slot until the one before it is on
the radio, so the file is not queued ahead of the phone. The pump takes
turns between streams a message at a time, so a `WatchStage` frame goes
out after at most one chunk (half a second) and not after the file. A phone
has two streams, so it fetches one asset at a time beside `WatchStage`.

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
- The stream is open only while the tab is showing, the app is on screen and
  the link is up.
- The stage card loses what means nothing here: the Edit link and the expand
  button. The plot fills the width.
- Against an older player the tab says so, as the settings sheet does.
- The app talks to the page through `window.mtrackStage` and hears from it on
  a JavaScript channel named `MtrackHost` (§8.1). It passes each `StageUpdate`
  of the stream through as it arrived, and calls `reset()` before the first
  message of every stream it opens. (The first draft said "when the stream
  ends": the session announces a stream's start, not its end, and resetting
  then is always right. While the link is down the last picture stays,
  dimmed, under the "Reconnecting…" banner, as every other tab's last state
  does.)

As built (L3, app side):

- **The page ships in the app.** `make app-stage-assets` builds the stage
  page (§8) and copies it to `app/assets/stage/`, which is not committed.
  The app's tests do not need it; CI's Android build makes it first and
  checks the APK holds it. It adds about 0.66 MB to the APK (1.67 MB of
  files, most of it the rules module).
- **The loopback server** (`app/lib/src/app/stage_server.dart`): `dart:io`,
  bound to 127.0.0.1 on a port the system picks, serving only the bundle's
  `.html`, `.js`, `.css`, `.wasm` and `.json` files, with the content types
  modules and streaming WebAssembly need. Every path starts with a random
  name made when the server starts, and a `Host` that is not the server's
  own loopback name is refused: another app on the phone can reach a
  loopback port, and although what is served is public, the page is not at
  a guessable address and a web page elsewhere cannot reach it through DNS.
  It starts when the tab is first opened and stops when the tabs go (the
  player forgotten, or the link lost for good). On coming back to the tab,
  or to the app, the server is asked for the page; if it does not answer it
  is started again at a new address and the page reloaded.
- **Permissions.** Android gives an app no sockets, loopback included,
  without `INTERNET`, so the app now declares it, with a network security
  config that allows cleartext to 127.0.0.1 and nowhere else. iOS gets the
  App Transport Security exception for local networking and nothing more.
  The app still talks to nothing but the player. `webview_flutter` has no
  way to answer a web view's requests from the app's assets without a
  socket (Android's `WebViewAssetLoader`, iOS's `WKURLSchemeHandler`);
  loading the files straight from the bundle gives the page a `file:`
  origin, where module scripts and streaming WebAssembly are refused. A
  plugin or native code that exposes those two would remove the server and
  the permission; it is not built.
- **The pusher** (`app/lib/src/app/stage_pusher.dart`) sits between the
  stream and the page. Nothing is sent before the page's `ready`. One
  JavaScript call is out at a time, and everything that waits when it
  returns goes in the next one: resets and layouts in order, none dropped,
  then the newest frame. A frame that waits is dropped when a layout or a
  reset arrives behind it, and a reset empties what waited ahead of it. A
  layout is told from a frame by its first byte, as the player does; the
  app decodes nothing. A page that loads again gets the stream started
  over, so it is sent a layout.
- **States.** Until the page is ready and holds a layout of the stream being
  listened to, the tab says "Waiting for the lights…" over the page, so
  the page's own "No lights to show." is only ever seen for a layout with
  no fixtures. A page that does not load, does not say `ready` within 20
  seconds, or reports an error before it is ready, is replaced by "Could
  not show the lights." and a "Try again" that loads it again. Once the
  page runs, one `error` from it is logged and no more (by §8.1 an update
  that does not decode changes nothing on screen); three within ten
  seconds are taken for a broken page.
- **A picture, not a control.** No touch reaches the page: the plot has
  nothing to tap in this mode, and a web view that took gestures would
  scroll, zoom and select text. A screen reader is told "Stage lights", an
  image. 3D (L4) turns under a finger, and will need touches let through.
  (L4 let them through: see below.)
- **The web view is behind an interface** (`StageSurface`, `StageHost` in
  `app/lib/src/ui/stage_surface.dart`), with the real one in
  `app/lib/src/platform/webview_stage.dart`, the only part no test runs.

As built (L4, app side):

- **Fetching.** `PlayerClient.getLightingAsset` opens `GetLightingAsset`
  with `gzip` set and returns an `AssetDownload`
  (`app/lib/src/protocol/asset_download.dart`): the chunks joined as they
  arrive, with progress, and unpacked at the end. Every chunk is taken the
  moment it arrives, so the stream holds nothing for the listener and the
  256 kB it would hold for a paused one is no limit on an asset. 64 MiB is
  the most taken, as sent and once unpacked (the player's own cap); a total
  over it is refused at the first chunk and the stream cancelled. Gzip is
  unpacked a piece at a time and stopped at the cap, and its trailing length
  is checked, since `dart:io` takes gzip that was cut short without a word.
  An asset that arrives short of its total, or past it, is `DATA_LOSS`.
- **One at a time.** `LightingAssets` (`app/lib/src/app/lighting_assets.dart`,
  on the session) fetches in the order asked, and the next only once the
  player has let go of the stream before, since the stage holds one of the
  two a phone may have. Two asks for one path share a fetch. A download
  that goes 30 seconds without a chunk is given up. Losing the link cancels
  the one under way and fails those waiting.
- **The cache** (`app/lib/src/app/asset_cache.dart`) is a directory of the
  app's cache directory (`path_provider`, a new dependency), one file per
  asset, named for its path with `/` written `%2F`. It is the app's, not a
  player's: the path holds the content's hash. A file is written under a
  temporary name and renamed, so a read never finds half of one. It is kept
  under 128 MiB by deleting the least recently used after each write (a
  read touches the file); a leftover temporary file over an hour old goes
  too. Anything that fails there is a miss.
- **Paths.** Only a path the player could have made is fetched, cached or
  used as a file name: names of letters, digits, `.`, `_`, `-` and inner
  spaces, none starting with a dot, no empty name, at most 200 characters,
  and a `json`, `glb`, `png` or `svg` file. A mesh named with other
  characters (GDTF does not forbid them) is not drawn.
- **Serving.** The page asks for `api/lighting/assets/<path>` relative to
  itself (§8), so under the server's random prefix; the server checks the
  path, then answers from the cache or waits for the fetch: the player's
  `NOT_FOUND` is a 404, anything else that keeps the asset from the phone
  (no link, a lost link, too old) a 503. The type is the player's for the
  kind, with `nosniff`. The prefix and the `Host` check stand as before.
- **Touches reach the page.** The web view takes every touch on it at once
  (an eager gesture recognizer). Nothing around it in the tab scrolls, and
  the ambient card is above it, not under it, so nothing is fought over;
  the covers over the page ("Waiting…", the dimmed picture) take the
  touches they cover. In the plot the page has nothing to do with a touch
  (the embedded card drags nothing), so a drag scrolls the page if it is
  taller than the view; the page selects no text and shows no menu on a
  long press. A screen reader is told "Stage lights" as a container, so the
  Plot | 3D switch inside it can be reached; how the web view reads to one
  is unverified.
- **Progress and failure** are the app's, over the bottom of the 3D view,
  since the app is what serves the files: while any are loading, "Loading
  the 3D models…" with "n of m" and a bar; after a failure "Some 3D models
  did not load, so they are drawn as boxes." (the 3D view draws a fixture
  with no model as a box), or for `NOT_FOUND` only "The player does not
  have some 3D models…", each with **Try again**, which calls the page's
  `retry3d()`. A new stream (the page rebuilds its 3D view for it) and a
  page load forget what failed. Over the plot nothing is said, and the plot
  runs throughout.
- **An older player.** Against a player with the stage stream but not
  `GetLightingAsset` (it refuses the method as one it does not allow), the
  switch stays: what the cache already holds still draws, since it is the
  app's and not the player's. The tab says over 3D "This player runs an
  older mtrack, so it cannot send the 3D models. Update mtrack on the player
  to see them.", the settings sheet names "the 3D light view" among what
  the player is too old for, and nothing more is asked of that player until
  the link is made again.
- **The view is remembered** across launches (`shared_preferences`,
  `lights.view`): the page is loaded with `?view=3d` when 3D was showing,
  and a page that loads again keeps the view it had.
- **Size.** 3D adds three.js (639 kB, 160 kB gzipped), the scene (17 kB)
  and the view (8 kB) to the bundle: about 170 kB gzipped, so the page is
  about 285 kB gzipped besides the 462 kB rules module, near §3's 260 kB.
  In the APK the page now takes 0.86 MB (2.33 MB of files), 0.20 MB more
  than the plot alone.

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
  once the app answers `/api/lighting/assets/`. (L4 turned it on for the
  app: below.)
- The 3D view rebuilds only for metadata that differs from what its scene was
  built from. The comparison is in the view, not the stores: two pages of the
  web UI re-read from the server whenever metadata arrives, changed or not.
- Size, gzipped: about 80 kB of script, 27 kB of strings (the web UI's whole
  English table, of which the page uses a handful), 6 kB of styles and the
  462 kB rules module, which still ships whole.

As built (L4, web UI side):

- `make app-stage-assets` builds the page with `MTRACK_STAGE_3D=1`; the
  build stamp tells the two builds apart. `make build-stage-ui` and the
  frontend checks still build it without.
- The asset store's address is the 3D scene's `ASSETS`, now
  `import.meta.env.VITE_STAGE_ASSET_BASE`, or `/api/lighting/assets/` when
  that is not set. Only `vite.stage.config.ts` sets it, to
  `api/lighting/assets/`, relative, so the page asks the host under
  whatever path it was served from. The web UI's own build is as it was.
- The show preview is left out of the stage build with or without 3D:
  only the editing card opens it.
- A card that is not editable takes `view` as the view it starts in, and
  tells `onViewChange` when the person switches. The dashboard passes
  neither and is as it was.
- `StageOnly` builds its card again on `retry3d()`, in the view last
  shown, so the 3D view starts from nothing and asks again for every file.

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
  /** Build the 3D view again from nothing, asking again for every file it
   *  could not have. Nothing happens in the plot or a page without 3D. */
  retry3d(): void,
};
```

- The page tells the host things by
  `window.MtrackHost?.postMessage(JSON.stringify({type, ...}))`:
  `{type: 'ready'}` once, and `{type: 'error', message}` for an update that
  does not decode and for uncaught errors and rejections. A page with 3D
  also posts `{type: 'view', view}`, `'plot'` or `'3d'`: the view it starts
  in, right after `ready`, and again whenever the person switches. With no
  `MtrackHost` it works the same.
- A page with 3D starts in the plot, or in 3D when loaded with `?view=3d`.
  Its 3D view asks the host for the player's lighting assets at
  `api/lighting/assets/<path>`, relative to the page, and draws a fixture
  whose model it could not have as a box. It takes touches: a drag turns
  it.
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
- The page asks the network for nothing but its own files and, in 3D, the
  lighting assets beside them: no fonts, no API, no websocket.

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
| L4 | `GetLightingAsset`, the cache, and 3D (the app's side built; the player's in its own branch) |

## 11. Not verified

- Anything on iOS, where the link is assumed to carry a third of what the
  Pixel's does.
- Movers, per-cell fixtures and scenery: the rig has none.
- Streams over Bluetooth on hardware: the 16 kB/s pace, how late a Stop's
  answer is while a layout streams, and what a weak link does.
- Layouts are not yet gzipped (§4); one over 64 KiB ends the stream.
- The frame sizes in §4 are measured on made-up fixtures, not on the rig.
- The stage-only build (§8) in a phone's web view: it has run in desktop
  Chromium only, from the dev server and from its built files. The spike that
  ran on a Pixel was fed JSON; this one decodes protobuf in the page.
- 3D on a phone through the app (L4): the web view asking the app's server
  for assets under the prefix, three.js from the bundle, a drag turning the
  view in the web view rather than scrolling it, the progress note against
  a real link, and `GetLightingAsset` against a real player (the app was
  written against the proto; the player's side was built at the same time).
  The built 3D page has run only in headless desktop Chromium, served under
  a prefix by a stand-in server that answered no asset (every fixture a
  box): it asked for its assets under the prefix, posted its views, and
  asked again on `retry3d()`.
- The Lights tab (§7) on a phone, all of it: the web view loading the page
  from the app's own server, the page's `ready` reaching the app, the plot
  drawing from the stream, and whether the server outlives the app's time in
  the background on either platform. It is tested against a fake web view.

## 12. Decisions

1. **The feed is a gRPC stream** (§4). Decided.
2. **One visualization, reused in a web view** (§2). Decided; L0 passed.
3. **Plot with movers first, 3D after** (§10). Decided.
4. **Retire the websocket's JSON stage messages later, not now** (§4).
   Decided.
5. **A fifth tab, and ambient moves to it** (§7). Decided.
