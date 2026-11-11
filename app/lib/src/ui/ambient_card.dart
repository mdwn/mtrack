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

import '../app/session.dart';
import 'theme.dart';

/// What the ambient light is doing, in a word or two; null when unknown.
String? ambientStateLabel(String? state) => switch (state) {
  'playing' => 'playing',
  'pending' => 'waiting',
  'off' => 'off',
  'unavailable' => 'no ambient show',
  _ => null,
};

const List<(String, String)> _ambientModes = [
  ('auto', 'Auto'),
  ('on', 'On'),
  ('off', 'Off'),
];

/// The ambient light's control: Auto, On or Off, and what the light is doing.
/// It sits on the page, at the top of the Lights tab.
class AmbientCard extends StatelessWidget {
  /// The control for [session]'s player, which must have an ambient mode.
  const AmbientCard({required this.session, super.key});

  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final state = ambientStateLabel(session.ambientState);
    final usable = session.isReady && !session.isBusy(ActionKind.setAmbient);
    return Container(
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      decoration: BoxDecoration(
        color: colors.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: colors.line.withAlpha(120)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            children: [
              Text(
                'Ambient light',
                style: AppText.bodyStrong.copyWith(color: colors.text),
              ),
              const SizedBox(width: 10),
              if (state != null)
                Expanded(
                  child: Text(
                    state,
                    textAlign: TextAlign.end,
                    style: AppText.small.copyWith(color: colors.muted),
                  ),
                ),
            ],
          ),
          const SizedBox(height: 10),
          Opacity(
            opacity: usable ? 1 : 0.5,
            child: Container(
              padding: const EdgeInsets.all(3),
              decoration: BoxDecoration(
                color: colors.raised,
                borderRadius: BorderRadius.circular(10),
              ),
              child: Row(
                children: [
                  for (final (mode, label) in _ambientModes)
                    Expanded(
                      child: _Segment(
                        label: label,
                        selected: session.ambientMode == mode,
                        onTap: usable
                            ? () => session.setAmbientMode(mode)
                            : null,
                      ),
                    ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Semantics(
      button: true,
      selected: selected,
      enabled: onTap != null,
      label: 'Ambient light $label',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: selected ? colors.cyan : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: onTap,
          child: Container(
            constraints: const BoxConstraints(minHeight: 48),
            alignment: Alignment.center,
            child: Text(
              label,
              style: AppText.bodyStrong.copyWith(
                color: selected ? colors.page : colors.muted,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
