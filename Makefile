SHELL=/usr/bin/env bash

# Run cargo fmt
rustfmt:
	cargo fmt --all --check

# NOTE: Check all targets, then check the build target specifically. Otherwise, it might build for
# testing but not otherwise due to feature resolution shenanigans.

# Run cargo check
check:
	cargo clippy --all --all-targets -- -D warnings
	cargo clippy --all -- -D warnings

# Run cargo test
test:
	cargo test --workspace

docker-builder:
	docker build . -t builtin-actors-builder

# Create a bundle in a deterministic location
bundle:
	cargo run -- -o output/builtin-actors.car

bundle-repro: docker-builder
	docker run --rm -e BUILD_FIL_NETWORK -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle"

# Create all canonical network bundles
all-bundles-repro: bundle-mainnet-repro bundle-caterpillarnet-repro bundle-butterflynet-repro bundle-calibrationnet-repro bundle-devnet-repro bundle-testing-repro

bundle-mainnet: deps-build
	BUILD_FIL_NETWORK=mainnet cargo run -- -o output/builtin-actors-mainnet.car

bundle-mainnet-repro: docker-builder
	docker run --rm -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle-mainnet"

bundle-caterpillarnet:
	BUILD_FIL_NETWORK=caterpillarnet cargo run -- -o output/builtin-actors-caterpillarnet.car

bundle-caterpillarnet-repro: docker-builder
	docker run --rm -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle-caterpillarnet"

bundle-butterflynet:
	BUILD_FIL_NETWORK=butterflynet cargo run -- -o output/builtin-actors-butterflynet.car

bundle-butterflynet-repro: docker-builder
	docker run --rm -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle-butterflynet"

bundle-calibrationnet:
	BUILD_FIL_NETWORK=calibrationnet cargo run -- -o output/builtin-actors-calibrationnet.car

bundle-calibrationnet-repro: docker-builder
	docker run --rm -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle-calibrationnet"

bundle-devnet:
	BUILD_FIL_NETWORK=devnet cargo run -- -o output/builtin-actors-devnet.car

bundle-devnet-repro: docker-builder
	docker run --rm -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle-devnet"

bundle-testing:
	BUILD_FIL_NETWORK=testing cargo run -- -o output/builtin-actors-testing.car
	BUILD_FIL_NETWORK=testing-fake-proofs cargo run -- -o output/builtin-actors-testing-fake-proofs.car

bundle-testing-repro: docker-builder
	docker run --rm -it -v `pwd`/output:/usr/src/builtin-actors/output builtin-actors-builder "make bundle-testing"

# Check if the working tree is clean.
check-clean:
	@git diff --quiet || { \
		echo "Working tree dirty, please commit any changes first."; \
		exit 1; \
	}

.PHONY: rustfmt check check-clean test bundle
.PHONY: all-bundles bundle-mainnet bundle-caterpillarnet bundle-butterflynet bundle-calibrationnet bundle-devnet bundle-testing bundle-mainnet-repro bundle-caterpillarnet-repro bundle-butterflynet-repro bundle-calibrationnet-repro bundle-devnet-repro bundle-testing-repro docker-builder
