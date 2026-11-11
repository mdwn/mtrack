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

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:webview_flutter/webview_flutter.dart';

import '../app/stage_server.dart';
import '../ui/stage_surface.dart';

/// Where the stage bundle sits among the app's assets (`make
/// app-stage-assets` puts it there).
const String stageAssetRoot = 'assets/stage';

/// Reads a file of the stage bundle from the app's assets.
Future<Uint8List?> loadBundledStageAsset(String path) async {
  try {
    final data = await rootBundle.load('$stageAssetRoot/$path');
    return data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
  } on Object {
    return null;
  }
}

/// The stage page as the app really shows it: the bundled files, served on
/// the phone's loopback address with the lighting assets [assets] answers,
/// in a platform web view. Nothing here is run by a test; it is kept to what
/// cannot be.
class WebViewStageHost implements StageHost {
  /// A host that serves the bundled page, and the lighting assets [assets]
  /// answers.
  WebViewStageHost({
    LightingAssetSource? assets,
    SharedPreferencesAsync? preferences,
  }) : _server = StageServer(load: loadBundledStageAsset, assets: assets),
       _preferences = preferences ?? SharedPreferencesAsync();

  static const String _viewKey = 'lights.view';

  final StageServer _server;
  final SharedPreferencesAsync _preferences;

  @override
  Future<String?> rememberedView() async {
    try {
      return await _preferences.getString(_viewKey);
    } on Object {
      return null;
    }
  }

  @override
  Future<void> rememberView(String view) async {
    try {
      await _preferences.setString(_viewKey, view);
    } on Object {
      // Not remembered: the next launch starts in the plot.
    }
  }

  @override
  Future<Uri> serve() => _server.start();

  @override
  Future<void> stop() => _server.stop();

  @override
  StageSurface createSurface() => WebViewStageSurface();
}

/// A platform web view showing the stage page.
class WebViewStageSurface implements StageSurface {
  /// A web view with nothing loaded.
  WebViewStageSurface() {
    unawaited(_controller.setJavaScriptMode(JavaScriptMode.unrestricted));
    unawaited(_controller.enableZoom(false));
    unawaited(
      _controller.addJavaScriptChannel(
        stageHostChannel,
        onMessageReceived: (message) => _add(_messages, message.message),
      ),
    );
    unawaited(
      _controller.setOnConsoleMessage(
        (message) => debugPrint(
          'stage page console.${message.level.name}: ${message.message}',
        ),
      ),
    );
    unawaited(
      _controller.setNavigationDelegate(
        NavigationDelegate(
          // The page goes nowhere but where it was loaded from.
          onNavigationRequest: (request) {
            final to = Uri.tryParse(request.url);
            final from = _page;
            final same =
                to != null &&
                from != null &&
                to.scheme == from.scheme &&
                to.host == from.host &&
                to.port == from.port;
            return same
                ? NavigationDecision.navigate
                : NavigationDecision.prevent;
          },
          onWebResourceError: (error) {
            final text =
                'stage page resource error ${error.errorCode} '
                '${error.description} ${error.url ?? ''}';
            // Only the page itself: a web view also asks for a favicon.
            if (error.isForMainFrame ?? true) {
              _add(_errors, text);
            } else {
              debugPrint(text);
            }
          },
          onHttpError: (error) {
            final text =
                'stage page HTTP ${error.response?.statusCode} '
                '${error.request?.uri}';
            // The page itself not being there (an app built without the
            // bundle) is the page failing to load; a favicon is not.
            final page = _page;
            if (page != null && error.request?.uri.path == page.path) {
              _add(_errors, text);
            } else {
              debugPrint(text);
            }
          },
        ),
      ),
    );
  }

  final WebViewController _controller = WebViewController();
  final StreamController<String> _messages = StreamController.broadcast();
  final StreamController<String> _errors = StreamController.broadcast();
  Uri? _page;

  void _add(StreamController<String> to, String text) {
    if (!to.isClosed) {
      to.add(text);
    }
  }

  @override
  Future<void> load(Uri page) {
    _page = page;
    return _controller.loadRequest(page);
  }

  @override
  Future<void> runJavaScript(String javaScript) =>
      _controller.runJavaScript(javaScript);

  @override
  Future<void> setBackground(Color color) =>
      _controller.setBackgroundColor(color);

  @override
  Stream<String> get messages => _messages.stream;

  @override
  Stream<String> get errors => _errors.stream;

  /// Every touch on the web view is its own at once: nothing around it in
  /// the app takes a drag, and the 3D view turns under one.
  @override
  Widget get widget => WebViewWidget(
    controller: _controller,
    gestureRecognizers: {
      const Factory<OneSequenceGestureRecognizer>(EagerGestureRecognizer.new),
    },
  );

  @override
  void dispose() {
    unawaited(_messages.close());
    unawaited(_errors.close());
  }
}
