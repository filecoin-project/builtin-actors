SHELL=/usr/bin/env bash

DOCKER := docker
DOCKER_IMAGE_NAME := builtin-actors-builder
DOCKER_RUN_OPTS := --rm -v $(PWD)/output:/output
DOCKER_PLATFORM := --platform linux/amd64

toolchain:
	rustup show active-toolchain || rustup toolchain install
.PHONY: toolchain

NEXTEST_VERSION ?= 0.9.106

install-nextest:
	@command -v cargo-nextest >/dev/null 2>&1 || cargo install cargo-nextest --version $(NEXTEST_VERSION) --locked
.PHONY: install-nextest

# Run cargo fmt
rustfmt: toolchain
	cargo fmt --all --check

# NOTE: Check all targets, then check the build target specifically. Otherwise, it might build for
# testing but not otherwise due to feature resolution shenanigans.

# Run cargo check
check: toolchain
	SKIP_BUNDLE=1 cargo clippy --all --all-targets -- -D warnings
	SKIP_BUNDLE=1 cargo clippy --all -- -D warnings

# NOTE: nextest doesn't run doctests https://github.com/nextest-rs/nextest/issues/16,
# enable once doc tests are added: `cargo test --doc`

# Run cargo test
test: toolchain install-nextest
	cargo nextest run --workspace --no-fail-fast

docker-builder:
	$(DOCKER) buildx build $(DOCKER_PLATFORM) . -t $(DOCKER_IMAGE_NAME); \

# Create a bundle in a deterministic location
bundle: toolchain
	cargo run -- -o output/builtin-actors.car

bundle-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) -e BUILD_FIL_NETWORK $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME)

# Create all canonical network bundles
all-bundles: bundle-mainnet bundle-caterpillarnet bundle-butterflynet bundle-calibrationnet bundle-devnet bundle-testing bundle-testing

all-bundles-repro: bundle-mainnet-repro bundle-caterpillarnet-repro bundle-butterflynet-repro bundle-calibrationnet-repro bundle-devnet-repro bundle-testing-repro

bundle-mainnet: toolchain
	BUILD_FIL_NETWORK=mainnet cargo run -- -o output/builtin-actors-mainnet.car

bundle-mainnet-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME) "mainnet"

bundle-caterpillarnet: toolchain
	BUILD_FIL_NETWORK=caterpillarnet cargo run -- -o output/builtin-actors-caterpillarnet.car

bundle-caterpillarnet-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME) "caterpillarnet"

bundle-butterflynet: toolchain
	BUILD_FIL_NETWORK=butterflynet cargo run -- -o output/builtin-actors-butterflynet.car

bundle-butterflynet-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME) "butterflynet"

bundle-calibrationnet: toolchain
	BUILD_FIL_NETWORK=calibrationnet cargo run -- -o output/builtin-actors-calibrationnet.car

bundle-calibrationnet-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME) "calibrationnet"

bundle-devnet: toolchain
	BUILD_FIL_NETWORK=devnet cargo run -- -o output/builtin-actors-devnet.car

bundle-devnet-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME) "devnet"

bundle-testing: toolchain
	BUILD_FIL_NETWORK=testing cargo run -- -o output/builtin-actors-testing.car
	BUILD_FIL_NETWORK=testing-fake-proofs cargo run -- -o output/builtin-actors-testing-fake-proofs.car

bundle-testing-repro: docker-builder
	$(DOCKER) run $(DOCKER_PLATFORM) $(DOCKER_RUN_OPTS) $(DOCKER_IMAGE_NAME) "testing"

# Check if the working tree is clean.
check-clean:
	@git diff --quiet || { \
		echo "Working tree dirty, please commit any changes first."; \
		exit 1; \
	}

.PHONY: rustfmt check check-clean test bundle
.PHONY: all-bundles bundle-mainnet bundle-caterpillarnet bundle-butterflynet bundle-calibrationnet \
	bundle-devnet bundle-testing all-bundles-repro bundle-mainnet-repro bundle-caterpillarnet-repro \
	bundle-butterflynet-repro bundle-calibrationnet-repro bundle-devnet-repro bundle-testing-repro \
	docker-builder
