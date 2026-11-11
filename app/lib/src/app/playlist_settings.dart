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

/// A playlist's settings as plain values: what the editor's sheets show and
/// change, kept apart from the widgets so they can be checked alone.
///
/// Lengths stay text: the player reads them and says what is wrong with one
/// when the playlist is saved, so the app has no parser of its own.
library;

import '../../gen/player/v1/player.pb.dart' as player;

/// What a length may be, as the player reads one, for a hint under a field.
const String lengthUnitsHint =
    'A time (500ms, 2s) or musical (4beats, 1measure). 0s for none.';

/// What a gap or an ambient timing may be: times only.
const String timeUnitsHint = 'A time, such as 2s or 500ms.';

/// Said under a fade's length that cannot be a length.
const String notALengthMessage =
    'Not a length. Try 2s, 500ms, 4beats or 1measure.';

/// Said under a gap or an ambient timing that cannot be a time.
const String notATimeMessage = 'Not a time. Try 2s or 500ms.';

// A number as the player reads one: an optional sign, digits with an
// optional point (`5.`, `.5`), an optional exponent (`1e3`).
const String _number = r'[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?';

// The player's clock units (config/length.rs): ms, s, m, h, and also ns,
// us, d, w, y. Lower case only, as the player reads them.
const String _clockUnit = '(?:ns|us|ms|s|m|h|d|w|y)';

final RegExp _timePattern = RegExp('^(?:$_number$_clockUnit)+\$');

final RegExp _musicalPattern = RegExp(
  '^\\s*$_number\\s*(?:beats?|measures?)\\s*\$',
);

/// Whether [text] could be a time the player reads: one or more
/// `<number><unit>` parts (`2s`, `500ms`, `1.5s`, `1m30s`, `.5s`, spaces
/// anywhere between them), or a bare `0`. Empty (none, or the level
/// above's) is always fine.
///
/// Deliberately lenient. The player's check is the real one, made when the
/// playlist is saved; this only catches text that cannot be a time at all
/// (`2bananas`), so it accepts every unit the player does and leaves the
/// rest (negative, too long, nothing) to it.
bool looksLikeTime(String text) {
  // The player ignores whitespace anywhere in a time (`1 000ms`).
  final value = text.replaceAll(RegExp(r'\s'), '');
  return value.isEmpty || value == '0' || _timePattern.hasMatch(value);
}

/// Whether [text] could be a fade's length the player reads: a time (see
/// [looksLikeTime]) or `<number>beat`, `beats`, `measure` or `measures`.
/// Empty is always fine. Lenient in the same way: the player's check is
/// the real one.
bool looksLikeLength(String text) =>
    looksLikeTime(text) || _musicalPattern.hasMatch(text.trim());

/// Where an ambient light setting comes from.
enum AmbientChoice {
  /// The level above's: the playlist's for a song, the profile's for a
  /// playlist.
  inherit,

  /// None.
  off,

  /// Settings of its own, each overriding the level above's.
  own,
}

/// The choice an ambient setting is.
AmbientChoice ambientChoiceOf(player.AmbientSetting? ambient) {
  if (ambient == null) {
    return AmbientChoice.inherit;
  }
  return ambient.on ? AmbientChoice.own : AmbientChoice.off;
}

/// The ambient setting for [choice], keeping [current]'s own settings when
/// it stays [AmbientChoice.own].
player.AmbientSetting? ambientForChoice(
  AmbientChoice choice,
  player.AmbientSetting? current,
) => switch (choice) {
  AmbientChoice.inherit => null,
  AmbientChoice.off => player.AmbientSetting(),
  AmbientChoice.own =>
    current != null && current.on ? current : player.AmbientSetting(on: true),
};

/// One of an ambient setting's text fields.
enum AmbientField {
  /// The `.light` file.
  show,

  /// How long the last frame holds first.
  delay,

  /// How long it takes to come up.
  fadeIn,

