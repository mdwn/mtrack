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

/// The app's colours in one theme, named by what they are for.
///
/// The palette is the web UI's (ink, paper, the warm greys, cyan, amber, pink,
/// success green, error red). Each theme is designed, not inverted: the accent
/// colours that are text on the page are darker in the light theme so they read
/// on paper. What a colour means is the same in both: Play is green, Stop is
/// red, the current item is cyan and a looping section is amber.
@immutable
class AppColors extends ThemeExtension<AppColors> {
  /// A set of colours.
  const AppColors({
    required this.page,
    required this.surface,
    required this.raised,
    required this.line,
    required this.text,
    required this.muted,
    required this.dim,
    required this.cyan,
    required this.amber,
    required this.green,
    required this.red,
    required this.play,
    required this.onPlay,
    required this.stop,
    required this.onStop,
    required this.stopFill,
    required this.currentTint,
    required this.loopTint,
    required this.shadow,
  });

  /// The page behind everything.
  final Color page;

  /// Cards and rows.
  final Color surface;

  /// Tracks, wells and other raised neutral fills.
  final Color raised;

  /// Borders and dividers.
  final Color line;

  /// Primary text.
  final Color text;

  /// Secondary text.
  final Color muted;

  /// Tertiary text: hints and labels. Still readable, not decoration.
  final Color dim;

  /// The current item, as text or a mark.
  final Color cyan;

  /// A looping section, as text or a mark.
  final Color amber;

  /// "Playing" and success, as text.
  final Color green;

  /// Errors and warnings, as text.
  final Color red;

  /// The Play button's fill, and the text on it.
  final Color play;
  final Color onPlay;

  /// The Stop button's fill, the text on it, and the fill that grows over it
  /// while it is held.
  final Color stop;
  final Color onStop;
  final Color stopFill;

  /// Background of the current row.
  final Color currentTint;

  /// Background of the looping row.
  final Color loopTint;

  /// The hard shadow under the main button: the web UI's "pop".
  final Color shadow;

  /// The dark theme.
  static const AppColors dark = AppColors(
    page: Color(0xFF141112),
    surface: Color(0xFF231F20),
    raised: Color(0xFF2F2A2B),
    line: Color(0xFF3D3738),
    text: Color(0xFFF7F6F6),
    muted: Color(0xFFA79FA1),
    dim: Color(0xFF8F8789),
    cyan: Color(0xFF5ECAEA),
    amber: Color(0xFFF2B544),
    green: Color(0xFF4DC08A),
    red: Color(0xFFFF7A7A),
    play: Color(0xFF4DC08A),
    onPlay: Color(0xFF0D2A1C),
    stop: Color(0xFFC93535),
    onStop: Color(0xFFFFFFFF),
    stopFill: Color(0xFF8E1F1F),
    currentTint: Color(0x245ECAEA),
    loopTint: Color(0x24F2B544),
    shadow: Color(0xFF000000),
  );

  /// The light theme.
  static const AppColors light = AppColors(
    page: Color(0xFFF7F6F6),
    surface: Color(0xFFFFFFFF),
    raised: Color(0xFFE6E2E3),
    line: Color(0xFFD6D1D2),
    text: Color(0xFF231F20),
    muted: Color(0xFF5F5759),
    dim: Color(0xFF6B6365),
    cyan: Color(0xFF0B6A85),
    amber: Color(0xFF8A5F0E),
    green: Color(0xFF1D7A50),
    red: Color(0xFFC42F2F),
    play: Color(0xFF35A873),
    onPlay: Color(0xFF0D2A1C),
    stop: Color(0xFFC93535),
    onStop: Color(0xFFFFFFFF),
    stopFill: Color(0xFF8E1F1F),
    currentTint: Color(0x1A1487A8),
    loopTint: Color(0x24C98D1C),
    shadow: Color(0xFF231F20),
  );

  @override
  AppColors copyWith() => this;

  @override
  AppColors lerp(ThemeExtension<AppColors>? other, double t) =>
      t < 0.5 ? this : (other as AppColors? ?? this);
}

