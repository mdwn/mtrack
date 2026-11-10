# mtrack phone app

A Flutter app (Android and iOS) that controls a running mtrack player over Bluetooth LE. The
protocol it speaks is documented in [`docs/src/interfaces/bluetooth.md`](../docs/src/interfaces/bluetooth.md)
and the design in [`design/bluetooth_remote_design.md`](../design/bluetooth_remote_design.md).

The app is not released: it is not in any store. It has three tabs: **Now** (the song, its time and
the transport), **Playlist** (the songs, and a way to switch playlist) and **Sections** (tap a section
to jump to it, hold to loop it), with a connect screen for the first run and a small settings sheet.
It follows the phone's light or dark setting.

## Layout

- `lib/src/protocol/` is pure Dart with no Flutter or plugin imports: the chunk framing
  (`frame_codec.dart`, a port of `src/controller/ble/codec.rs`), the `PlayerClient` that numbers,
  frames and matches calls, the playback clock, and `PlayerTransport`, the seam to the radio.
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
- `lib/src/ble/` (`ble_transport.dart`, `ble_link.dart`) is the only code that touches the Bluetooth
  plugin. `lib/src/platform/` holds the other two plugin adapters (`shared_preferences`,
  `wakelock_plus`). None of these can be tested without a phone, so each is kept to translating calls.
- `lib/src/ui/` is widgets only: the theme (`theme.dart`), the screens, and the press-and-hold Stop
  button (`hold_to_stop.dart`). No protocol logic.
- `lib/main.dart` wires the real radio, store and wake lock.
- `lib/gen/` is the protobuf messages, generated and not committed.
- `assets/fonts/` holds Nunito, Inter and JetBrains Mono (variable fonts, SIL Open Font License; each
  `OFL.txt` is beside its font and shown on the app's licence page). They are bundled so the app needs
  no network.
- `test/` holds the tests. `test/support/fake_player.dart` is an in-memory player behind
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
the section arithmetic, and each screen in each state in both themes, at normal and 1.3x text, on a
360x640 phone, where a layout overflow fails the test. There are no golden-image tests: font
rendering differs between machines.

## Building and installing

Building needs the Android SDK (platform and build tools, with its licences accepted) and a JDK 17
for an APK, or Xcode on a Mac for iOS. Neither is needed for the commands above.

```sh
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
- A dropped link being noticed, and reconnecting after it, including a phone that leaves and returns
  to range, and the player restarting.
- The remembered player surviving a launch, and the phone's per-app identifier for the player (a
  MAC address on Android, a per-phone UUID on iOS) staying the same.
- Keeping the screen on while connected (`wakelock_plus`), and releasing it in the background.
- Haptics on Play and Stop, and the feel of the 600 ms hold under a real finger.
- The bundled variable fonts' weights on a real device (the tests load the files but cannot show
  that the weight axis is applied), and how the screens look: they were checked in a rendered test
  at 360x640, not on a phone.
- TalkBack and VoiceOver reading the screens and the Stop button's long-press action.

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
(the remembered player), `wakelock_plus` (keeping the screen on) and `package_info_plus` (the version
on the settings sheet, and already a dependency of `wakelock_plus`), all BSD-3-Clause. There is no
state-management package: `Session` is a plain `ChangeNotifier`.

## Bluetooth permissions

Android declares `BLUETOOTH_SCAN` (with `neverForLocation`) and `BLUETOOTH_CONNECT`, which Android
12 and newer ask for at run time, and the legacy Bluetooth and location permissions limited to
Android 11 and older (`maxSdkVersion="30"`). iOS declares `NSBluetoothAlwaysUsageDescription`.
`universal_ble` asks for the permissions itself when a scan starts, and the connect screen has a
button that asks again after a refusal.
