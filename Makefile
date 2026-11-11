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
ROOT_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
SVELTE_DIR := $(ROOT_DIR)/src/webui/svelte
WASM_CRATE_DIR := $(ROOT_DIR)/crates/mtrack-rules-wasm
WASM_GEN_DIR := $(SVELTE_DIR)/src/gen/rules
DOCS_DIR := $(ROOT_DIR)/docs

.PHONY: all setup setup-dev build gen-proto gen-app-proto gen-wasm install-ui build-ui build-stage-ui app-stage-assets build-rust test test-ui test-journeys test-systemd deb test-deb test-pi-image test-pi-image-boot lint lint-ui lint-rust lint-shell fmt fmt-ui fmt-rust check fmt-ui-check fmt-rust-check clean dev-ui docs docs-serve docs-clean

all: build

## Install system build dependencies
setup:
	./setup.sh

## Install system build + development dependencies
setup-dev:
	./setup.sh --dev

## Build everything
build: build-ui build-rust

## The player sources the rules crate compiles into the web UI's WebAssembly
## module: read off the crate's `#[path]` includes (each a `../`-prefixed path
## under the player's src/, in lib.rs and its lighting module), so the list
## lives in the crate alone; and the grammar its parser derives from.
WASM_SOURCES := $(shell grep -ho '\[path = "[^"]*"\]' $(WASM_CRATE_DIR)/src/*.rs $(WASM_CRATE_DIR)/src/lighting/*.rs | sed 's|.*"\(\.\./\)*\(src/[^"]*\)"\]|$(ROOT_DIR)/\2|') $(ROOT_DIR)/src/lighting/grammar.pest

## Compute a hash of all frontend inputs (source, proto, lockfile, the rules crate and the
## player sources it compiles).
# src/gen is generated from src/proto and the rules crate (hashed here already) and gitignored,
# so it is left out: a checkout that has never run buf would otherwise never match a stamp
# built elsewhere.
FRONTEND_HASH := $(shell find $(SVELTE_DIR)/src $(ROOT_DIR)/src/proto $(WASM_CRATE_DIR) $(WASM_SOURCES) -type f -not -path '$(SVELTE_DIR)/src/gen/*' 2>/dev/null | sort | xargs cat 2>/dev/null | git hash-object --stdin)
FRONTEND_STAMP := $(SVELTE_DIR)/dist/.build-stamp

## Install frontend dependencies
install-ui:
	cd $(SVELTE_DIR) && npm ci

## Generate protobuf TypeScript bindings
gen-proto:
	cd $(SVELTE_DIR) && npx @bufbuild/buf generate

## Generate the phone app's protobuf Dart messages into app/lib/gen (gitignored, like the
## TypeScript bindings). Needs protoc 3.15 or newer on PATH and the Flutter SDK; the Dart
## plugin is the protoc_plugin dev dependency of app/, so its version is the one in
## app/pubspec.lock. Messages only: the app speaks gRPC's messages over Bluetooth, not gRPC.
gen-app-proto:
	cd $(ROOT_DIR)/app && flutter pub get
	rm -rf $(ROOT_DIR)/app/lib/gen
	mkdir -p $(ROOT_DIR)/app/lib/gen
	@# protoc finds its plugin by path; a throwaway wrapper runs the pinned one from app/.
	plugin_dir=$$(mktemp -d) && \
		printf '#!/bin/sh\ncd "%s" && exec dart run protoc_plugin\n' "$(ROOT_DIR)/app" > "$$plugin_dir/protoc-gen-dart" && \
		chmod +x "$$plugin_dir/protoc-gen-dart" && \
		protoc -I $(ROOT_DIR)/src/proto --plugin=protoc-gen-dart="$$plugin_dir/protoc-gen-dart" \
			--dart_out=$(ROOT_DIR)/app/lib/gen ble/v1/ble.proto player/v1/player.proto; \
		status=$$?; rm -rf "$$plugin_dir"; exit $$status

## Compile the player's rules (beat grid, tempo map, metronome) to WebAssembly for the
## web UI, with their TypeScript types. Generated into src/gen like the proto bindings.
## wasm-pack uses the wasm-bindgen CLI on PATH (pinned to the crate's wasm-bindgen; setup.sh
## and CI install it) and the module is optimised with the pinned wasm-opt
## (scripts/install-binaryen.sh), so nothing is downloaded at build time.
gen-wasm:
	cd $(WASM_CRATE_DIR) && wasm-pack build --release --target web --out-dir $(WASM_GEN_DIR) --out-name mtrack_rules
	wasm-opt -Os -o $(WASM_GEN_DIR)/mtrack_rules_bg.wasm $(WASM_GEN_DIR)/mtrack_rules_bg.wasm

## Build the Svelte frontend (skips if sources unchanged)
build-ui:
	@if [ -f "$(FRONTEND_STAMP)" ] && [ "$$(cat "$(FRONTEND_STAMP)")" = "$(FRONTEND_HASH)" ]; then \
		echo "Frontend up to date, skipping build."; \
	else \
		$(MAKE) install-ui gen-proto gen-wasm && \
		cd $(SVELTE_DIR) && MTRACK_UI_BUILD=$(FRONTEND_HASH) npm run build && \
		echo "$(FRONTEND_HASH)" > "$(FRONTEND_STAMP)"; \
	fi

## Build the stage-only page (the phone app's Lights tab: the web UI's stage plot, fed the
## gRPC stage stream by its host; src/stage-main.ts) into dist-stage/. Skips if its inputs are
## unchanged: the frontend's, and the page's own two files outside src/.
STAGE_INPUTS := $(SVELTE_DIR)/stage.html $(SVELTE_DIR)/vite.stage.config.ts
STAGE_HASH := $(shell (echo $(FRONTEND_HASH); cat $(STAGE_INPUTS) 2>/dev/null) | git hash-object --stdin)
STAGE_STAMP := $(SVELTE_DIR)/dist-stage/.build-stamp

build-stage-ui:
	@if [ -f "$(STAGE_STAMP)" ] && [ "$$(cat "$(STAGE_STAMP)")" = "$(STAGE_HASH)" ]; then \
		echo "Stage page up to date, skipping build."; \
	else \
		$(MAKE) install-ui gen-proto gen-wasm && \
		cd $(SVELTE_DIR) && npm run build:stage && \
		echo "$(STAGE_HASH)" > "$(STAGE_STAMP)"; \
	fi

## Put the stage-only page among the phone app's assets (app/assets/stage, gitignored), where
## the app's builds take it from: the Lights tab shows it in a web view. `flutter analyze` and
## `flutter test` do not need it; `flutter build` and `flutter run` ship whatever is there.
APP_STAGE_DIR := $(ROOT_DIR)/app/assets/stage

app-stage-assets: build-stage-ui
	@# Everything but the .gitignore files that keep the two directories in a checkout.
	find "$(APP_STAGE_DIR)" -type f ! -name .gitignore -delete
	cp "$(SVELTE_DIR)/dist-stage/stage.html" "$(APP_STAGE_DIR)/"
	cp "$(SVELTE_DIR)"/dist-stage/assets/* "$(APP_STAGE_DIR)/assets/"

## Build the Rust binary
build-rust:
	cargo build --release --manifest-path $(ROOT_DIR)/Cargo.toml

## Run all tests (the player's, the harness's and the rules crate's equality test)
test: test-ui
	cargo test --workspace --manifest-path $(ROOT_DIR)/Cargo.toml

## Run Playwright UI tests (mock server)
test-ui:
	cd $(SVELTE_DIR) && npx playwright test --project=mock

## Run the Lighting-area journeys against the real binary: builds the UI and
## a debug mtrack (which serves the UI from dist/ on disk), then starts one
## mtrack per test on a throwaway project. MTRACK_BIN overrides the binary.
test-journeys: build-ui
	cargo build --bin mtrack --manifest-path $(ROOT_DIR)/Cargo.toml
	cd $(SVELTE_DIR) && npx playwright test --config=playwright.journeys.config.ts

## Build the Debian package (expects a binary at target/release/mtrack)
deb:
	cargo deb --no-build --no-strip

## Directory the package test builds in. Deliberately not the normal target/:
## the test stages a debug binary where cargo-deb expects a release one, and
## doing that under target/release would leave a debug binary that cargo, which
## fingerprints against its own database rather than the file, may then decline
## to rebuild -- handing someone a debug binary labelled release.
DEB_TEST_TARGET_DIR := $(ROOT_DIR)/target/deb-test

## Test the Debian package: install it and check the maintainer scripts
##
## Destructive -- installs, reinstalls and purges mtrack on this machine and
## creates the mtrack system user. Meant for CI and throwaway containers.
test-deb: build-ui
	@command -v cargo-deb >/dev/null || { echo "cargo-deb not found: cargo install cargo-deb"; exit 1; }
	@# A debug binary is fine here: nothing under test depends on optimisation,
	@# only on the binary being a real mtrack that can render its own unit.
	CARGO_TARGET_DIR=$(DEB_TEST_TARGET_DIR) cargo build --bin mtrack --manifest-path $(ROOT_DIR)/Cargo.toml
	mkdir -p $(DEB_TEST_TARGET_DIR)/release
	install -m 755 $(DEB_TEST_TARGET_DIR)/debug/mtrack $(DEB_TEST_TARGET_DIR)/release/mtrack
	CARGO_TARGET_DIR=$(DEB_TEST_TARGET_DIR) cargo deb --no-build --no-strip --fast \
		--output $(DEB_TEST_TARGET_DIR)/debian/
	$(ROOT_DIR)/tests/packaging/test.sh $$(ls -t $(DEB_TEST_TARGET_DIR)/debian/mtrack_*.deb | head -1)

## Run systemd integration test (requires Docker)
test-systemd:
	DOCKER_BUILDKIT=1 docker build -f $(ROOT_DIR)/tests/systemd/Dockerfile -t mtrack-systemd-test $(ROOT_DIR)
	@# Start container with systemd as PID 1, run tests via exec, then stop.
	@cid=$$(docker run -d --rm --privileged mtrack-systemd-test) && \
	  cleanup() { docker stop $$cid >/dev/null 2>&1 || true; }; \
	  trap cleanup EXIT; \
	  echo "Waiting for systemd to boot..."; \
	  for i in $$(seq 1 30); do \
	    docker exec $$cid systemctl is-system-running 2>/dev/null | grep -qE 'running|degraded' && break; \
	    sleep 1; \
	  done; \
	  docker exec $$cid /test.sh

## Inspect a built Raspberry Pi image
##
## Mounts the image's root filesystem and checks what the pi-gen stage should
## have left there. Needs root (or sudo) to mount. Does not boot anything --
## whether the card actually comes up still needs a Pi.
##
##   make test-pi-image IMAGE=path/to/mtrack-x.y.z-raspberrypi-arm64.img.xz
test-pi-image:
	@test -n "$(IMAGE)" || { echo "set IMAGE=<path to .img or .img.xz>"; exit 1; }
	$(ROOT_DIR)/tests/pi-image/test.sh "$(IMAGE)"

## Boot a built Raspberry Pi image's root filesystem in a container and check
## the player comes up. Needs an arm64 host, systemd-container and root.
##   make test-pi-image-boot IMAGE=path/to/mtrack-x.y.z-raspberrypi-arm64.img.xz
test-pi-image-boot:
	@test -n "$(IMAGE)" || { echo "set IMAGE=<path to .img or .img.xz>"; exit 1; }
	$(ROOT_DIR)/tests/pi-image/boot.sh "$(IMAGE)"

## Lint everything
lint: lint-ui lint-rust lint-shell

## Lint the Svelte frontend
lint-ui:
	cd $(SVELTE_DIR) && npm run lint && npm run check

## Lint shell scripts
lint-shell:
	@command -v shellcheck >/dev/null || { \
		echo "shellcheck not found -- see https://www.shellcheck.net/ for install options"; \
		exit 1; \
	}
	@# Discovered rather than listed, so a script added later is linted without
	@# anyone remembering to add it here. Plenty of shell under packaging/ carries
	@# no extension and so is missed by the *.sh glob -- the Debian maintainer
	@# scripts, pi-gen's sourced config and EXPORT_IMAGE. What is excluded is the
	@# data: the conffile shipped as /etc/default, pi-gen's package lists,
	@# which are fed to apt a line at a time, and systemd units.
	shellcheck $$(find $(ROOT_DIR) -name '*.sh' \
		-not -path '*/node_modules/*' -not -path '*/target/*' -not -path '*/.git/*' \
		| sort) \
		$$(find $(ROOT_DIR)/packaging -type f \
			! -name '*.sh' ! -name '*.md' ! -name default \
			! -name '*-packages' ! -name '*-packages-nr' ! -name '*.service' \
			| sort)

## Lint the Rust code
lint-rust:
	cargo clippy --manifest-path $(ROOT_DIR)/Cargo.toml --all --all-targets --all-features

## Format everything
fmt: fmt-ui fmt-rust

## Format the Svelte frontend
fmt-ui:
	cd $(SVELTE_DIR) && npm run format

## Format the Rust code
fmt-rust:
	cargo fmt --manifest-path $(ROOT_DIR)/Cargo.toml

## Check formatting without writing
check: fmt-ui-check fmt-rust-check

## Check frontend formatting
fmt-ui-check:
	cd $(SVELTE_DIR) && npm run format:check

## Check Rust formatting
fmt-rust-check:
	cargo fmt --manifest-path $(ROOT_DIR)/Cargo.toml --check

## Start the Vite dev server
dev-ui:
	cd $(SVELTE_DIR) && npm run dev

## Build the documentation
docs:
	mdbook build $(DOCS_DIR)

## Serve docs locally with live reload
docs-serve:
	mdbook serve $(DOCS_DIR)

## Clean documentation build artifacts
docs-clean:
	rm -rf $(DOCS_DIR)/book

## Clean build artifacts
clean: docs-clean
	cargo clean --manifest-path $(ROOT_DIR)/Cargo.toml
	rm -rf $(SVELTE_DIR)/dist $(SVELTE_DIR)/dist-stage $(WASM_GEN_DIR)
	find "$(APP_STAGE_DIR)" -type f ! -name .gitignore -delete
