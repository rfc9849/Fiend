# syntax=docker/dockerfile:1.7
ARG RUST_VERSION=1.100
ARG NODE_VERSION=22
ARG ALPINE_VERSION=3.22

FROM --platform=$BUILDPLATFORM node:${NODE_VERSION}-alpine AS web
WORKDIR /src/fiend-panel/web
COPY fiend-panel/web/package.json fiend-panel/web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY fiend-desktop/assets/icons/ /src/fiend-desktop/assets/icons/
COPY fiend-panel/web/ ./
RUN npm run build

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-trixie AS toolchain
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$(uname -m)" in \
        x86_64) arch=x86_64; sum=1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026 ;; \
        aarch64) arch=aarch64; sum=9e8d11661d4ae3bd57702a3832781e23ad151dde5798e16a5ccd503f65234ff8 ;; \
        *) echo "unsupported build host: $(uname -m)" >&2; exit 1 ;; \
    esac; \
    curl -fsSL --proto '=https' --tlsv1.2 "https://ziglang.org/download/0.17.0/zig-$arch-linux-0.17.0.tar.xz" -o /tmp/zig.tar.xz; \
    echo "$sum  /tmp/zig.tar.xz" | sha256sum -c -; \
    mkdir /opt/zig; \
    tar -xJf /tmp/zig.tar.xz -C /opt/zig --strip-components=1; \
    rm /tmp/zig.tar.xz
ENV PATH=/opt/zig:$PATH
RUN cargo install --locked cargo-zigbuild --version 0.24.0 \
    && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl

FROM toolchain AS rust
ARG TARGETARCH
WORKDIR /src
COPY . .
COPY --from=web /src/fiend-panel/web/build fiend-panel/web/build
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=fiend-registry-$TARGETARCH,sharing=locked \
    --mount=type=cache,target=/src/target,id=fiend-target-$TARGETARCH \
    set -eu; \
    case "$TARGETARCH" in \
        amd64) target=x86_64-unknown-linux-musl ;; \
        arm64) target=aarch64-unknown-linux-musl ;; \
        *) echo "unsupported target: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    cargo zigbuild --locked --release -p fiend-core -p fiend-panel --target "$target"; \
    mkdir /out; \
    cp "target/$target/release/fiend-core" "target/$target/release/fiend-panel" /out/

FROM --platform=$BUILDPLATFORM alpine:${ALPINE_VERSION} AS wireproxy
ARG TARGETARCH
WORKDIR /tmp/wireproxy
COPY docker/wireproxy.sha256 ./
RUN set -eu; \
    asset="wireproxy_linux_$TARGETARCH.tar.gz"; \
    wget -q "https://github.com/windtf/wireproxy/releases/download/v1.1.3/$asset"; \
    grep " $asset\$" wireproxy.sha256 | sha256sum -c -; \
    tar -xzf "$asset" wireproxy; \
    install -D -m 0755 wireproxy /out/wireproxy

FROM alpine:${ALPINE_VERSION}
RUN addgroup -S -g 10001 fiend \
    && adduser -S -D -H -u 10001 -G fiend -h /var/lib/fiend -s /sbin/nologin fiend
COPY --from=rust /out/ /usr/local/bin/
COPY --from=wireproxy /out/wireproxy /usr/local/bin/wireproxy
COPY docker/entrypoint.sh /usr/local/bin/fiend-entrypoint
RUN apk add --no-cache --virtual .caps libcap-utils \
    && setcap cap_net_bind_service,cap_net_raw=+ep /usr/local/bin/fiend-core \
    && apk del .caps \
    && install -d -o fiend -g fiend -m 0700 /etc/fiend /var/lib/fiend /run/fiend
USER fiend
VOLUME ["/etc/fiend", "/var/lib/fiend", "/run/fiend"]
ENTRYPOINT ["fiend-entrypoint"]
CMD ["panel"]
