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
DOCS_DIR := $(ROOT_DIR)/docs

.PHONY: all build gen-proto build-ui build-rust test test-ui lint lint-ui lint-rust fmt fmt-ui fmt-rust check clean dev-ui docs docs-serve docs-clean

all: build

## Build everything
build: build-ui build-rust

## Generate protobuf TypeScript bindings
gen-proto:
	cd $(SVELTE_DIR) && buf generate

## Build the Svelte frontend (generates protobuf bindings first)
build-ui: gen-proto
	cd $(SVELTE_DIR) && npm run build

## Build the Rust binary
build-rust:
	cargo build --release --manifest-path $(ROOT_DIR)/Cargo.toml

## Run all tests
test: test-ui
	cargo test --manifest-path $(ROOT_DIR)/Cargo.toml

## Run Playwright UI tests (mock server)
test-ui:
	cd $(SVELTE_DIR) && npx playwright test --project=mock

## Lint everything
lint: lint-ui lint-rust

## Lint the Svelte frontend
lint-ui:
	cd $(SVELTE_DIR) && npm run lint && npm run check

## Lint the Rust code
lint-rust:
	cargo clippy --manifest-path $(ROOT_DIR)/Cargo.toml

## Format everything
fmt: fmt-ui fmt-rust

## Format the Svelte frontend
fmt-ui:
	cd $(SVELTE_DIR) && npm run format

## Format the Rust code
fmt-rust:
	cargo fmt --manifest-path $(ROOT_DIR)/Cargo.toml

## Check formatting without writing
check:
	cd $(SVELTE_DIR) && npm run format:check
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
	rm -rf $(SVELTE_DIR)/dist
