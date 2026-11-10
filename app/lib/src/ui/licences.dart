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
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The bundled fonts and where their licence texts are.
const Map<String, String> _fontLicences = {
  'Nunito': 'assets/fonts/nunito/OFL.txt',
  'Inter': 'assets/fonts/inter/OFL.txt',
  'JetBrains Mono': 'assets/fonts/jetbrains_mono/OFL.txt',
};

/// Adds the bundled fonts' licences to the licence page. Call once at start.
void registerFontLicences([AssetBundle? bundle]) {
  LicenseRegistry.addLicense(() async* {
    final source = bundle ?? rootBundle;
    for (final entry in _fontLicences.entries) {
      yield LicenseEntryWithLineBreaks([
        '${entry.key} font',
      ], await source.loadString(entry.value));
    }
  });
}
