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
import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'src/app/asset_cache.dart';
import 'src/app/session.dart';
import 'src/ble/ble_link.dart';
import 'src/platform/preferences_store.dart';
import 'src/platform/wakelock_screen_wake.dart';
import 'src/platform/webview_stage.dart';
import 'src/ui/app.dart';
import 'src/ui/licences.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  registerFontLicences();
  final session = Session(
    link: BlePlayerLink(),
    store: PreferencesPlayerStore(),
    wake: const WakelockScreenWake(),
    assetCache: LightingAssetCache(getApplicationCacheDirectory),
  );
  session.start();
  runApp(
    MtrackApp(
      session: session,
      stage: WebViewStageHost(assets: session.lightingAssets.get),
    ),
  );
}
