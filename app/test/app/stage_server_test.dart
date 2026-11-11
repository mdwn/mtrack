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
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mtrack_remote/src/app/stage_server.dart';

/// An answer read off a socket.
class Answer {
  Answer(this.status, this.headers, this.body);

  final int status;
  final Map<String, String> headers;
  final List<int> body;

  String get text => utf8.decode(body);
}

/// Sends [request] to the loopback [port] as written, and reads the answer:
/// no client library tidies the path or the headers on the way.
Future<Answer> send(int port, String request) async {
  final socket = await Socket.connect(InternetAddress.loopbackIPv4, port);
  socket.write(request);
  await socket.flush();
  final bytes = BytesBuilder();
  await socket.forEach(bytes.add);
  socket.destroy();
  final all = bytes.takeBytes();
  var split = -1;
  for (var i = 0; i + 3 < all.length; i++) {
    if (all[i] == 13 && all[i + 1] == 10 && all[i + 2] == 13) {
      split = i;
      break;
    }
  }
  final head = latin1.decode(all.sublist(0, split)).split('\r\n');
  final headers = <String, String>{};
  for (final line in head.skip(1)) {
    final colon = line.indexOf(':');
    headers[line.substring(0, colon).toLowerCase()] = line
        .substring(colon + 1)
        .trim();
  }
  return Answer(
    int.parse(head.first.split(' ')[1]),
    headers,
    all.sublist(split + 4),
  );
}

