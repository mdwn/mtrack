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
import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/gen/player/v1/player.pb.dart' as pp;
import 'package:mtrack_remote/src/app/playlist_settings.dart';

void main() {
  group('a fade', () {
    final full = pp.Fade(
      length: '2beats',
      curve: 'equal_power',
      exclude: pp.TrackNames(names: ['Click', 'Cue']),
      lights: '0s',
    );

    test('takes a new length and keeps the rest', () {
      final next = withFadeLength(full, ' 1measure ')!;
      expect(next.length, '1measure');
      expect(next.curve, 'equal_power');
      expect(next.exclude.names, ['Click', 'Cue']);
      expect(next.lights, '0s');
      expect(full.length, '2beats', reason: 'the original is left alone');
    });

    test('is made from a length, and an empty one is none', () {
      expect(withFadeLength(null, '1s'), pp.Fade(length: '1s'));
      expect(withFadeLength(full, '  '), isNull);
    });

    test('says what it has beyond its length', () {
      expect(
        fadeDetails(full),
        "equal power curve, doesn't fade 2 tracks, lights 0s",
      );
      expect(fadeDetails(pp.Fade(length: '1s')), isNull);
      expect(fadeDetails(null), isNull);
      expect(
        fadeDetails(pp.Fade(length: '1s', exclude: pp.TrackNames())),
        'every track fades',
      );
    });
  });

  group('ambient light', () {
    test('is inherited, none or its own', () {
      expect(ambientChoiceOf(null), AmbientChoice.inherit);
      expect(ambientChoiceOf(pp.AmbientSetting()), AmbientChoice.off);
      expect(ambientChoiceOf(pp.AmbientSetting(on: true)), AmbientChoice.own);
    });

    test('keeps its own settings while it stays its own', () {
      final own = pp.AmbientSetting(on: true, show: 'a.light');
      expect(ambientForChoice(AmbientChoice.own, own), same(own));
      expect(
        ambientForChoice(AmbientChoice.own, null),
        pp.AmbientSetting(on: true),
      );
      expect(ambientForChoice(AmbientChoice.off, own), pp.AmbientSetting());
      expect(ambientForChoice(AmbientChoice.inherit, own), isNull);
    });

    test('a field set, and cleared to take the level above', () {
      final own = pp.AmbientSetting(on: true, show: 'a.light');
      final timed = withAmbientText(own, AmbientField.fadeIn, ' 6s ');
      expect(timed.fadeIn, '6s');
      expect(own.hasFadeIn(), isFalse, reason: 'the original is left alone');
      expect(ambientText(timed, AmbientField.fadeIn), '6s');
      final cleared = withAmbientText(timed, AmbientField.show, '');
      expect(cleared.hasShow(), isFalse);
      expect(ambientText(cleared, AmbientField.show), '');
      expect(ambientText(null, AmbientField.delay), '');
    });

    test("a playlist's is summed up", () {
      expect(playlistAmbientSummary(null), "The profile's");
      expect(playlistAmbientSummary(pp.AmbientSetting()), 'None');
      expect(
        playlistAmbientSummary(
          pp.AmbientSetting(on: true, show: 'lighting/walk_in.light'),
        ),
        'walk_in.light',
      );
      expect(playlistAmbientSummary(pp.AmbientSetting(on: true)), 'Its own');
    });
  });

  test("an entry's settings are summed up as the web UI does", () {
    expect(entrySummary(pp.EntrySettings()), '');
    expect(
      entrySummary(
        pp.EntrySettings(
          fadeOut: pp.Fade(length: '2beats'),
          fadeIn: pp.Fade(length: '1s'),
          autoPlay: pp.AutoPlay(gap: '2s'),
          ambient: pp.AmbientSetting(),
        ),
      ),
      'fade out 2beats · fade in 1s · auto play, 2s gap · no ambient before',
    );
    expect(
      entrySummary(
        pp.EntrySettings(
          autoPlay: pp.AutoPlay(),
          ambient: pp.AmbientSetting(on: true),
        ),
      ),
      'auto play · own ambient',
    );
  });

  group('the shape check for a length', () {
    test('lets every time the player reads through', () {
      for (final text in [
        '',
        '  ',
        '0',
        ' 0 ',
        '2s',
        '500ms',
        '1.5s',
        '2.5ms',
        '2m',
        '1h',
        '1m30s',
        ' 1 m 30 s ',
        '.5s',
        '5.s',
        '+2s',
        '-0s',
        '1e3ms',
        '1e2s',
        '1 000ms',
        '7ns',
        '250us',
        '1d',
        '2w',
        '1y',
        '1h30m',
        '1d2h3m4s5ms',
        '0s',
        '00s',
      ]) {
        expect(looksLikeTime(text), isTrue, reason: '"$text"');
        expect(looksLikeLength(text), isTrue, reason: '"$text"');
      }
    });

    test('lets every musical length the player reads through', () {
      for (final text in [
        '4beats',
        '1beat',
        '1measure',
        '2measures',
        '8 beats',
        ' 2.5 measures ',
        '.5beat',
        '1e1beats',
      ]) {
        expect(looksLikeLength(text), isTrue, reason: '"$text"');
        // Not for a time-only field.
        expect(looksLikeTime(text), isFalse, reason: '"$text"');
      }
    });

    test('refuses text that cannot be a length', () {
      for (final text in [
        '2bananas',
        'soon',
        '2',
        's',
        'beats',
        '1x',
        '2 seconds',
        '1s 30',
        '1s,2s',
        '1/2s',
        '1.2.3s',
        '1S',
        '4Beats',
        '4beatss',
        'abeats',
        '4 beats later',
      ]) {
        expect(looksLikeLength(text), isFalse, reason: '"$text"');
        expect(looksLikeTime(text), isFalse, reason: '"$text"');
      }
    });
  });
}
