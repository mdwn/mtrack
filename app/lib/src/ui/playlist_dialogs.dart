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

import 'theme.dart';

/// Asks for a playlist's name. [accept] is given the name as typed and
/// returns what is wrong with it, or null to take it, which closes the
/// dialog. Returns true when it was taken.
Future<bool> askPlaylistName(
  BuildContext context, {
  required String title,
  required String action,
  required String? Function(String) accept,
  String initial = '',
}) async {
  final taken = await showDialog<bool>(
    context: context,
    builder: (_) => _NameDialog(
      title: title,
      action: action,
      accept: accept,
      initial: initial,
    ),
  );
  return taken ?? false;
}

class _NameDialog extends StatefulWidget {
  const _NameDialog({
    required this.title,
    required this.action,
    required this.accept,
    required this.initial,
  });

  final String title;
  final String action;
  final String? Function(String) accept;
  final String initial;

  @override
  State<_NameDialog> createState() => _NameDialogState();
}

class _NameDialogState extends State<_NameDialog> {
  late final TextEditingController _text = TextEditingController(
    text: widget.initial,
  );
  String? _problem;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _submit() {
    final problem = widget.accept(_text.text);
    if (problem == null) {
      Navigator.of(context).pop(true);
    } else {
      setState(() => _problem = problem);
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        controller: _text,
        autofocus: true,
        textInputAction: TextInputAction.done,
        textCapitalization: TextCapitalization.sentences,
        onSubmitted: (_) => _submit(),
        onChanged: (_) {
          if (_problem != null) {
            setState(() => _problem = null);
          }
        },
        decoration: InputDecoration(labelText: 'Name', errorText: _problem),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(false),
          child: const Text('Cancel'),
        ),
        TextButton(onPressed: _submit, child: Text(widget.action)),
      ],
    );
  }
}

/// Asks whether to throw away the changes made to a playlist, or with
/// [unchanged] said, to something else. True to discard them.
Future<bool> confirmDiscard(
  BuildContext context, {
  String unchanged = 'The playlist on the player has not been changed.',
}) async {
  final colors = context.colors;
  final discard = await showDialog<bool>(
    context: context,
    builder: (dialog) => AlertDialog(
      title: const Text('Discard your changes?'),
      content: Text(unchanged),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialog).pop(false),
          child: const Text('Keep editing'),
        ),
        TextButton(
          onPressed: () => Navigator.of(dialog).pop(true),
          child: Text('Discard', style: TextStyle(color: colors.red)),
        ),
      ],
    ),
  );
  return discard ?? false;
}

/// Asks whether to delete the playlist [name]. True to delete it.
Future<bool> confirmDelete(BuildContext context, String name) async {
  final colors = context.colors;
  final delete = await showDialog<bool>(
    context: context,
    builder: (dialog) => AlertDialog(
      title: Text('Delete $name?'),
      content: const Text(
        'The playlist is deleted from the player. The songs stay.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialog).pop(false),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () => Navigator.of(dialog).pop(true),
          child: Text('Delete', style: TextStyle(color: colors.red)),
        ),
      ],
    ),
  );
  return delete ?? false;
}