void main() {
  final files = <String, List<int>>{
    'stage.html': utf8.encode('<!doctype html><title>stage</title>'),
    'assets/stage-abc.js': utf8.encode('export {};'),
    'assets/_left-out-x.js': utf8.encode('export default undefined;'),
    'assets/stage-abc.css': utf8.encode('html{}'),
    'assets/rules-abc.wasm': [0, 97, 115, 109, 1, 0, 0, 0],
    'assets/table.json': utf8.encode('{}'),
    'assets/notes.md': utf8.encode('not served'),
    'README.md': utf8.encode('not served'),
    '.gitignore': utf8.encode('*'),
    'secret': utf8.encode('not served'),
  };
  final asked = <String>[];
  late StageServer server;
  late Uri page;
  late int port;
  late String prefix;

  /// A request for [path] with this server's own Host header.
  Future<Answer> get(String path, {String method = 'GET', String? host}) =>
      send(
        port,
        '$method $path HTTP/1.1\r\n'
        'Host: ${host ?? '127.0.0.1:$port'}\r\n'
        'Connection: close\r\n\r\n',
      );

  setUp(() async {
    asked.clear();
    server = StageServer(
      load: (path) async {
        asked.add(path);
        final bytes = files[path];
        return bytes == null ? null : Uint8List.fromList(bytes);
      },
    );
    page = await server.start();
    port = page.port;
    prefix = '/${page.pathSegments.first}';
  });

  tearDown(() => server.stop());

  test('the page is on the loopback address, under a random name', () {
    expect(page.scheme, 'http');
    expect(page.host, '127.0.0.1');
    expect(page.port, greaterThan(0));
    expect(page.pathSegments, hasLength(2));
    expect(page.pathSegments.last, 'stage.html');
    // 16 random bytes, as base64 for a URL.
    expect(page.pathSegments.first, matches(RegExp(r'^[A-Za-z0-9_-]{22}$')));
  });

  test('the name is different every time it starts', () async {
    final names = {page.pathSegments.first};
    for (var i = 0; i < 3; i++) {
      await server.stop();
      names.add((await server.start()).pathSegments.first);
    }
    expect(names, hasLength(4));
  });

  test('the name comes from the random source it is given', () async {
    final a = StageServer(load: (_) async => null, random: Random(1));
    final b = StageServer(load: (_) async => null, random: Random(1));
    try {
      expect(
        (await a.start()).pathSegments.first,
        (await b.start()).pathSegments.first,
      );
    } finally {
      await a.stop();
      await b.stop();
    }
  });

  group('serves', () {
    const types = {
      'stage.html': 'text/html; charset=utf-8',
      'assets/stage-abc.js': 'text/javascript; charset=utf-8',
      'assets/_left-out-x.js': 'text/javascript; charset=utf-8',
      'assets/stage-abc.css': 'text/css; charset=utf-8',
      'assets/rules-abc.wasm': 'application/wasm',
      'assets/table.json': 'application/json',
    };
    for (final entry in types.entries) {
      test('${entry.key} as ${entry.value}', () async {
        final answer = await get('$prefix/${entry.key}');
        expect(answer.status, 200);
        expect(answer.headers['content-type'], entry.value);
        expect(answer.body, files[entry.key]);
        expect(answer.headers['content-length'], '${answer.body.length}');
        expect(answer.headers['cache-control'], 'no-store');
        expect(answer.headers['x-content-type-options'], 'nosniff');
        expect(answer.headers.keys, isNot(contains(startsWith('access-'))));
      });
    }

    test('the page with a query, as the app loads it', () async {
      final answer = await get('$prefix/stage.html?theme=dark');
      expect(answer.status, 200);
      expect(answer.text, contains('<title>stage</title>'));
    });

    test('to a Host of localhost', () async {
      expect(
        (await get('$prefix/stage.html', host: 'localhost:$port')).status,
        200,
      );
    });

    test('HEAD with the headers and no body', () async {
      final answer = await get('$prefix/assets/rules-abc.wasm', method: 'HEAD');
      expect(answer.status, 200);
      expect(answer.headers['content-type'], 'application/wasm');
      expect(answer.headers['content-length'], '8');
      expect(answer.body, isEmpty);
    });
  });

  group('answers 404 for', () {
    Future<void> notFound(String path) async {
      final answer = await get(path);
      expect(answer.status, 404, reason: path);
      expect(answer.body, isEmpty, reason: path);
    }

    test('a file that is not there', () async {
      await notFound('$prefix/assets/missing.js');
    });

    test('the page without the random name', () async {
      await notFound('/stage.html');
      await notFound('/');
      await notFound('/favicon.ico');
      await notFound('/assets/stage-abc.js');
      expect(asked, isEmpty);
    });

    test('the page under a wrong name', () async {
      await notFound('/${'A' * 22}/stage.html');
      await notFound('$prefix.x/stage.html');
      await notFound('/x$prefix/stage.html');
      expect(asked, isEmpty);
    });

    test('a directory: there is no listing', () async {
      await notFound(prefix);
      await notFound('$prefix/');
      await notFound('$prefix/assets');
      await notFound('$prefix/assets/');
      expect(asked, isEmpty);
    });

    test(
      'a kind of file it does not serve, though the bundle has it',
      () async {
        await notFound('$prefix/README.md');
        await notFound('$prefix/assets/notes.md');
        await notFound('$prefix/secret');
        await notFound('$prefix/.gitignore');
        expect(asked, isEmpty);
      },
    );

    test('a path that climbs out of the bundle', () async {
      for (final path in [
        '$prefix/../stage.html',
        '$prefix/assets/../../stage.html',
        '$prefix//stage.html',
        '$prefix/%2e%2e/stage.html',
        '$prefix/%2E%2E/%2E%2E/etc/passwd.json',
        '$prefix/assets/..%2fstage.html',
        '$prefix/assets%2fstage-abc.js',
        '$prefix/..\\stage.html',
        '$prefix/assets/stage-abc.js%00.html',
        '$prefix/assets/.hidden.js',
        '/..$prefix/../stage.html',
      ]) {
        await notFound(path);
      }
      // Whatever a path was tidied into before it was read, nothing but a
      // plain name inside the bundle was ever asked of it.
      for (final path in asked) {
        expect(path, isNot(contains('..')));
        expect(files.keys, contains(path));
      }
    });
  });

  test('a path that stays inside the bundle once tidied is the file it '
      'comes to', () async {
    // dart:io resolves the dots, and turns a backslash into a slash, before
    // the server sees the path, as a browser does before it asks; what is
    // left is checked like any other.
    for (final path in [
      '$prefix/assets/../stage.html',
      '$prefix/./stage.html',
      '$prefix/assets/./../assets/../stage.html',
      '$prefix\\stage.html',
    ]) {
      final answer = await get(path);
      expect(answer.status, 200, reason: path);
      expect(answer.text, contains('<title>stage</title>'), reason: path);
    }
    expect(asked.toSet(), {'stage.html'});
  });

  group('refuses', () {
    test('a Host that is not this server', () async {
      for (final host in [
        'evil.example:$port',
        'evil.example',
        '127.0.0.1',
        '127.0.0.1:${port + 1}',
        '127.0.0.1:$port.evil.example',
        'LOCALHOST.evil.example:$port',
        '[::1]:$port',
      ]) {
        final answer = await get('$prefix/stage.html', host: host);
        expect(answer.status, 403, reason: host);
        expect(answer.body, isEmpty, reason: host);
      }
      expect(asked, isEmpty);
    });

    test('a request with no Host', () async {
      final answer = await send(
        port,
        'GET $prefix/stage.html HTTP/1.0\r\n\r\n',
      );
      expect(answer.status, 403);
    });

    test('anything but GET and HEAD', () async {
      for (final method in ['POST', 'PUT', 'DELETE', 'OPTIONS', 'PATCH']) {
        final answer = await get('$prefix/stage.html', method: method);
        expect(answer.status, 405, reason: method);
        expect(answer.headers['allow'], 'GET, HEAD', reason: method);
        expect(answer.body, isEmpty, reason: method);
      }
      expect(asked, isEmpty);
    });
  });

  group('starting', () {
    test('again while it runs keeps the address', () async {
      expect(await server.start(), page);
      expect(await server.start(), page);
      expect(server.page, page);
    });

    test('twice at once is one start', () async {
      await server.stop();
      final both = await Future.wait([server.start(), server.start()]);
      expect(both[0], both[1]);
    });

    test('after a stop serves again, somewhere new', () async {
      await server.stop();
      expect(server.page, isNull);
      await expectLater(
        Socket.connect(InternetAddress.loopbackIPv4, port),
        throwsA(isA<SocketException>()),
      );
      final again = await server.start();
      expect(again, isNot(page));
      port = again.port;
      expect((await get(again.path)).status, 200);
    });

    test('replaces a server that no longer answers', () async {
      // The bundle stops answering for the page, as a server would whose
      // socket the system took.
      final kept = files.remove('stage.html')!;
      try {
        final again = await server.start();
        expect(again, isNot(page));
        expect(server.page, again);
      } finally {
        files['stage.html'] = kept;
      }
    });
  });

  test('a bundle that cannot be read is an error, not a hang', () async {
    final broken = StageServer(load: (_) => throw StateError('no bundle'));
    try {
      final at = await broken.start();
      final answer = await send(
        at.port,
        'GET ${at.path} HTTP/1.1\r\nHost: 127.0.0.1:${at.port}\r\n'
        'Connection: close\r\n\r\n',
      );
      expect(answer.status, 500);
    } finally {
      await broken.stop();
    }
  });
}
