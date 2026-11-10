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

import 'levels_screen.dart';
import 'now_screen.dart';
import 'playlist_editor.dart';
import 'playlist_screen.dart';
import 'sections_screen.dart';
import 'session_scope.dart';
import 'theme.dart';

/// The four tabs and the bar to switch between them. Each tab keeps its state
/// while another is showing.
class HomeShell extends StatefulWidget {
  /// The tabs.
  const HomeShell({super.key});

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  static const List<String> _labels = ['Now', 'Playlist', 'Sections', 'Levels'];

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
                        PlaylistScreen(active: _index == 1 && !editing),
                        const SectionsScreen(),
                        LevelsScreen(active: _index == 3 && !editing),
                      ],
                    ),
                  ),
                ),
                if (editing) const Positioned.fill(child: PlaylistEditor()),
              ],
            ),
          ),
          if (!editing) _tabBar(colors),
        ],
      ),
    );
  }

  Widget _tabBar(AppColors colors) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surface,
        border: Border(top: BorderSide(color: colors.line)),
      ),
      child: SafeArea(
        top: false,
        child: Row(
          children: [
            for (var i = 0; i < _labels.length; i++)
              Expanded(
                child: _Tab(
                  label: _labels[i],
                  selected: i == _index,
                  onTap: () => setState(() => _index = i),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Tab extends StatelessWidget {
  const _Tab({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
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
            style: AppText.bodyStrong.copyWith(
              fontSize: 12.5,
              color: selected ? colors.cyan : colors.dim,
            ),
          ),
        ),
      ),
    );
  }
}
