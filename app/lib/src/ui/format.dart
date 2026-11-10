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
/// A time as a clock reads: `3:07`, or `1:02:03` from an hour up. Fractions of
/// a second are dropped, so a time never reads later than it is.
String formatClock(Duration time) {
  final total = time.isNegative ? 0 : time.inSeconds;
  final hours = total ~/ 3600;
  final minutes = (total % 3600) ~/ 60;
  final seconds = total % 60;
  final ss = seconds.toString().padLeft(2, '0');
  if (hours > 0) {
    return '$hours:${minutes.toString().padLeft(2, '0')}:$ss';
  }
  return '$minutes:$ss';
}

/// A length as a total, such as `52 min` or `1 h 5 min`.
String formatTotal(Duration total) {
  final minutes = total.inMinutes;
  if (minutes < 60) {
    return '$minutes min';
  }
  final rest = minutes % 60;
  return rest == 0 ? '${minutes ~/ 60} h' : '${minutes ~/ 60} h $rest min';
}

/// A gain as the mixer reads it: `−3.0 dB`, `0.0 dB`, `+3.0 dB`, with a real
/// minus sign and one decimal. A gain that rounds to zero has no sign.
String formatDb(double db) {
  final tenths = (db * 10).round();
  if (tenths == 0) {
    return '0.0 dB';
  }
  final size = tenths.abs();
  return '${tenths < 0 ? '−' : '+'}${size ~/ 10}.${size % 10} dB';
}

/// A count of songs and, when it is known, how long they run: `11 songs ·
/// 52 min`.
String formatSongs(int count, Duration total) =>
    '$count ${count == 1 ? 'song' : 'songs'}'
    '${total > Duration.zero ? ' · ${formatTotal(total)}' : ''}';
