FROM rust:1.81.0-bookworm@sha256:7b7f7ae5e49819e708369d49925360bde2af4f1962842e75a14af17342f08262

WORKDIR /usr/src/builtin-actors

# Install the compiler. Unfortunately, the rust docker container doesn't actually contain the rust
# compiler...
COPY ./rust-toolchain.toml .
RUN rustup show

COPY . .

ENTRYPOINT ["/bin/bash", "-c"]
