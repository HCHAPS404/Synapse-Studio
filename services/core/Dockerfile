# Multi-stage image for the Core API (task E2.10).
#
# Stage 1 builds the `neuron` CLI from the Rust workspace so the API can run real
# `neuron check|build|emit` jobs as a subprocess (design decision 1). Stage 2 is the
# Python service plus the data the API reads at run time (device manifests, targets, the
# standard library and the JSON Schemas). Build context is the repository root.
#
# Build locally:   docker build -f services/core/Dockerfile -t synapse-core .
# Render uses this via render.yaml (runtime: docker, dockerfilePath: services/core/Dockerfile).

# ---- Stage 1: the Neuron CLI -------------------------------------------------------
FROM rust:1.82-slim-bookworm AS neuron
WORKDIR /src
# Copy the whole workspace: the CLI depends on most crates by path.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY lib ./lib
COPY targets ./targets
COPY devices ./devices
COPY spec ./spec
COPY conformance ./conformance
COPY examples ./examples
# Release build of just the CLI. If this ever gets too slow or large on Render, the
# fallback (design) is to drop this stage and ship a check/emit-only image; the compile
# runner already degrades to "binary not found" without crashing.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p neuron-cli \
    && cp /src/target/release/neuron /usr/local/bin/neuron

# ---- Stage 2: the Python service ---------------------------------------------------
FROM python:3.12-slim-bookworm AS api
# unshare (util-linux) lets the compile sandbox drop the network for a job; without it the
# self-test reports "unverified" and the job still runs with no sockets opened.
RUN apt-get update \
    && apt-get install -y --no-install-recommends util-linux ca-certificates \
    && rm -rf /var/lib/apt/lists/*
# uv, pinned, for reproducible installs.
COPY --from=ghcr.io/astral-sh/uv:0.5.11 /uv /usr/local/bin/uv

WORKDIR /app
# Install dependencies first for layer caching.
COPY services/core/pyproject.toml services/core/uv.lock ./
RUN uv sync --locked --no-dev --no-install-project

# The service and its migrations.
COPY services/core/ ./
RUN uv sync --locked --no-dev

# The Neuron binary and the data the API reads at run time.
COPY --from=neuron /usr/local/bin/neuron /usr/local/bin/neuron
COPY devices /opt/synapse/devices
COPY targets /opt/synapse/targets
COPY lib /opt/synapse/lib
COPY spec/schemas /opt/synapse/spec/schemas

ENV SYNAPSE_NEURON_BIN=/usr/local/bin/neuron \
    SYNAPSE_DEVICES_DIR=/opt/synapse/devices \
    PATH="/app/.venv/bin:${PATH}"

EXPOSE 8000
# $PORT is set by Render; default 8000 for a plain `docker run`.
CMD ["sh", "-c", "uv run --no-dev uvicorn synapse_core.main:app --host 0.0.0.0 --port ${PORT:-8000}"]
