SHELL=/usr/bin/env bash

ORDERED_PACKAGES:=fil_actors_runtime \
                  fil_actor_account \
                  fil_actor_cron \
                  fil_actor_init \
                  fil_actor_market \
                  fil_actor_miner \
                  fil_actor_multisig \
                  fil_actor_paych \
                  fil_actor_power \
                  fil_actor_reward \
                  fil_actor_system \
                  fil_actor_verifreg \
                  fil_builtin_actors_bundle

# How much to "bump" the version by on release.
BUMP ?= patch
VERSION ?= $(error VERSION environment variable must be set)

# Run cargo check
check: deps-build
	cargo check --workspace --tests --benches --lib --bins --examples

# Ensure we have the build dependencies
deps-build:
	rustup target add wasm32-unknown-unknown

# Print out the current "bundle" version.
version: deps-release
	@cargo metadata -q --format-version=1 --no-deps | jq -r '.packages[] | select(.name == "fil_builtin_actors_bundle") | .version'

# Run cargo test
test: deps-build
	cargo test --workspace

# Release a new version. Specify the version "bump" with BUMP
bump-version: check-clean deps-release check test
	echo "$(ORDERED_PACKAGES)" | xargs -n1 cargo set-version --bump $(BUMP) -p
	cargo update --workspace
	@echo "Bumped actors to version $$($(MAKE) --quiet version)"

set-version: check-clean deps-release check test
	echo "$(ORDERED_PACKAGES)" | xargs -n1 cargo set-version $(VERSION) -p
	cargo update --workspace
	@echo "Set actors to version $(VERSION)"

# Publish the current version to crates.io
publish:
	for pkg in "$(ORDERED_PACKAGES)"; do cargo publish -p "$$pkg" && sleep 5; done

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
	@which jq >/dev/null 2>&1 || { \
		echo "Please install jq"; \
		exit 1; \
	}
	@which cargo-set-version >/dev/null 2>&1 || { \
		echo "Please install cargo-edit: 'cargo install cargo-edit'."; \
		exit 1; \
	}

.PHONY: check check-clean deps deps-release deps-release test publish bump-version set-version bundle
