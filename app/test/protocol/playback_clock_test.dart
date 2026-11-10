import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/ble/v1/ble.pb.dart' as ble;
import 'package:mtrack_remote/src/protocol/playback_clock.dart';
import 'package:protobuf/well_known_types/google/protobuf/duration.pb.dart'
    as pb;

ble.Status status({
  required bool playing,
  required int elapsedMs,
  int durationMs = 200000,
}) => ble.Status(
  songName: 'Song',
  playing: playing,
  elapsed: pb.Duration.fromDart(Duration(milliseconds: elapsedMs)),
  songDuration: pb.Duration.fromDart(Duration(milliseconds: durationMs)),
);

void main() {
  late Duration now;
  late PlaybackClock clock;

  setUp(() {
    now = const Duration(hours: 5);
    clock = PlaybackClock(() => now);
  });

  test('is zero before any status', () {
    expect(clock.position, Duration.zero);
    expect(clock.playing, isFalse);
  });

  test('advances with the local clock while playing', () {
    clock.update(status(playing: true, elapsedMs: 10000));
    expect(clock.position, const Duration(seconds: 10));
    now += const Duration(milliseconds: 750);
    expect(clock.position, const Duration(milliseconds: 10750));
  });

  test('snaps to each new status', () {
    clock.update(status(playing: true, elapsedMs: 10000));
    now += const Duration(seconds: 3);
    clock.update(status(playing: true, elapsedMs: 12000));
    expect(clock.position, const Duration(seconds: 12));
    now += const Duration(seconds: 1);
    expect(clock.position, const Duration(seconds: 13));
  });

  test('snaps backwards, as after a section loop', () {
    clock.update(status(playing: true, elapsedMs: 60000));
    now += const Duration(seconds: 2);
    clock.update(status(playing: true, elapsedMs: 30000));
    expect(clock.position, const Duration(seconds: 30));
  });

  test('holds while stopped', () {
    clock.update(status(playing: false, elapsedMs: 5000));
    now += const Duration(minutes: 1);
    expect(clock.position, const Duration(seconds: 5));
  });

  test('holds from the moment a stop arrives', () {
    clock.update(status(playing: true, elapsedMs: 5000));
    now += const Duration(seconds: 2);
    clock.update(status(playing: false, elapsedMs: 7000));
    now += const Duration(seconds: 10);
    expect(clock.position, const Duration(seconds: 7));
  });

  test('never runs past the song', () {
    clock.update(status(playing: true, elapsedMs: 199000));
    now += const Duration(seconds: 30);
    expect(clock.position, const Duration(seconds: 200));
    expect(clock.duration, const Duration(seconds: 200));
  });

  test('does not clamp when the song length is unknown', () {
    clock.update(status(playing: true, elapsedMs: 1000, durationMs: 0));
    now += const Duration(seconds: 5);
    expect(clock.position, const Duration(seconds: 6));
  });

  test('reset forgets the last status', () {
    clock.update(status(playing: true, elapsedMs: 5000));
    clock.reset();
    now += const Duration(seconds: 5);
    expect(clock.position, Duration.zero);
    expect(clock.playing, isFalse);
  });
}
