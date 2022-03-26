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

# Print out the current "bundle" version.
version:
	@cargo metadata -q --format-version=1 --no-deps | jq -r '.packages[] | select(.name == "fil_builtin_actors_bundle") | .version'

# Run cargo check
check:
	cargo check --workspace --tests --benches --lib --bins --examples

# Run cargo test (checking first)
test: check
	cargo test --workspace

# Release a new version. Specify the version "bump" with BUMP
release: check_clean check_deps test
	echo "$(ORDERED_PACKAGES)" | xargs -n1 cargo set-version --bump $(BUMP) -p
	cargo update --workspace
	@echo "Bumped actors to version $$($(MAKE) --quiet version)"

# Publish the current version to crates.io
publish:
	echo "$(ORDERED_PACKAGES)" | xargs -n1 cargo publish -p

# Check if the working tree is clean.
check_clean:
	@git diff --quiet || { \
		echo "Working tree dirty, please commit any changes first."; \
		exit 1; \
	}

# Check if we have the required deps.
check_deps:
	@which jq >/dev/null 2>&1 || { \
		echo "Please install jq"; \
		exit 1; \
	}
	@which cargo-set-version >/dev/null 2>&1 || { \
		echo "Please install cargo-edit: 'cargo install cargo-edit'."; \
		exit 1; \
	}

.PHONY: check_clean check_deps test publish check
