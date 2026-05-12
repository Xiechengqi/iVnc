FROM node:20-bookworm-slim AS web-builder

WORKDIR /build/web/ivnc

COPY web/ivnc/package.json web/ivnc/package-lock.json ./
RUN npm ci

COPY web/ivnc/ ./
RUN npm run build


FROM rust:1.88-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    cmake \
    curl \
    ca-certificates \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    libwayland-dev \
    libpixman-1-dev \
    libinput-dev \
    libudev-dev \
    libseat-dev \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    libsoup-3.0-dev \
    libjavascriptcoregtk-4.1-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY extension ./extension
COPY web/ivnc ./web/ivnc
COPY --from=web-builder /build/web/ivnc/dist ./web/ivnc/dist

RUN cargo build --release --no-default-features --features mcp --bin ivnc --bin ivnc-webview


FROM ubuntu:22.04

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    tzdata \
    bash \
    curl \
    ca-certificates \
    fontconfig \
    fonts-wqy-microhei \
    libgstreamer1.0-0 \
    libgstreamer-plugins-base1.0-0 \
    libpixman-1-0 \
    libxkbcommon0 \
    libxcb1 \
    libx11-6 \
    libgtk-3-0 \
    libwebkit2gtk-4.1-0 \
    libsoup-3.0-0 \
    libjavascriptcoregtk-4.1-0 \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-ugly \
    gstreamer1.0-x \
    && fc-cache -f \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ivnc /usr/local/bin/ivnc
COPY --from=builder /build/target/release/ivnc-webview /usr/local/bin/ivnc-webview
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY config.example.toml /etc/ivnc.toml

RUN chmod +x /usr/local/bin/docker-entrypoint.sh /usr/local/bin/ivnc /usr/local/bin/ivnc-webview

ENV XDG_RUNTIME_DIR=/run/user/0

EXPOSE 8008

HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:8008/health || exit 1

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh", "--config", "/etc/ivnc.toml", "--"]
CMD ["sleep", "infinity"]
