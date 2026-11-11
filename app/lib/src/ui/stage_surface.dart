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
import 'package:flutter/widgets.dart';

/// The JavaScript channel the stage page talks to its host on.
const String stageHostChannel = 'MtrackHost';

/// Something that shows the stage page: a web view, in the app. The Lights
/// tab is written against this, so its tests can run with no web view.
abstract class StageSurface {
  /// Loads [page] in place of whatever is showing.
  Future<void> load(Uri page);

  /// Runs [javaScript] in the page.
  Future<void> runJavaScript(String javaScript);

  /// Paints [color] wherever the page has not painted yet.
  Future<void> setBackground(Color color);

  /// What the page posts on [stageHostChannel], as it posted it.
  Stream<String> get messages;

  /// The page itself failing to load, described for a log.
  Stream<String> get errors;

  /// The surface as a widget. It fills the box it is given, and takes every
  /// touch on it: the page's Plot | 3D switch is pressed there, and the 3D
  /// view turns under a finger.
  Widget get widget;

  /// Lets go of the page.
  void dispose();
}

/// Where the Lights tab gets the stage page and something to show it in.
abstract class StageHost {
  /// Makes sure the page can be loaded, and answers where it is. The
  /// address changes if the page had to be served again from somewhere new;
  /// a surface showing the old one loads the new one.
  Future<Uri> serve();

  /// Stops serving the page, until [serve] is called again.
  Future<void> stop();

  /// A new surface to show the page in.
  StageSurface createSurface();

  /// The view of the stage the page was last showing on this phone, `plot`
  /// or `3d`, or null when none is remembered.
  Future<String?> rememberedView();

  /// Remembers [view] (`plot` or `3d`) for the next launch.
  Future<void> rememberView(String view);
}
