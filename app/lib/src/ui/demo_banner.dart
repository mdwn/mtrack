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

/// What the banner says while the demo is on.
const String demoBannerMessage = 'Demo: nothing here is a real player';

/// The icon that stands for the demo where a radio's would be.
const IconData demoIcon = Icons.science_outlined;

/// The slim line across the top of the app for as long as the demo is on:
/// what is shown is simulated, and the way out.
class DemoBanner extends StatelessWidget {
  /// A banner for [session], whose demo it leaves.
  const DemoBanner({required this.session, super.key});

  /// The session in the demo.
  final Session session;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Material(
      // Opaque, as it looks over the page: it stands outside the screens.
      color: Color.alphaBlend(colors.loopTint, colors.page),
      child: SafeArea(
        bottom: false,
        child: Padding(
          padding: const EdgeInsets.only(left: 18, right: 6),
          child: Row(
            children: [
              Icon(demoIcon, size: 15, color: colors.amber),
              const SizedBox(width: 10),
              Expanded(
                child: Semantics(
                  liveRegion: true,
                  child: Text(
                    demoBannerMessage,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: AppText.small.copyWith(color: colors.amber),
                  ),
                ),
              ),
              Semantics(
                button: true,
                label: 'Leave the demo',
                excludeSemantics: true,
                onTap: session.leaveDemo,
                child: InkWell(
                  borderRadius: BorderRadius.circular(14),
                  onTap: session.leaveDemo,
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(
                      minHeight: 48,
                      minWidth: 64,
                    ),
                    child: Center(
                      widthFactor: 1,
                      child: Padding(
                        padding: const EdgeInsets.symmetric(horizontal: 12),
                        child: Text(
                          'Leave',
                          style: AppText.bodyStrong.copyWith(
                            color: colors.amber,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
