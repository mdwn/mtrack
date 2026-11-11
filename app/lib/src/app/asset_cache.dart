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
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';

/// The kinds of lighting asset, by extension, and what each is served as:
/// the player's own list (`ASSET_TYPES` in `src/webui/api/lighting_api.rs`).
const Map<String, String> lightingAssetTypes = {
  'json': 'application/json',
  'glb': 'model/gltf-binary',
  'png': 'image/png',
  'svg': 'image/svg+xml',
};

/// The longest asset path taken. The player's are well under it: a hash
/// directory, a file name, and for a mesh one more directory.
const int maxLightingAssetPath = 200;

/// One name in an asset path: letters, digits, `.`, `_`, `-` and spaces,
/// starting with neither a dot nor a space. So no `.` or `..`, no hidden
/// file, and no separator of any platform.
final RegExp _assetName = RegExp(r'^[A-Za-z0-9_-][A-Za-z0-9._ -]*$');

/// [path] when it is the path of a lighting asset as the player names them
/// (`<hash>/rig-<hash>-v2.json`, `<hash>/models/Base.glb`), or null when it
/// is not one: empty, absolute, too long, with a name that is not a plain
/// one (see above), or of a kind not in [lightingAssetTypes].
///
/// Only a path that passes is fetched, cached or used as a file name.
String? checkLightingAssetPath(String path) {
  if (path.isEmpty || path.length > maxLightingAssetPath) {
    return null;
  }
  final names = path.split('/');
  if (names.any((name) => !_assetName.hasMatch(name) || name.endsWith(' '))) {
    return null;
  }
  return lightingAssetTypes.containsKey(lightingAssetKind(path)) ? path : null;
}

/// The kind of asset at [path]: its extension, in lower case.
String lightingAssetKind(String path) {
  final dot = path.lastIndexOf('.');
  return dot < 0 ? '' : path.substring(dot + 1).toLowerCase();
}

/// The lighting assets the phone has fetched, on its own storage.
///
/// One file per asset, named for its path, in a directory of their own. The
/// player names an asset by the hash of what it is made from, so a file
/// here never goes stale and serves every player: the cache is the app's,
/// not a player's.
///
/// A file is written whole under a temporary name and renamed into place,
/// so a read never finds half of one. The files together are kept under
/// [maxBytes]: after a write, the least recently used go first. Anything
/// that fails here is a miss, never an error: the asset is fetched again.
class LightingAssetCache {
  /// A cache in a directory of its own under the one [directory] gives (the
  /// app's cache directory), of at most [maxBytes].
  LightingAssetCache(
    this._directory, {
    this.maxBytes = defaultMaxBytes,
    Random? random,
  }) : _random = random ?? Random();

  /// The size the cache is kept under unless told otherwise.
  static const int defaultMaxBytes = 128 * 1024 * 1024;

  /// The name of the cache's own directory.
  static const String directoryName = 'lighting-assets';

  /// The most bytes kept. A single asset larger than this is still kept
  /// until the next is written.
  final int maxBytes;

  final Future<Directory> Function() _directory;
  final Random _random;
  Future<Directory>? _opened;

  /// The file name an asset is kept under: its checked path with each `/`
  /// written as `%2F` and each space as `%20` (a checked path has no `%`).
  /// Null for a path that does not pass [checkLightingAssetPath].
  static String? fileNameFor(String path) =>
      checkLightingAssetPath(path)
          ?.replaceAll('/', '%2F')
          .replaceAll(' ', '%20');

  /// The cache's directory, made if need be. One that could not be made is
  /// tried again next time.
  Future<Directory> _open() {
    final opened = _opened ??= () async {
      final dir = Directory(
        '${(await _directory()).path}${Platform.pathSeparator}$directoryName',
      );
      await dir.create(recursive: true);
      return dir;
    }();
    opened.then<void>(
      (_) {},
      onError: (Object _) {
        if (identical(_opened, opened)) {
          _opened = null;
        }
      },
    );
    return opened;
  }

  Future<File?> _file(String path) async {
    final name = fileNameFor(path);
    if (name == null) {
      return null;
    }
    final dir = await _open();
    return File('${dir.path}${Platform.pathSeparator}$name');
  }

  /// The asset at [path], or null when it is not here.
  Future<Uint8List?> read(String path) async {
    try {
      final file = await _file(path);
      if (file == null) {
        return null;
      }
      final bytes = await file.readAsBytes();
      // Its last use, which is what eviction goes by.
      try {
        await file.setLastModified(DateTime.now());
      } on Object {
        // Kept with its old time: it may go a little early.
      }
      return bytes;
    } on Object {
      return null;
    }
  }

  /// Keeps [bytes] as the asset at [path], then makes room. Nothing is kept
  /// for a path that does not pass [checkLightingAssetPath].
  Future<void> write(String path, Uint8List bytes) async {
    File? temp;
    try {
      final file = await _file(path);
      if (file == null) {
        return;
      }
      // A leading dot: no asset's file name has one.
      temp = File(
        '${file.parent.path}${Platform.pathSeparator}'
        '.tmp-${_random.nextInt(1 << 32).toRadixString(16)}',
      );
      await temp.writeAsBytes(bytes, flush: true);
      await temp.rename(file.path);
      temp = null;
      await _evict(keep: file.path);
    } on Object catch (e) {
      debugPrint('Lighting assets: could not keep $path: $e');
      try {
        await temp?.delete();
      } on Object {
        // Swept up by the next eviction.
      }
    }
  }

  /// Deletes the least recently used assets until the rest are within
  /// [maxBytes], never [keep]. Temporary files left by a write that did not
  /// finish (the app was killed) go too, unless they are recent enough to be
  /// a write still under way.
  Future<void> _evict({required String keep}) async {
    final dir = await _open();
    final kept = <(File, int, DateTime)>[];
    var total = 0;
    final now = DateTime.now();
    await for (final entry in dir.list(followLinks: false)) {
      if (entry is! File) {
        continue;
      }
      final FileStat stat;
      try {
        stat = await entry.stat();
      } on Object {
        continue;
      }
      final name = entry.uri.pathSegments.last;
      if (name.startsWith('.')) {
        if (now.difference(stat.modified) > const Duration(hours: 1)) {
          await _delete(entry);
        }
        continue;
      }
      total += stat.size;
      if (entry.path != keep) {
        kept.add((entry, stat.size, stat.modified));
      }
    }
    kept.sort((a, b) => a.$3.compareTo(b.$3));
    for (final (file, size, _) in kept) {
      if (total <= maxBytes) {
        break;
      }
      if (await _delete(file)) {
        total -= size;
      }
    }
  }

  Future<bool> _delete(File file) async {
    try {
      await file.delete();
      return true;
    } on Object {
      return false;
    }
  }
}
