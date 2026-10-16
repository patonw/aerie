# This image comes in multiple variants (flavors):
# - slim: Minimal image containing just the app
# - nodepy: app with nodejs, npm, python and uv
# - nix: app with the nix tool
#
# slim can be used as a base image for further customization.
# If you want to use other distros, you can copy /app from here.
#
# The majority of NCP servers run on python or node.js so nodepy,
# should be sufficient for most cases. However, the image itself
# uses considerable space.
#
# The nix flavor provides the most flexibility. However, at runtime it must download
# an entire build chain the first time anything is installed with the nix command.

ARG flavor=slim

FROM debian:stable-slim AS base-slim

FROM ghcr.io/astral-sh/uv:debian AS base-nodepy
RUN apt-get update && apt-get install -y --no-install-recommends \
    nodejs \
    npm \
    git \
    && rm -rf /var/lib/apt/lists/*

# This is even worse:
# RUN apt-get update && apt-get install -y --no-install-recommends \
#     curl \
#     ca-certificates \
#     libssl-dev \
#     && rm -rf /var/lib/apt/lists/*
#
# RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.3/install.sh | bash
#
# ENV NVM_DIR=/root/.nvm
# RUN ls -a /root/.nvm
# RUN cat /root/.profile
# RUN . /root/.nvm/nvm.sh && nvm install node

FROM base-slim AS base-nix
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install linux \
  --extra-conf "sandbox = false" \
  --init none \
  --no-confirm

ENV PATH="$PATH:/nix/var/nix/profiles/default/bin"

# Pre-load build tools:
# RUN nix-shell -p hello --run hello

FROM base-${flavor} AS final

WORKDIR /app
COPY target/release/backend /app/backend
COPY dist /app/dist

CMD ["/app/backend"]
