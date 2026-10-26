# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS source
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    mkdir src && touch src/main.rs && cargo fetch --locked && rm -rf src
COPY src ./src
COPY assets ./assets
COPY web ./web

# The default build contains no desktop or graphics dependencies.
FROM source AS build-web
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked --no-default-features --features web \
    && install -m 755 target/release/trunq /usr/local/bin/trunq

# Optional native target for a local X11/Wayland display.
FROM source AS build-native
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libxkbcommon-dev libwayland-dev \
    && rm -rf /var/lib/apt/lists/*
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked && install -m 755 target/release/trunq /usr/local/bin/trunq

FROM debian:bookworm-slim AS native
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 \
    libx11-6 libxcursor1 libxrandr2 libxi6 libxinerama1 \
    libegl1 libegl-mesa0 libgl1 libglx-mesa0 libgl1-mesa-dri mesa-vulkan-drivers \
    dbus-x11 xdg-utils tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 trunq \
    && install -d -m 1777 /tmp/.X11-unix \
    && install -d -o trunq -g trunq /data /tmp/runtime-trunq \
    && chmod 700 /tmp/runtime-trunq
COPY --from=build-native /usr/local/bin/trunq /usr/local/bin/trunq
ENV TRUNQ_CONFIG_DIR=/data XDG_RUNTIME_DIR=/tmp/runtime-trunq
USER trunq
WORKDIR /home/trunq
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/trunq"]

# Browser-rendered HTML/CSS with a Rust PostgreSQL backend.
FROM debian:bookworm-slim AS web
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 curl tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 trunq \
    && install -d -o trunq -g trunq /data
COPY --from=build-web /usr/local/bin/trunq /usr/local/bin/trunq
ENV TRUNQ_CONFIG_DIR=/data TRUNQ_WEB_LISTEN=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD curl -fsS http://127.0.0.1:8080/healthz > /dev/null || exit 1
USER trunq
WORKDIR /home/trunq
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/trunq"]
