FROM rust:1.81.0-bookworm@sha256:7b7f7ae5e49819e708369d49925360bde2af4f1962842e75a14af17342f08262

# Install dependencies
RUN apt-get update && apt-get install --no-install-recommends -y build-essential clang

WORKDIR /usr/src/builtin-actors

COPY . .

# Grab the correct toolchain
RUN make deps-build

ENTRYPOINT ["/bin/bash", "-c"]
