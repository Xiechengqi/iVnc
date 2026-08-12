ARG RUNTIME_IMAGE=ubuntu:24.04
ARG EXPECTED_RUNTIME_VERSION=24.04

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
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY extension ./extension
COPY web/ivnc ./web/ivnc
COPY --from=web-builder /build/web/ivnc/dist ./web/ivnc/dist

ARG IVNC_BUILD_GIT_COMMIT=unknown
ARG IVNC_BUILD_GIT_MESSAGE=unknown

RUN IVNC_REFRESH_MIAO=1 \
    IVNC_BUILD_GIT_COMMIT="$IVNC_BUILD_GIT_COMMIT" \
    IVNC_BUILD_GIT_MESSAGE="$IVNC_BUILD_GIT_MESSAGE" \
    cargo build --release --no-default-features --bin ivnc


FROM ${RUNTIME_IMAGE}

ARG AGENT_BROWSER_VERSION=0.29.0
ARG EXPECTED_RUNTIME_VERSION

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
    libegl1 \
    libgbm1 \
    libpixman-1-0 \
    libxkbcommon0 \
    libxcb1 \
    libx11-6 \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-ugly \
    gstreamer1.0-x \
    && curl -fL https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb -o /tmp/google-chrome-stable_current_amd64.deb \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends /tmp/google-chrome-stable_current_amd64.deb \
    && rm -f /tmp/google-chrome-stable_current_amd64.deb \
    && curl -fsSL "https://github.com/vercel-labs/agent-browser/releases/download/v${AGENT_BROWSER_VERSION}/agent-browser-linux-x64" -o /usr/local/bin/agent-browser \
    && chmod +x /usr/local/bin/agent-browser \
    && fc-cache -f \
    && grep -qx "VERSION_ID=\"${EXPECTED_RUNTIME_VERSION}\"" /etc/os-release \
    && google-chrome --version \
    && agent-browser --version \
    && gst-inspect-1.0 x264enc >/dev/null \
    && gst-inspect-1.0 rtph264pay >/dev/null \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ivnc /usr/local/bin/ivnc
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
COPY config.example.toml /etc/ivnc.toml

RUN chmod +x /usr/local/bin/docker-entrypoint.sh /usr/local/bin/ivnc \
    && ldd /usr/local/bin/ivnc > /tmp/ivnc-ldd \
    && cat /tmp/ivnc-ldd \
    && ! grep -q 'not found' /tmp/ivnc-ldd \
    && rm -f /tmp/ivnc-ldd

ENV XDG_RUNTIME_DIR=/run/user/0

EXPOSE 8008

HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:8008/health || exit 1

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh", "--config", "/etc/ivnc.toml", "--"]
CMD ["sleep", "infinity"]
