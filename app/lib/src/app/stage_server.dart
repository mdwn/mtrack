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
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

/// Reads one file of the stage bundle by its path inside it (`stage.html`,
/// `assets/stage-abc.js`), or answers null when there is no such file.
typedef StageAssetLoader = Future<Uint8List?> Function(String path);

/// The page's file in the bundle.
const String stagePageFile = 'stage.html';

/// The only kinds of file served, and what each is served as. The page loads
/// its script as modules and its WebAssembly with `instantiateStreaming`,
/// and a browser refuses both unless the type is right.
const Map<String, String> stageContentTypes = {
  'html': 'text/html; charset=utf-8',
  'js': 'text/javascript; charset=utf-8',
  'css': 'text/css; charset=utf-8',
  'wasm': 'application/wasm',
  'json': 'application/json',
};

/// One name in a path: letters, digits, `.`, `_` and `-`, not starting with
/// a dot. So no `..`, no hidden file, nothing percent-encoded and no
/// backslash.
final RegExp _segment = RegExp(r'^[A-Za-z0-9_-][A-Za-z0-9._-]*$');

/// Serves the bundled stage page to the app's own web view, from the phone's
/// loopback address. A web view will not load modules or streaming
/// WebAssembly from a file, and this is the same on Android and iOS.
///
/// Another app on the phone can reach a loopback port. What is here is
/// public anyway (the web UI's stage plot), and still:
///
/// - it listens on 127.0.0.1 only, on a port the system picks;
/// - every path starts with a random name made when the server starts, so
///   the page is not at an address that can be guessed; anything else is a
///   404, the same as a file that is not there;
/// - a request whose `Host` is not this server by its loopback name is
///   refused, so a web page elsewhere cannot reach it through a DNS name it
///   points here;
/// - only GET and HEAD, only the kinds of file in [stageContentTypes], only
///   plain names: there is no listing and no way out of the bundle. (`dart:io`
///   resolves `.` and `..` in a path, and turns `\` into `/`, before it is
///   seen here; what is left must still start with the random name and be
///   made of plain names.)
class StageServer {
  /// A server of the files [load] reads. [random] makes the path's prefix.
  StageServer({required this._load, Random? random})
    : _random = random ?? Random.secure();

  final StageAssetLoader _load;
  final Random _random;

  HttpServer? _server;
  String _prefix = '';
  Future<Uri>? _starting;

  /// Where the page is, while the server runs.
  Uri? get page {
    final server = _server;
    return server == null
        ? null
        : Uri(
            scheme: 'http',
            host: InternetAddress.loopbackIPv4.address,
            port: server.port,
            path: '/$_prefix/$stagePageFile',
          );
  }

  /// Makes sure the server runs, and answers where the page is. A server
  /// that runs already is asked for the page first: one whose socket the
  /// system took while the app was in the background is replaced, at a new
  /// address.
  Future<Uri> start() {
    return _starting ??= _start().whenComplete(() => _starting = null);
  }

  Future<Uri> _start() async {
    final running = page;
    if (running != null && await _answers(running)) {
      return running;
    }
    await stop();
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final bytes = List<int>.generate(16, (_) => _random.nextInt(256));
    _prefix = base64Url.encode(bytes).replaceAll('=', '');
    _server = server;
    server.listen(
      (request) => unawaited(_handle(server, request)),
      onError: (Object _) => _gone(server),
      onDone: () => _gone(server),
    );
    return page!;
  }

  void _gone(HttpServer server) {
    if (identical(_server, server)) {
      _server = null;
    }
  }

  /// Whether the server still answers for the page.
  Future<bool> _answers(Uri page) async {
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 2);
    try {
      final request = await client
          .openUrl('HEAD', page)
          .timeout(const Duration(seconds: 2));
      final response = await request.close().timeout(
        const Duration(seconds: 2),
      );
      await response.drain<void>();
      return response.statusCode == HttpStatus.ok;
    } on Object {
      return false;
    } finally {
      client.close(force: true);
    }
  }

  /// Stops serving. [start] starts again, at a new address.
  Future<void> stop() async {
    final server = _server;
    _server = null;
    await server?.close(force: true);
  }

  Future<void> _handle(HttpServer server, HttpRequest request) async {
    final response = request.response;
    try {
      response.headers
        ..set(HttpHeaders.cacheControlHeader, 'no-store')
        ..set('X-Content-Type-Options', 'nosniff');
      final host = request.headers.value(HttpHeaders.hostHeader);
      if (host != '127.0.0.1:${server.port}' &&
          host != 'localhost:${server.port}') {
        response.statusCode = HttpStatus.forbidden;
        return;
      }
      if (request.method != 'GET' && request.method != 'HEAD') {
        response.statusCode = HttpStatus.methodNotAllowed;
        response.headers.set(HttpHeaders.allowHeader, 'GET, HEAD');
        return;
      }
      final file = _fileFor(request.uri.path);
      final type = file == null
          ? null
          : stageContentTypes[file.substring(file.lastIndexOf('.') + 1)];
      final bytes = type == null ? null : await _load(file!);
      if (bytes == null) {
        response.statusCode = HttpStatus.notFound;
        return;
      }
      response.headers.set(HttpHeaders.contentTypeHeader, type!);
      response.contentLength = bytes.length;
      if (request.method == 'GET') {
        response.add(bytes);
      }
    } on Object {
      // The web view went away mid-answer, or the bundle could not be read.
      try {
        response.statusCode = HttpStatus.internalServerError;
      } on Object {
        // The headers have gone already.
      }
    } finally {
      try {
        await response.close();
      } on Object {
        // Nobody is there to hear it.
      }
    }
  }

  /// The bundle's file a request path names, or null when it names none:
  /// the prefix is missing or wrong, or a name in it is not a plain one.
  String? _fileFor(String path) {
    final lead = '/$_prefix/';
    if (!path.startsWith(lead)) {
      return null;
    }
    final names = path.substring(lead.length).split('/');
    if (names.any((name) => !_segment.hasMatch(name)) ||
        !names.last.contains('.')) {
      return null;
    }
    return names.join('/');
  }
}
