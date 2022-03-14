SHELL=/usr/bin/env bash

ORDERED_PACKAGES:=fil_actors_runtime \
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

publish:
	echo "$(ORDERED_PACKAGES)" | xargs -n1 cargo publish -p
.PHONY: publish
