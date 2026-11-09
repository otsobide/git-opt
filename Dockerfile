# syntax=docker/dockerfile:1
# Single image: the Rust binary serves the API and the built Vue UI.

# ---- web UI -----------------------------------------------------------------
FROM node:24-slim AS web
WORKDIR /web
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# ---- backend ----------------------------------------------------------------
FROM rust:1.98-slim-trixie AS backend
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/git-opt /usr/local/bin/git-opt

# ---- runtime ----------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates git gifsicle tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 gitopt \
    && mkdir -p /data && chown gitopt:gitopt /data

COPY --from=backend /usr/local/bin/git-opt /usr/local/bin/git-opt
COPY --from=web /web/dist /app/web

ENV GIT_OPT_BIND=0.0.0.0:8484 \
    GIT_OPT_DATA_DIR=/data \
    GIT_OPT_STATIC_DIR=/app/web \
    RUST_LOG=git_opt=info,tower_http=warn

USER gitopt
WORKDIR /home/gitopt
VOLUME /data
EXPOSE 8484
ENTRYPOINT ["tini", "--"]
CMD ["git-opt"]
