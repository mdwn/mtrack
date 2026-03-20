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
    nodejs npm \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install buf for protobuf code generation.
RUN curl -sSL https://github.com/bufbuild/buf/releases/latest/download/buf-Linux-x86_64 -o /usr/local/bin/buf \
    && chmod +x /usr/local/bin/buf

WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY .git/ .git/
COPY src/ src/

# Build the Svelte frontend (embedded into the Rust binary).
RUN cd src/webui/svelte && npm ci && buf generate && npm run build

RUN cargo build --release

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

RUN mtrack systemd > /etc/systemd/system/mtrack.service
RUN echo 'MTRACK_PATH=/var/lib/mtrack' > /etc/default/mtrack

COPY tests/systemd/test.sh /test.sh
RUN chmod +x /test.sh

# systemd runs as PID 1. Tests are run via `docker exec`.
STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
