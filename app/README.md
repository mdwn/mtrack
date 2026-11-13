# mtrack phone app

A Flutter app (Android and iOS) that controls a running mtrack player over Bluetooth LE, or over
Wi-Fi when the player answers there (see [Wi-Fi](#wi-fi)). The protocol it speaks is documented in [`docs/src/interfaces/bluetooth.md`](../docs/src/interfaces/bluetooth.md)
and the design in [`design/bluetooth_remote_design.md`](../design/bluetooth_remote_design.md).

The app is not released: it is not in any store. It has five tabs: **Now** (the song, its time and
the transport), **Library** (a **Playlist | Songs** switch at the top: the playlist, with a way to
switch playlist and an editor for the playlists: songs and their order, each song's fades, auto
play and ambient light, and the playlist's ambient light; or **Songs**, the song library, below),
**Sections** (tap a section
to jump to it, hold to loop it), **Levels** (a fader and a mute for each output track) and **Lights**
(below), with a connect screen for the first run and a small settings sheet. It follows the phone's
light or dark setting.

## Wi-Fi

The app prefers the network when the player answers on it, and falls back to Bluetooth when it
does not, without the person choosing. The seam is one level up from the radio:
`lib/src/protocol/player_channel.dart` (`PlayerChannel`) is "call this method with these request
bytes, get response bytes" and "open this server stream", plus the status and the state of the link.
`EnvelopeChannel` implements it over Bluetooth with the envelope (framing, chunking, request ids,
the two-stream limit and the cancel wait), and `GrpcChannel` implements it with real gRPC calls on
the player's gRPC port: plaintext HTTP/2, no generated stubs (a method is a path and its bytes), a
two second connect timeout, and a heartbeat (`GetNetworkInfo` every 8 s, 5 s to answer) because
a link that vanishes is not known to TCP for minutes, and the `grpc` package's own keepalive does
not end a connection that has a stream open. `PlayerClient` is the typed face: it knows
`PlayerService`'s methods and nothing of which carrier is under it, and errors reach the screens as
the same `PlayerException` with the same gRPC codes either way.

- Status over Wi-Fi is `WatchStatus` (`RemoteStatus`, the message Bluetooth pushes); nothing is polled.
- Where the player is: when connected by Bluetooth the app calls `GetNetworkInfo` and remembers the
  addresses, the gRPC port and the host name with the player (`RememberedPlayer`, in the
  preferences next to what Bluetooth needs); or **Enter an address** on the connect screen takes a
  host name or IP with an optional `:port` (`lib/src/app/wifi_address.dart`), and the player it
  makes is Wi-Fi only. No mDNS browsing.
- Connecting tries Wi-Fi first for two seconds (up to three addresses), then Bluetooth. On
  Bluetooth, with addresses known, it looks again every twenty seconds and moves over once the
  player answers and no call is in flight (it waits up to ten seconds for that, then leaves it to
  the next look). A Wi-Fi link that goes is reconnected like any other, Wi-Fi first.
- The wrong player: the host name the player reports on Wi-Fi is compared with the one remembered,
  and a different one is closed and ignored, falling back to Bluetooth (a Wi-Fi-only player keeps
  waiting for its own). A player from before `WatchStatus` / `GetNetworkInfo` answers
  `UNIMPLEMENTED` and simply stays on Bluetooth, with nothing said and no more tries.
- Over Wi-Fi the stage asks for the player's own frame rate, where Bluetooth asks for 10 a second.
- The top bar shows a small Wi-Fi or Bluetooth mark beside the player's name, and the settings sheet
  says "Connected over Wi-Fi" or "Connected over Bluetooth".

Wi-Fi is open to whoever is on the player's network, as the web UI is; Bluetooth keeps its pairing.
See the [Bluetooth Control](../docs/src/interfaces/bluetooth.md#over-wi-fi) page and the design,
[`design/remote_phase3_design.md`](../design/remote_phase3_design.md) §4.

## Songs

The **Songs** side of the Library tab's switch is the song library: every song on the player,
each row saying whether it loops, has lights, MIDI or samples, or failed to load. Songs are
grouped under their folders as the web UI's Songs page groups them: the songs at the top of the
songs directory first, then a heading for each folder, nested ones under their whole path
(`LIVE/2026 TOUR`), with how many songs it holds; tap a heading to close or open it, which the
app keeps until the player is forgotten. A search lists the songs that match, each with its
folder under its name. A player that does not say where its songs are shows one list. The
switch keeps the side last shown for the next launch, and each side keeps its place while the
other shows; it is not there while a playlist is being edited. It reads while the player is
locked. Tapping a song opens its page, which says the song's folder under its name: first what
it has that the phone does not change (tempo map, sections, metronome, pilot hints, MIDI, light
shows), then the settings it does, worded as the web UI's song editor words them:

- **Looping**: Loop Playback, the loop crossfade (a length, checked loosely here and properly by
  the player) and the tracks that switch at its midpoint.
- **Tracks**: each track's name, its file (picked from the audio files in the song's folder,
  with their channel counts) and the file's channel; add and remove.
- **Samples**: the song's own samples, their file, output track or channels, release and
  retrigger behaviour, voice limit and fade time. Velocity is shown and kept as the file has it;
  the triggers are listed and not changed here.

Changes are staged on the page and sent with **Save**, only the groups changed. A song changed
elsewhere meanwhile offers **Save anyway** or **Discard my changes**; a change the player
refuses is shown in its words. While the player is locked the page is read only. A change to
the song that is playing is saved at once and takes effect the next time it plays; turning
looping on or off changes at once whether Play and Next break out of it. A player too old for
the library says so, and the settings sheet lists it.

## The Lights tab

At the top is the ambient light's control (Auto, On, Off, and what the light is doing). Under it,
filling the rest of the tab, is the player's stage, live, as a plot or in 3D: the web UI's own stage
view, running in a web view from a copy of the page that ships inside the app, and fed the player's
stage stream (`WatchStage`) over Bluetooth. The app draws nothing of it and decodes nothing: it hands each
message of the stream to the page. The design is
[`design/bluetooth_light_view_design.md`](../design/bluetooth_light_view_design.md); its §8.1 is the
contract between the app and the page.

- The stream is open only while the Lights tab is the one showing, the app is on screen and the link
  is up. Leaving the tab, backgrounding the app or losing the link cancels it.
- The tab says "Waiting for the lights…" until the page is ready and has a layout; "Could not show
  the lights." with a **Try again** when the page cannot be loaded; and, against a player from
  before the stage stream, that the player is too old to show its lights. While the link is down the
  last picture stays, dimmed, under the "Reconnecting…" banner.
- Touches reach the page: its **Plot | 3D** switch, and the 3D view, which turns under a finger. The
  plot itself takes no touch. The view shown is remembered for the next launch.
- The 3D view draws each fixture from its model, which the app fetches from the player
  (`GetLightingAsset`, one file at a time, gzipped) the first time it is needed and keeps in its
  cache directory (`lib/src/app/asset_cache.dart`, at most 128 MiB, least recently used out first).
  A model's path holds its content's hash, so the cache serves every player and never goes stale.
  While models load the tab says so over the 3D view, with a bar; one that did not load is drawn as a
  box, and the tab says so with a **Try again**. Against a player that cannot send models it says
  the player is too old, and the models already in the cache still draw.
- The page is served to the web view by a small HTTP server inside the app, on the phone's loopback
  address (`lib/src/app/stage_server.dart`): a web view will not load module scripts or streaming
  WebAssembly from a file. It listens on 127.0.0.1 only, on a port the system picks, serves only the
  bundled page's files, puts them under a random path made at each start, and refuses a request
  whose `Host` is not its own; the 3D view's models are answered under the same random path. See
  [Network permission](#network-permission).

### The stage bundle

The page is built from the web UI (`src/webui/svelte`, `make build-stage-ui`) and is not committed.
`make app-stage-assets`, from the repository root, builds it with its 3D view
(`MTRACK_STAGE_3D=1`) and copies it to `assets/stage/`:

```sh
make app-stage-assets     # needs Node, Rust with the wasm32 target, wasm-pack, wasm-bindgen, wasm-opt
```

That is the web UI's toolchain (`./setup.sh` at the repository root installs it). Run it before
`flutter build` or `flutter run`, and again when the web UI changes. `flutter analyze` and
`flutter test` do not need it: no test loads the real page. An app built without it works, and its
Lights tab says "Could not show the lights." The two `.gitignore` files under `assets/stage/` keep
the directories in a fresh checkout, so the asset entries in `pubspec.yaml` are true before the page
is built; Flutter leaves hidden files out of the bundle. The page adds about 0.86 MB to the APK, of
which 0.20 MB is the 3D view (three.js, mostly).

## Layout

- `lib/src/protocol/` is pure Dart with no Flutter or plugin imports: the chunk framing
  (`frame_codec.dart`, a port of `src/controller/ble/codec.rs`), the `PlayerClient` that numbers,
  frames and matches calls and carries server streams (`openStream`: messages on the `stream`
  characteristic, a cancel, an id kept until the player's last response), the playback clock, and
  `PlayerTransport`, the seam to the radio. `asset_download.dart` joins and unpacks a lighting
  asset as `GetLightingAsset` streams it.
- `lib/src/app/` is the application layer, also with no plugin imports, and where every decision is
  made:
  - `Session` (a `ChangeNotifier`) owns the connection and everything the screens show: it finds the
    player, runs the connect sequence (connect, pair at once if not paired, subscribe, read the
    status, fetch the song's sections and the playlist), remembers the player, reconnects with
    backoff (1 s, 2 s, 5 s, then every 5 s), follows the status, and sends the actions, one in flight
    per kind.
  - `PlayerLink` is the seam over the radio for what `PlayerTransport` does not cover: scanning,
    adapter and permission state, connecting, pairing. `PlayerStore` (the remembered player) and
    `ScreenWake` are the other two seams.
  - `sections.dart` turns a song's sections and beat grid into time ranges, the way the player does
    when it seeks to or loops a section.
  - `stage_watch.dart` keeps the player's stage stream (`WatchStage`) open while anything listens
    to `Session.watchStage()`: one stream for every listener, opened again after a reconnect, with a
    reset before each new stream. It hands on the serialized messages and decodes nothing. The
    Lights tab is its listener.
  - `stage_pusher.dart` hands that stream to the stage page, one JavaScript call at a time: nothing
    before the page is ready, resets and layouts in order and never dropped, the newest frame only,
    and the stream started over for a page that loads again.
  - `stage_server.dart` is the loopback server the page is loaded from. It reads the files, and
    the 3D view's lighting assets, through functions it is given, so its tests serve made-up files
    over real sockets.
  - `lighting_assets.dart` fetches the player's lighting assets for the 3D view, one at a time, and
    `asset_cache.dart` keeps them on the phone.
- `lib/src/protocol/` is the protocol without a radio: `player_channel.dart` is the seam,
  `envelope_channel.dart` and `grpc_channel.dart` are its two carriers, `player_client.dart` the typed
  calls, and `frame_codec.dart`, `asset_download.dart` and `playback_clock.dart` what they share.
  `lib/src/platform/grpc_wifi_link.dart` opens `GrpcChannel`s for the session.
- `lib/src/ble/` (`ble_transport.dart`, `ble_link.dart`) is the only code that touches the Bluetooth
  plugin. `lib/src/platform/` holds the other plugin adapters: `shared_preferences`, `wakelock_plus`,
  and `webview_stage.dart`, the web view the Lights tab shows the stage page in (`webview_flutter`)
  with the reading of the bundled page from the app's assets. None of these can be tested without a
  phone, so each is kept to translating calls.
- `lib/src/ui/` is widgets only: the theme (`theme.dart`), the screens, and the press-and-hold Stop
  button (`hold_to_stop.dart`). No protocol logic. `stage_surface.dart` is the seam the Lights tab
  (`lights_screen.dart`) sees the web view through: `StageSurface` (load, run JavaScript, the page's
  messages, errors, a widget) and `StageHost` (where the page is, and a new surface).
- `lib/main.dart` wires the real radio, store, wake lock and web view, and puts the lighting asset
  cache in the app's cache directory (`path_provider`).
- `lib/gen/` is the protobuf messages, generated and not committed.
- `assets/fonts/` holds Nunito, Inter and JetBrains Mono (variable fonts, SIL Open Font License; each
  `OFL.txt` is beside its font and shown on the app's licence page). They are bundled so the app needs
  no network.
- `assets/stage/` is where the stage page goes (see [The stage bundle](#the-stage-bundle)).
- `test/` holds the tests. `test/support/fake_stage.dart` is the fake `StageHost` and `StageSurface`
  the widget tests run the Lights tab against. `test/support/fake_player.dart` is an in-memory player behind
  `PlayerTransport`; `fake_link.dart` is the fake `PlayerLink`; `rig.dart` is a player with songs and
  playlists that answers like the real one and sends the status. The frame tests read the golden vectors
  in `../src/proto/ble/v1/testdata/frames.json`, which the Rust codec's tests read too.

## Setup

You need the Flutter SDK (the version in `.github/workflows/app.yaml`, `FLUTTER_VERSION`) and
`protoc` 3.15 or newer on `PATH` (`scripts/install-protoc.sh` installs one).

```sh
make gen-app-proto        # from the repository root: generate lib/gen
cd app
dart format lib/main.dart lib/src test
flutter analyze
flutter test
```

`make gen-app-proto` has to run before `flutter analyze` or `flutter test`, and again whenever
`src/proto/` changes. The Dart plugin for protoc is the `protoc_plugin` dev dependency, so the
generator's version is pinned by `pubspec.lock`.

The tests cover the connect sequence, reconnecting (with fake time), every action and its failures,
the section arithmetic, the stage pusher, the loopback server (over real sockets), fetching and
caching the lighting assets (the cache on a real temporary directory), and each screen in
each state in both themes, at normal and 1.3x text, on a 360x640 phone, where a layout overflow fails
the test. There are no golden-image tests: font
rendering differs between machines.

## Building and installing

Building needs the Android SDK (platform and build tools, with its licences accepted) and a JDK 17
for an APK, or Xcode on a Mac for iOS. Neither is needed for the commands above.

```sh
make app-stage-assets          # from the repository root: the Lights tab's page
cd app
flutter build apk --debug      # app/build/app/outputs/flutter-apk/app-debug.apk
flutter run                    # on a connected phone
```

To put the APK on an Android phone, enable USB debugging and run
`adb install -r build/app/outputs/flutter-apk/app-debug.apk`. Or take it from CI: the "Build the
Android app" job of the app workflow uploads `mtrack-remote-debug-apk` (kept for 7 days); download it
from the run's page, copy it to the phone and open it (Android asks to allow installing from the
source). It is signed with the debug key, so it is for trying, not for a store.

The iOS project is built (`flutter build ios --no-codesign --debug`) on pushes to main only, to check
that it compiles. Running it on an iPhone needs Xcode and an Apple developer account for signing.

Bluetooth does not work in the Android emulator or the iOS simulator, so the radio path needs a
real phone.

## What is not verified without a phone

Everything above `lib/src/ble/` and `lib/src/platform/` is tested against fakes. These have not run
on a device:

- Scanning, filtering on the service, and the signal strength the scan reports.
- Connecting, the MTU request (517 on Android), service discovery and subscribing to notifications.
- Pairing: that `pair` makes the player and the phone bond, that a refused or closed pairing window
  comes back as a failure (which the app shows as "not accepting new phones"), and that a phone
  already paired reports so (`isPaired`). On iOS pairing is started by the system when an encrypted
  characteristic is used, so the app reads `status` to start it.
- The permission prompts, "Bluetooth off" detection, and what the adapter reports while the system
  is still starting.
- The `stream` characteristic: that discovery lists it on both platforms, that subscribing to it
  works, and that a player from before streams (which has none) still connects. A phone that
  bonded before the player had it may remember the service without it: on Android the app then
  discovers once more with the phone's cache dropped (`clearGattCache`, a hidden call), which has
  never run; iOS has no such call and relies on the player telling the phone its services changed.
- Stream messages and call answers interleaving on the air, and what the stage stream does to the
  latency of a tap.
- A dropped link being noticed, and reconnecting after it, including a phone that leaves and returns
  to range, and the player restarting.
- The remembered player surviving a launch, and the phone's per-app identifier for the player (a
  MAC address on Android, a per-phone UUID on iOS) staying the same.
- Wi-Fi, all of it (`lib/src/protocol/grpc_channel.dart` and the session logic around it are tested
  against an in-process gRPC server and a fake network, never a player): that gRPC to the Rust
  player's port works from the phone (HTTP/2 framing, `WatchStatus` and the streams against tonic);
  that `mtrack.local` resolves on Android (the system resolver does not always do mDNS, so a
  typed name may not work where an IP does); that the heartbeat finds a phone leaving range in
  about ten seconds and not falsely on a busy network; the Wi-Fi to Bluetooth fallback and the move
  back, on a real network; iOS asking for local network access and what it does when refused; and
  that binding the process to Wi-Fi keeps Android from sending the app's sockets over mobile data
  when the Wi-Fi network has no internet (see [Network permission](#network-permission)).
- Keeping the screen on while connected (`wakelock_plus`), and releasing it in the background.
- Haptics on Play and Stop, and the feel of the 600 ms hold under a real finger.
- The bundled variable fonts' weights on a real device (the tests load the files but cannot show
  that the weight axis is applied), and how the screens look: they were checked in a rendered test
  at 360x640, not on a phone.
- TalkBack and VoiceOver reading the screens and the Stop button's long-press action.
- The Lights tab's web view, all of it (`lib/src/platform/webview_stage.dart` and everything it
  stands for in the tests): that the web view loads the page from the loopback server on Android
  (cleartext to 127.0.0.1, the `INTERNET` permission in a release build) and on iOS (the local
  networking exception); that the page's modules and WebAssembly load from it and the page says
  `ready`; that the plot draws from the stream, in both themes with no seam or flash around it; that
  a JavaScript call carrying a layout of tens of kilobytes goes through; that touches do not reach
  the page and the tab bar still works over it; that the server survives the app's time in the
  background, or is replaced when it does not; what the web view does when the system kills its
  renderer; and what a screen reader says for the stage. The iOS build in CI does not carry the
  page.

## Identifiers

The application id `dev.mdwn.mtrack_remote` is a placeholder. Flutter has no single place for it;
change it in all of these:

- `android/app/build.gradle.kts`: `namespace` and `applicationId`.
- `android/app/src/main/kotlin/.../MainActivity.kt`: the `package` line, and the directory it is in.
- `ios/Runner.xcodeproj/project.pbxproj`: every `PRODUCT_BUNDLE_IDENTIFIER` (iOS cannot have an
  underscore, so it is `dev.mdwn.mtrackRemote`, and `.RunnerTests` beside it).

The display name, "mtrack", is `android:label` in `AndroidManifest.xml` and `CFBundleDisplayName`
in `ios/Runner/Info.plist`.

## Dependencies

The app uses [`universal_ble`](https://pub.dev/packages/universal_ble) (BSD-3-Clause) for the radio;
its Linux desktop backend pulls in `bluez` and `dbus` (MPL-2.0), which the app does not use but which
ship in its licence notices. `PlayerLink` and `PlayerTransport` are all the rest of the app depends
on, so the plugin can be replaced by rewriting `lib/src/ble/`. The others are `shared_preferences`
(the remembered player), `grpc` (Wi-Fi; Apache-2.0, with `http2` and a few of Google's auth packages it
pulls in, BSD-3-Clause or Apache-2.0), `wakelock_plus` (keeping the screen on), `package_info_plus` (the version
on the settings sheet, and already a dependency of `wakelock_plus`) and `webview_flutter` (the Lights
tab's web view), all BSD-3-Clause. There is no
state-management package: `Session` is a plain `ChangeNotifier`.

## Network permission

The app talks to nothing but the player, over Bluetooth or, since [Wi-Fi](#wi-fi), gRPC on the
player's own port. The network declarations are:

- Android: `android.permission.INTERNET` in `AndroidManifest.xml`. Android gives an app no sockets
  at all without it, loopback included; Wi-Fi gRPC and the Lights tab's loopback server both need it.
  `res/xml/network_security_config.xml` allows cleartext HTTP to `127.0.0.1` and nowhere else; it
  governs the platform's HTTP stacks and the web view, not the Dart sockets the gRPC connection uses,
  so it does not block plaintext gRPC to a LAN address, and the web view stays limited to loopback.
- iOS: `NSAppTransportSecurity` with `NSAllowsLocalNetworking` in `Info.plist` for the stage
  page, and `NSLocalNetworkUsageDescription` ("mtrack uses your local network to control your
  mtrack player over Wi-Fi."), which makes iOS ask once before the app may reach a device on the
  network. Not run on a phone.

The stage page asks the network for nothing but its own files: no fonts, no API, no websocket.

On Android, when the Wi-Fi network has no internet (a player's own network) and the phone has
mobile data, the system makes mobile data the default network and a socket to the player would
leave that way. So `MainActivity` binds the app's process to the Wi-Fi network whenever the phone
has one (`ConnectivityManager.bindProcessToNetwork`, watching with a network callback that does
not ask for internet), and unbinds when it goes. This needs `ACCESS_NETWORK_STATE`, a normal
permission the user is not asked about. Android can refuse the binding (it did on a Pixel with a
VPN up that apps may not bypass, the likely reason); the app logs that and its sockets go where
the system sends them, as they did before, and on that Pixel Wi-Fi worked as before. Not yet run
with the binding in force, on a network with or without internet, so that the stage page on
`127.0.0.1` still loads under it is expected, not seen.

## Bluetooth permissions

Android declares `BLUETOOTH_SCAN` (with `neverForLocation`) and `BLUETOOTH_CONNECT`, which Android
12 and newer ask for at run time, and the legacy Bluetooth and location permissions limited to
Android 11 and older (`maxSdkVersion="30"`). iOS declares `NSBluetoothAlwaysUsageDescription`.
`universal_ble` asks for the permissions itself when a scan starts, and the connect screen has a
button that asks again after a refusal.
