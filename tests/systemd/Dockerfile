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
# Systemd integration test container.
#
# Stage 1: Build mtrack against the same glibc as the test image.
# Stage 2: Minimal systemd image with just the binary.
#
# Usage (via Makefile):
#   make test-systemd

# --- Build stage ---
FROM ubuntu:24.04 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl build-essential pkg-config libasound2-dev \
    protobuf-compiler libprotobuf-dev git \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Ubuntu 24.04 ships Node.js 18, which is too old for Vite 8 (requires
# Node 20.19+/22.12+). Install Node.js 22.x from NodeSource; this package
# bundles npm.
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# The web UI compiles the player's rules to WebAssembly (make gen-wasm), with
# the same pinned tools as CI and setup.sh: wasm-pack, the wasm-bindgen CLI at
# the rules crate's pinned version, and binaryen's wasm-opt.
RUN rustup target add wasm32-unknown-unknown \
    && cargo install --locked wasm-pack --version 0.15.1 \
    && cargo install --locked wasm-bindgen-cli --version 0.2.122
COPY scripts/install-binaryen.sh /tmp/install-binaryen.sh
RUN /tmp/install-binaryen.sh && rm /tmp/install-binaryen.sh

WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY .git/ .git/
COPY src/ src/
# Required to parse the manifest: Cargo.toml declares the mixer bench.
COPY benches/ benches/
# Likewise: Cargo.toml declares a workspace with the harness as a member, so its
# manifest must exist for the workspace to load. Only the manifest is copied --
# the harness is a hardware-verification tool with no place in this image, and
# the build below is scoped to mtrack so its sources are never needed.
COPY harness/Cargo.toml harness/Cargo.toml
# The rules crate is a workspace member too, and the web UI build compiles it
# to WebAssembly, so it is copied whole.
COPY crates/ crates/
COPY Makefile ./

# Build the Svelte frontend (embedded into the Rust binary) the way CI does:
# npm ci, the protobuf bindings, the WASM rules module, then the Vite build.
RUN make build-ui

RUN cargo build --release -p mtrack --bin mtrack

# --- Test stage ---
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y --no-install-recommends \
    systemd systemd-sysv dbus \
    libasound2t64 \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/mtrack /usr/local/bin/mtrack

RUN useradd --system --no-create-home --shell /usr/sbin/nologin mtrack \
    && usermod -aG audio mtrack \
    && mkdir -p /var/lib/mtrack \
    && chown mtrack:mtrack /var/lib/mtrack

# With the library path, so the generated unit carries ReadWritePaths and the
# specific permission hint — the form the deployment guide recommends, and the
# one that can fail under real systemd if the directive is malformed.
RUN mtrack systemd /var/lib/mtrack > /etc/systemd/system/mtrack.service
RUN echo 'MTRACK_PATH=/var/lib/mtrack' > /etc/default/mtrack

COPY tests/systemd/test.sh /test.sh
RUN chmod +x /test.sh

# systemd runs as PID 1. Tests are run via `docker exec`.
STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
