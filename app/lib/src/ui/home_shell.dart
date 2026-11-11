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

import '../app/ports.dart';
import 'levels_screen.dart';
import 'library_screen.dart';
import 'lights_screen.dart';
import 'now_screen.dart';
import 'playlist_editor.dart';
import 'sections_screen.dart';
import 'session_scope.dart';
import 'stage_surface.dart';
import 'theme.dart';

/// The five tabs and the bar to switch between them. Each tab keeps its state
/// while another is showing.
class HomeShell extends StatefulWidget {
  /// The tabs. The Lights tab shows the stage page it gets from [stage];
  /// [views] keeps which side of the Library tab was showing.
  const HomeShell({required this.stage, required this.views, super.key});

  /// Where the Lights tab's stage page comes from.
  final StageHost stage;

  /// Where view choices are kept across launches.
  final ViewStore views;

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  static const List<String> _labels = [
    'Now',
    'Library',
    'Sections',
    'Levels',
    'Lights',
  ];

  /// The size of a tab's label before the phone's text setting.
  static const double _labelSize = 12.5;

  /// The room kept clear on each side of a tab's label.
  static const double _labelInset = 4;

  int _index = 0;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final editing = SessionScope.of(context).editing != null;
    return PopScope(
      canPop: !editing,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) {
          leaveEditor(context);
        }
      },
      child: Column(
        children: [
          Expanded(
            // The tabs stay built under the editor, so they are as they were
            // when it closes.
            child: Stack(
              fit: StackFit.expand,
              children: [
                Offstage(
                  offstage: editing,
                  child: TickerMode(
                    enabled: !editing,
                    child: IndexedStack(
                      index: _index,
                      children: [
                        const NowScreen(),
                        LibraryScreen(
                          active: _index == 1 && !editing,
                          views: widget.views,
                        ),
                        const SectionsScreen(),
                        LevelsScreen(active: _index == 3 && !editing),
                        LightsScreen(
                          active: _index == 4 && !editing,
                          host: widget.stage,
                        ),
                      ],
                    ),
                  ),
                ),
                if (editing) const Positioned.fill(child: PlaylistEditor()),
              ],
            ),
          ),
          if (!editing) _tabBar(context, colors),
        ],
      ),
    );
  }

  /// The size every label is drawn at: [_labelSize], or as much less as it
  /// takes for the widest label to fit its tab, so that five tabs still read
  /// on a narrow phone set to large text. One size for all of them: a bar
  /// whose labels differ in size looks broken.
  double _fittedLabelSize(BuildContext context, double barWidth) {
    final scaler = MediaQuery.textScalerOf(context);
    final room = barWidth / _labels.length - 2 * _labelInset;
    var widest = 0.0;
    for (final label in _labels) {
      final painter = TextPainter(
        text: TextSpan(
          text: label,
          style: AppText.bodyStrong.copyWith(fontSize: _labelSize),
        ),
        textDirection: TextDirection.ltr,
        textScaler: scaler,
        maxLines: 1,
      )..layout();
      if (painter.width > widest) {
        widest = painter.width;
      }
      painter.dispose();
    }
    // A little under, so a rounding of the layout never clips a letter.
    return widest <= room ? _labelSize : _labelSize * room / widest * 0.98;
  }

  Widget _tabBar(BuildContext context, AppColors colors) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surface,
        border: Border(top: BorderSide(color: colors.line)),
      ),
      child: SafeArea(
        top: false,
        child: LayoutBuilder(
          builder: (context, constraints) {
            final size = _fittedLabelSize(context, constraints.maxWidth);
            return Row(
              children: [
                for (var i = 0; i < _labels.length; i++)
                  Expanded(
                    child: _Tab(
                      label: _labels[i],
                      fontSize: size,
                      selected: i == _index,
                      onTap: () => setState(() => _index = i),
                    ),
                  ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class _Tab extends StatelessWidget {
  const _Tab({
    required this.label,
    required this.fontSize,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final double fontSize;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Semantics(
      button: true,
      selected: selected,
      label: label,
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        onTap: onTap,
        child: Container(
          alignment: Alignment.center,
          constraints: const BoxConstraints(minHeight: 52),
          decoration: BoxDecoration(
            border: Border(
              top: BorderSide(
                color: selected ? colors.cyan : Colors.transparent,
                width: 2,
              ),
            ),
          ),
          child: Text(
            label,
            maxLines: 1,
            softWrap: false,
            style: AppText.bodyStrong.copyWith(
              fontSize: fontSize,
              color: selected ? colors.cyan : colors.dim,
            ),
          ),
        ),
      ),
    );
  }
}