/// The font families bundled with the app.
abstract final class AppFonts {
  /// Headings and the song title.
  static const String display = 'Nunito';

  /// Body text.
  static const String sans = 'Inter';

  /// Times and durations.
  static const String mono = 'JetBrainsMono';
}

TextStyle _style(
  String family,
  double size,
  int weight, {
  double? height,
  double? letterSpacing,
  List<FontFeature>? features,
}) {
  return TextStyle(
    fontFamily: family,
    fontSize: size,
    fontWeight: FontWeight.values[(weight ~/ 100) - 1],
    // The fonts are variable: the weight is an axis, not a file.
    fontVariations: [FontVariation('wght', weight.toDouble())],
    height: height,
    letterSpacing: letterSpacing,
    fontFeatures: features,
  );
}

/// The app's text styles, without colour: the widget that uses one says what
/// colour it is.
abstract final class AppText {
  /// The song title, before it is shrunk to fit.
  static TextStyle songTitle(double size) =>
      _style(AppFonts.display, size, 900, height: 1.05);

  /// A screen heading.
  static final TextStyle heading = _style(
    AppFonts.display,
    26,
    900,
    height: 1.1,
  );

  /// A button's label.
  static TextStyle button(double size) =>
      _style(AppFonts.display, size, 800, letterSpacing: 1);

  /// A row's title.
  static final TextStyle row = _style(AppFonts.sans, 15, 500);

  /// A row's title when it is the current one.
  static final TextStyle rowCurrent = _style(AppFonts.sans, 15, 600);

  /// Ordinary text.
  static final TextStyle body = _style(AppFonts.sans, 14, 400, height: 1.4);

  /// Bold ordinary text.
  static final TextStyle bodyStrong = _style(AppFonts.sans, 14, 600);

  /// Small supporting text.
  static final TextStyle small = _style(AppFonts.sans, 12.5, 500);

  /// A small uppercase label.
  static final TextStyle eyebrow = _style(
    AppFonts.sans,
    11.5,
    600,
    letterSpacing: 1.0,
  );

  /// Times and durations, in a size.
  static TextStyle time(double size, {int weight = 700}) => _style(
    AppFonts.mono,
    size,
    weight,
    letterSpacing: size > 24 ? -0.5 : 0,
    features: const [FontFeature.tabularFigures()],
  );
}

/// The colours of the theme in force.
extension AppColorsContext on BuildContext {
  /// The app's colours for the current theme.
  AppColors get colors => Theme.of(this).extension<AppColors>()!;
}

/// The theme for [colors] and [brightness].
ThemeData buildTheme(AppColors colors, Brightness brightness) {
  final scheme = ColorScheme(
    brightness: brightness,
    primary: colors.cyan,
    onPrimary: colors.page,
    secondary: colors.amber,
    onSecondary: colors.page,
    error: colors.red,
    onError: colors.page,
    surface: colors.surface,
    onSurface: colors.text,
  );
  final base = ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    scaffoldBackgroundColor: colors.page,
    fontFamily: AppFonts.sans,
    extensions: [colors],
  );
  return base.copyWith(
    textTheme: base.textTheme.apply(
      fontFamily: AppFonts.sans,
      bodyColor: colors.text,
      displayColor: colors.text,
    ),
    dividerColor: colors.line,
    splashFactory: InkRipple.splashFactory,
    snackBarTheme: SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      backgroundColor: colors.raised,
      contentTextStyle: AppText.body.copyWith(color: colors.text),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(color: colors.line),
      ),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: colors.surface,
      modalBackgroundColor: colors.surface,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(20)),
      ),
    ),
    progressIndicatorTheme: ProgressIndicatorThemeData(color: colors.cyan),
    appBarTheme: AppBarTheme(
      backgroundColor: colors.page,
      foregroundColor: colors.text,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
    ),
  );
}

/// The light theme.
final ThemeData lightTheme = buildTheme(AppColors.light, Brightness.light);

/// The dark theme.
final ThemeData darkTheme = buildTheme(AppColors.dark, Brightness.dark);
