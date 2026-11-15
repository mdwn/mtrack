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
import 'dart:ffi';
import 'dart:io';

/// The key of an address in [interfacePrefixes]: the interface's name and
/// the address, as text.
String prefixKey(String interface, String address) => '$interface $address';

/// The number of leading one bits of [mask], a netmask as a 32-bit number
/// (255.255.255.0 is 0xFFFFFF00), or null when the mask is not a run of ones
/// followed by a run of zeros (or is not 32 bits).
int? prefixLengthOfMask(int mask) {
  if (mask < 0 || mask > 0xFFFFFFFF) {
    return null;
  }
  var ones = 0;
  var bit = 31;
  while (bit >= 0 && (mask >> bit) & 1 == 1) {
    ones++;
    bit--;
  }
  // Everything after the ones must be zeros.
  final rest = ones == 32 ? 0 : mask & ((1 << (32 - ones)) - 1);
  return rest == 0 ? ones : null;
}

/// The prefix length of each IPv4 address of the phone, by [prefixKey],
/// from the C library's `getifaddrs`: `dart:io` lists the addresses but not
/// their netmasks. Works on Android (API 24 and up), iOS, macOS and Linux
/// alike. Anything it cannot read is left out, and any failure at all gives
/// an empty map: never an exception.
Map<String, int> interfacePrefixes() {
  if (!(Platform.isAndroid ||
      Platform.isLinux ||
      Platform.isIOS ||
      Platform.isMacOS)) {
    return const {};
  }
  try {
    return _read();
  } on Object {
    return const {};
  }
}

// `struct ifaddrs` starts { next*, name*, flags (unsigned int, padded to a
// pointer), addr*, netmask* } on Linux, Android and Darwin alike, so the
// fields are at whole pointers from its start.
final int _word = sizeOf<IntPtr>();

// AF_INET is 2 everywhere. On Linux and Android a sockaddr's family is a
// 16-bit number at offset 0; on Darwin the first byte is its length and the
// family a byte at offset 1. A sockaddr_in's four address bytes are at 4.
const int _afInet = 2;
const int _addressOffset = 4;

bool get _darwin => Platform.isIOS || Platform.isMacOS;

Map<String, int> _read() {
  final libc = DynamicLibrary.process();
  final getifaddrs = libc
      .lookupFunction<
        Int32 Function(Pointer<IntPtr>),
        int Function(Pointer<IntPtr>)
      >('getifaddrs');
  final freeifaddrs = libc
      .lookupFunction<Void Function(IntPtr), void Function(int)>('freeifaddrs');
  final malloc = libc
      .lookupFunction<
        Pointer<Void> Function(IntPtr),
        Pointer<Void> Function(int)
      >('malloc');
  final free = libc
      .lookupFunction<
        Void Function(Pointer<Void>),
        void Function(Pointer<Void>)
      >('free');

  final head = malloc(_word).cast<IntPtr>();
  if (head.address == 0) {
    return const {};
  }
  try {
    head.value = 0;
    if (getifaddrs(head) != 0) {
      return const {};
    }
    final first = head.value;
    try {
      final prefixes = <String, int>{};
      var node = first;
      // A list is never this long; a loop in it would be a broken libc.
      for (var guard = 0; node != 0 && guard < 1024; guard++) {
        final entry = _entry(node);
        if (entry != null) {
          prefixes.putIfAbsent(entry.$1, () => entry.$2);
        }
        node = _pointerAt(node, 0);
      }
      return prefixes;
    } finally {
      freeifaddrs(first);
    }
  } finally {
    free(head.cast());
  }
}

int _pointerAt(int base, int offset) =>
    Pointer<IntPtr>.fromAddress(base + offset).value;

// The key and prefix of one `struct ifaddrs` at [node], or null when it is
// not an IPv4 address with a contiguous netmask.
(String, int)? _entry(int node) {
  final namePtr = _pointerAt(node, _word);
  final addr = _pointerAt(node, 3 * _word);
  final mask = _pointerAt(node, 4 * _word);
  if (namePtr == 0 || addr == 0 || mask == 0) {
    return null;
  }
  final address = Pointer<Uint8>.fromAddress(addr);
  final family = _darwin ? address[1] : address[0] | (address[1] << 8);
  if (family != _afInet) {
    return null;
  }
  final netmask = Pointer<Uint8>.fromAddress(mask);
  // A Darwin netmask may be cut short: what is past its length is zeros.
  final length = _darwin ? netmask[0] : 16;
  var bits = 0;
  for (var i = 0; i < 4; i++) {
    final at = _addressOffset + i;
    bits = (bits << 8) | (at < length ? netmask[at] : 0);
  }
  final prefix = prefixLengthOfMask(bits);
  if (prefix == null) {
    return null;
  }
  final name = _cString(Pointer<Uint8>.fromAddress(namePtr));
  final text =
      '${address[_addressOffset]}.${address[_addressOffset + 1]}.'
      '${address[_addressOffset + 2]}.${address[_addressOffset + 3]}';
  return (prefixKey(name, text), prefix);
}

// A NUL-terminated string, read byte by byte (names are 16 bytes at most).
String _cString(Pointer<Uint8> start) {
  final bytes = <int>[];
  for (var i = 0; i < 64 && start[i] != 0; i++) {
    bytes.add(start[i]);
  }
  return String.fromCharCodes(bytes);
}
