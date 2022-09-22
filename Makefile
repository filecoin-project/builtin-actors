SHELL=/usr/bin/env bash

# How much to "bump" the version by on release.
BUMP ?= patch
VERSION ?= $(error VERSION environment variable must be set)

# Run cargo fmt
rustfmt: deps-build
	cargo fmt --all --check

# NOTE: Check all targets, then check the build target specifically. Otherwise, it might build for
# testing but not otherwise due to feature resolution shenanigans.

# Run cargo check
check: deps-build
	cargo clippy --all --all-targets -- -D warnings
	cargo clippy --all -- -D warnings

# Ensure we have the build dependencies
deps-build:
	rustup target add wasm32-unknown-unknown
	rustup component add clippy rustfmt

# Run cargo test
test: deps-build
	cargo test --workspace

# Release a new version. Specify the version "bump" with BUMP
bump-version: check-clean deps-release check
	cargo set-version --workspace --bump $(BUMP) -p
	cargo update --workspace
	@echo "Bumped actors to version $$($(MAKE) --quiet version)"

set-version: check-clean deps-release check
	cargo set-version --workspace $(VERSION) -p
	cargo update --workspace
	@echo "Set actors to version $(VERSION)"

# Publish the current version to crates.io
publish:
	cargo workspaces publish --from-git

# Create a bundle in a deterministic location
bundle: deps-build
	cargo run -- -o output/builtin-actors.car

# Create all canonical network bundles
all-bundles: bundle-mainnet bundle-caterpillarnet bundle-butterflynet bundle-calibrationnet bundle-devnet bundle-testing bundle-testing

bundle-mainnet: deps-build
	BUILD_FIL_NETWORK=mainnet cargo run -- -o output/builtin-actors-mainnet.car

bundle-caterpillarnet: deps-build
	BUILD_FIL_NETWORK=caterpillarnet cargo run -- -o output/builtin-actors-caterpillarnet.car

bundle-butterflynet: deps-build
	BUILD_FIL_NETWORK=butterflynet cargo run -- -o output/builtin-actors-butterflynet.car

bundle-calibrationnet: deps-build
	BUILD_FIL_NETWORK=calibrationnet cargo run -- -o output/builtin-actors-calibrationnet.car

bundle-devnet: deps-build
	BUILD_FIL_NETWORK=devnet cargo run -- -o output/builtin-actors-devnet.car

bundle-testing: deps-build
	BUILD_FIL_NETWORK=testing cargo run -- -o output/builtin-actors-testing.car
	BUILD_FIL_NETWORK=testing-fake-proofs cargo run -- -o output/builtin-actors-testing-fake-proofs.car

.PHONY: all-bundles bundle-mainnet bundle-caterpillarnet bundle-butterflynet bundle-calibrationnet bundle-devnet bundle-testing

# Check if the working tree is clean.
check-clean:
	@git diff --quiet || { \
		echo "Working tree dirty, please commit any changes first."; \
		exit 1; \
	}

# Check if we have the required deps.
deps-release:
	@which cargo-set-version >/dev/null 2>&1 || { \
		echo "Please install cargo-edit: 'cargo install cargo-edit'."; \
		exit 1; \
	}
	@which cargo-workspaces >/dev/null 2>&1 || { \
		echo "Please install cargo-workspaces: 'cargo install cargo-workspaces'."; \
		exit 1; \
	}

.PHONY: check check-clean deps deps-release deps-release test publish bump-version set-version bundle
