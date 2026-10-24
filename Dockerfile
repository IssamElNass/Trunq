# syntax=docker/dockerfile:1
FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev libxkbcommon-dev libwayland-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    mkdir src && touch src/main.rs && cargo fetch --locked && rm -rf src
COPY src ./src
COPY assets ./assets
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked && install -m 755 target/release/trunq /usr/local/bin/trunq

# Optional native target for a local X11/Wayland display.
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
COPY --from=build /usr/local/bin/trunq /usr/local/bin/trunq
ENV TRUNQ_CONFIG_DIR=/data XDG_RUNTIME_DIR=/tmp/runtime-trunq
USER trunq
WORKDIR /home/trunq
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/trunq"]

# The same native client in a browser-accessible desktop (noVNC).
FROM native AS desktop
USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb x11vnc x11-utils openbox novnc websockify curl procps zenity \
    && rm -rf /var/lib/apt/lists/* \
    && printf '{"version":"%s"}\n' "$(dpkg-query -W -f='${Version}' novnc)" > /usr/share/novnc/package.json
# Fit the client inside the virtual screen, including window decorations.
RUN sed -i 's|<applications>|<applications><application title="Trunq"><maximized>yes</maximized></application>|' /etc/xdg/openbox/rc.xml
COPY --chmod=755 packaging/docker/entrypoint.sh /usr/local/bin/trunq-desktop
ENV DISPLAY=:99 WGPU_BACKEND=gl LIBGL_ALWAYS_SOFTWARE=1 \
    TRUNQ_SCREEN=1440x900x24
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s \
    CMD curl -fsS http://127.0.0.1:8080/vnc.html > /dev/null || exit 1
USER trunq
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/trunq-desktop"]
