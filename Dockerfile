FROM rust:1.81.0-bookworm@sha256:7b7f7ae5e49819e708369d49925360bde2af4f1962842e75a14af17342f08262

ARG USER_ID
ARG GROUP_ID

RUN if ! getent group ${GROUP_ID} >/dev/null; then \
        groupadd -g ${GROUP_ID} user; \
    fi && \
    if ! id -u ${USER_ID} >/dev/null 2>&1; then \
        useradd -m -u ${USER_ID} -g ${GROUP_ID} user; \
    else \
        echo "User ${USER_ID} already exists"; \
    fi

USER user

WORKDIR /usr/src/builtin-actors

# Install the compiler. Unfortunately, the rust docker container doesn't actually contain the rust
# compiler...
COPY ./rust-toolchain.toml .
RUN rustup toolchain install $(cat rust-toolchain.toml | grep channel | cut -d '"' -f 2) && \
    rustup component add rustfmt clippy && \
    rustup show

USER root

# Then checkout a clean copy of the repo.
RUN --mount=type=bind,rw,target=/tmp/repo \
    echo "Building $(git -C /tmp/repo rev-parse HEAD)" && \
    git --git-dir /tmp/repo/.git --work-tree . checkout -f HEAD

USER user

ENTRYPOINT ["/bin/bash", "-c"]