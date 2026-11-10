# mtrack phone app

A Flutter app (Android and iOS) that controls a running mtrack player over Bluetooth LE. The
protocol it speaks is documented in [`docs/src/interfaces/bluetooth.md`](../docs/src/interfaces/bluetooth.md)
and the design in [`design/bluetooth_remote_design.md`](../design/bluetooth_remote_design.md).

The app is under development. The protocol layer is in place and tested; the screens are not.

## Layout

- `lib/src/protocol/` is pure Dart with no Flutter or plugin imports: the chunk framing
  (`frame_codec.dart`, a port of `src/controller/ble/codec.rs`), the `PlayerClient` that numbers,
  frames and matches calls, the playback clock, and `PlayerTransport`, the seam to the radio.
- `lib/src/ble/ble_transport.dart` is the only file that touches the Bluetooth plugin. It cannot be
  tested without a phone, so it is kept to moving chunks and reporting link state.
- `lib/gen/` is the protobuf messages, generated and not committed.
- `test/` holds the unit tests. `test/support/fake_player.dart` is an in-memory player that the
  client tests run against. The frame tests read the golden vectors in
  `../src/proto/ble/v1/testdata/frames.json`, which the Rust codec's tests read too.

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

## Building

Building needs the Android SDK (platform and build tools, with its licences accepted) for an APK,
or Xcode on a Mac for iOS. Neither is needed for the commands above.

```sh
flutter build apk --debug
flutter run                # on a connected phone
```

Bluetooth does not work in the Android emulator or the iOS simulator, so the radio path needs a
real phone.

## Identifiers

The application id `dev.mdwn.mtrack_remote` is a placeholder. Flutter has no single place for it;
change it in all of these:

- `android/app/build.gradle.kts`: `namespace` and `applicationId`.
- `android/app/src/main/kotlin/.../MainActivity.kt`: the `package` line, and the directory it is in.
- `ios/Runner.xcodeproj/project.pbxproj`: every `PRODUCT_BUNDLE_IDENTIFIER` (iOS cannot have an
  underscore, so it is `dev.mdwn.mtrackRemote`, and `.RunnerTests` beside it).

The display name, "mtrack", is `android:label` in `AndroidManifest.xml` and `CFBundleDisplayName`
in `ios/Runner/Info.plist`.

## The Bluetooth plugin

The app uses [`universal_ble`](https://pub.dev/packages/universal_ble) (BSD-3-Clause); its Linux
desktop backend pulls in `bluez` and `dbus` (MPL-2.0), which the app does not use but which ship
in its licence notices. `PlayerTransport` is the only thing the rest of the app depends on, so the
plugin can be replaced by rewriting `ble_transport.dart`.

## Bluetooth permissions

Android declares `BLUETOOTH_SCAN` (with `neverForLocation`) and `BLUETOOTH_CONNECT`, which Android
12 and newer ask for at run time, and the legacy Bluetooth and location permissions limited to
Android 11 and older (`maxSdkVersion="30"`). iOS declares `NSBluetoothAlwaysUsageDescription`.
`universal_ble` asks for the permissions itself when a scan starts. The screens that explain them
are not written yet.
