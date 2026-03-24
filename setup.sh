#!/usr/bin/env bash
# Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, version 3.
#
# This program is distributed in the hope that it will be useful, but WITHOUT
# ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
# FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License along with
# this program. If not, see <https://www.gnu.org/licenses/>.
#
# setup.sh — Install system dependencies for building mtrack.
#
# Usage:
#   ./setup.sh          Install build dependencies only
#   ./setup.sh --dev    Install build + development dependencies
#   ./setup.sh --yes    Skip confirmation prompts

set -euo pipefail

DEV=false
YES=false

for arg in "$@"; do
  case "$arg" in
    --dev) DEV=true ;;
    --yes|-y) YES=true ;;
    --help|-h)
      echo "Usage: $0 [--dev] [--yes]"
      echo ""
      echo "  --dev   Install development tools (buf, cargo-tarpaulin, licensure, mdbook)"
      echo "  --yes   Skip confirmation prompts"
      exit 0
      ;;
    *)
      echo "Unknown option: $arg"
      echo "Run '$0 --help' for usage."
      exit 1
      ;;
  esac
done

confirm() {
  if [ "$YES" = true ]; then
    return 0
  fi
  echo ""
  echo "$1"
  read -rp "Continue? [y/N] " response
  case "$response" in
    [yY][eE][sS]|[yY]) return 0 ;;
    *) echo "Aborted."; exit 1 ;;
  esac
}

# Detect OS and package manager.
detect_os() {
  if [ -f /etc/os-release ]; then
    # shellcheck source=/dev/null
    . /etc/os-release
    case "$ID" in
      ubuntu|debian|pop|linuxmint|raspbian) echo "debian"; return ;;
      fedora|rhel|centos|rocky|alma) echo "fedora"; return ;;
      arch|manjaro|endeavouros) echo "arch"; return ;;
    esac
    # Fall back to ID_LIKE for derivative distros.
    case "${ID_LIKE-}" in
      *debian*|*ubuntu*) echo "debian"; return ;;
      *fedora*|*rhel*)   echo "fedora"; return ;;
      *arch*)            echo "arch"; return ;;
    esac
    echo "unknown"
  elif [ "$(uname)" = "Darwin" ]; then
    echo "macos"
  else
    echo "unknown"
  fi
}

OS=$(detect_os)

echo "Detected platform: $OS"

# --- Build dependencies ---

install_build_deps_debian() {
  local pkgs=(
    build-essential
    pkg-config
    libasound2-dev
    libudev-dev
    libssl-dev
    protobuf-compiler
  )
  confirm "Will install (apt): ${pkgs[*]}"
  sudo apt-get update
  sudo apt-get install -y "${pkgs[@]}"
}

install_build_deps_fedora() {
  local pkgs=(
    gcc
    pkg-config
    alsa-lib-devel
    systemd-devel
    openssl-devel
    protobuf-compiler
  )
  confirm "Will install (dnf): ${pkgs[*]}"
  sudo dnf install -y "${pkgs[@]}"
}

install_build_deps_arch() {
  local pkgs=(
    base-devel
    pkg-config
    alsa-lib
    systemd-libs
    openssl
    protobuf
  )
  confirm "Will install (pacman): ${pkgs[*]}"
  sudo pacman -S --needed --noconfirm "${pkgs[@]}"
}

install_build_deps_macos() {
  if ! command -v brew &>/dev/null; then
    echo "Error: Homebrew is required on macOS. Install it from https://brew.sh"
    exit 1
  fi
  local pkgs=(
    pkg-config
    openssl
    protobuf
  )
  confirm "Will install (brew): ${pkgs[*]}"
  brew install "${pkgs[@]}"
}

# --- Dev dependencies ---

install_dev_deps() {
  echo ""
  echo "Installing development tools..."

  # Node.js (needed for frontend + buf)
  if ! command -v node &>/dev/null; then
    echo "Node.js not found. Please install Node.js 22+ (https://nodejs.org or via your package manager)."
  else
    echo "Node.js $(node --version) found."
  fi

  # buf (protobuf code generation for frontend)
  if ! command -v buf &>/dev/null; then
    confirm "Will install buf via npm (global)"
    npm install -g @bufbuild/buf
  else
    echo "buf $(buf --version) found."
  fi

  # Cargo tools
  local cargo_tools=()

  if ! command -v cargo-tarpaulin &>/dev/null; then
    cargo_tools+=(cargo-tarpaulin)
  else
    echo "cargo-tarpaulin found."
  fi

  if ! command -v licensure &>/dev/null; then
    cargo_tools+=(licensure)
  else
    echo "licensure found."
  fi

  if ! command -v mdbook &>/dev/null; then
    cargo_tools+=(mdbook)
  else
    echo "mdbook found."
  fi

  if [ ${#cargo_tools[@]} -gt 0 ]; then
    confirm "Will run: cargo install ${cargo_tools[*]}"
    cargo install "${cargo_tools[@]}"
  fi
}

# --- Rust toolchain ---

check_rust() {
  if ! command -v rustup &>/dev/null; then
    echo ""
    echo "Rust toolchain not found. Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "Then re-run this script."
    exit 1
  fi

  echo "Rust toolchain found: $(rustc --version)"

  # Ensure clippy and rustfmt are installed.
  rustup component add clippy rustfmt 2>/dev/null || true
}

# --- Main ---

case "$OS" in
  debian)  install_build_deps_debian ;;
  fedora)  install_build_deps_fedora ;;
  arch)    install_build_deps_arch ;;
  macos)   install_build_deps_macos ;;
  *)
    echo "Unsupported platform. Please install the following dependencies manually:"
    echo "  - C compiler and linker"
    echo "  - pkg-config"
    echo "  - ALSA development headers (Linux only)"
    echo "  - libudev development headers (Linux only)"
    echo "  - OpenSSL development headers"
    echo "  - protobuf compiler (protoc)"
    exit 1
    ;;
esac

check_rust

if [ "$DEV" = true ]; then
  install_dev_deps
fi

echo ""
echo "Setup complete! Run 'make build' to build mtrack."
