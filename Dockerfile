FROM rust:1-buster AS build-env

# Install dependencies
RUN apt-get update && apt-get install --no-install-recommends -y build-essential clang

WORKDIR /usr/src/builtin-actors

# Grab the correct toolchain
RUN rustup toolchain install nightly && rustup target add wasm32-unknown-unknown

ENTRYPOINT ["/bin/bash", "-c"]
