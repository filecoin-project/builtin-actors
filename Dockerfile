FROM rust:1.63.0-buster

# Install dependencies
RUN apt-get update && apt-get install --no-install-recommends -y build-essential clang

WORKDIR /usr/src/builtin-actors

COPY . .

# Grab the correct toolchain
RUN make deps-build

ENTRYPOINT ["/bin/bash", "-c"]