  /// How long it takes to cross into a song's show.
  fadeOut,
}

/// [field] of [ambient], or empty when it does not set it.
String ambientText(player.AmbientSetting? ambient, AmbientField field) {
  if (ambient == null) {
    return '';
  }
  return switch (field) {
    AmbientField.show => ambient.hasShow() ? ambient.show : '',
    AmbientField.delay => ambient.hasDelay() ? ambient.delay : '',
    AmbientField.fadeIn => ambient.hasFadeIn() ? ambient.fadeIn : '',
    AmbientField.fadeOut => ambient.hasFadeOut() ? ambient.fadeOut : '',
  };
}

/// [ambient] with [field] set to [text], trimmed, or cleared when empty (to
/// take the level above's). [ambient] itself is left alone.
player.AmbientSetting withAmbientText(
  player.AmbientSetting ambient,
  AmbientField field,
  String text,
) {
  final next = ambient.deepCopy();
  final value = text.trim();
  switch (field) {
    case AmbientField.show:
      next.clearShow();
      if (value.isNotEmpty) {
        next.show = value;
      }
    case AmbientField.delay:
      next.clearDelay();
      if (value.isNotEmpty) {
        next.delay = value;
      }
    case AmbientField.fadeIn:
      next.clearFadeIn();
      if (value.isNotEmpty) {
        next.fadeIn = value;
      }
    case AmbientField.fadeOut:
      next.clearFadeOut();
      if (value.isNotEmpty) {
        next.fadeOut = value;
      }
  }
  return next;
}

/// [fade] with its length set to [length], trimmed: its other settings
/// (curve, tracks left out, lights), which the app does not edit, kept. An
/// empty length is no fade at all, as in the web UI.
player.Fade? withFadeLength(player.Fade? fade, String length) {
  final value = length.trim();
  if (value.isEmpty) {
    return null;
  }
  final next = fade?.deepCopy() ?? player.Fade();
  next.length = value;
  return next;
}

/// What a fade has beyond its length, which the app shows but does not
/// edit; null when it has nothing more.
String? fadeDetails(player.Fade? fade) {
  if (fade == null) {
    return null;
  }
  final parts = [
    if (fade.hasCurve()) '${fade.curve.replaceAll('_', ' ')} curve',
    if (fade.hasExclude())
      fade.exclude.names.isEmpty
          ? 'every track fades'
          : fade.exclude.names.length == 1
          ? "doesn't fade ${fade.exclude.names.single}"
          : "doesn't fade ${fade.exclude.names.length} tracks",
    if (fade.hasLights()) 'lights ${fade.lights}',
  ];
  return parts.isEmpty ? null : parts.join(', ');
}

/// The gap of an auto play, empty for at once.
String autoPlayGap(player.AutoPlay autoPlay) => autoPlay.gap.trim();

/// Everything an entry sets, in short, for its row; empty when it sets
/// nothing. As the web UI's transitions button says it.
String entrySummary(player.EntrySettings settings) {
  final parts = <String>[
    if (settings.hasFadeOut()) 'fade out ${settings.fadeOut.length}',
    if (settings.hasFadeIn()) 'fade in ${settings.fadeIn.length}',
    if (settings.hasAutoPlay())
      autoPlayGap(settings.autoPlay).isEmpty
          ? 'auto play'
          : 'auto play, ${autoPlayGap(settings.autoPlay)} gap',
    if (settings.hasAmbient())
      settings.ambient.on ? 'own ambient' : 'no ambient before',
  ];
  return parts.join(' · ');
}

/// What a playlist's ambient light is, in a few words.
String playlistAmbientSummary(player.AmbientSetting? ambient) =>
    switch (ambientChoiceOf(ambient)) {
      AmbientChoice.inherit => "The profile's",
      AmbientChoice.off => 'None',
      AmbientChoice.own =>
        ambient!.hasShow() ? ambient.show.split('/').last : 'Its own',
    };
