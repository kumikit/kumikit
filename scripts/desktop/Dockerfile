# A Linux desktop environment for the shell's end-to-end tests and its footprint check:
# WebKitGTK with its WebDriver, a virtual X server, Rust and Node. CI's Ubuntu runner
# installs the same packages; this image lets the same tests run locally.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl pkg-config git \
      libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsoup-3.0-dev \
      webkit2gtk-driver xvfb xauth dbus-x11 fonts-dejavu-core x11-apps imagemagick \
    && rm -rf /var/lib/apt/lists/*
ARG RUST_VERSION=1.100.0
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain ${RUST_VERSION} \
      --target wasm32-wasip2 && chmod -R a+rwX /opt/rustup /opt/cargo
# The WebDriver server for Tauri apps; it drives WebKitWebDriver.
RUN cargo install --locked tauri-driver --version 2.1.0 && chmod -R a+rwX /opt/cargo
ARG NODE_MAJOR=24
RUN curl -fsSL https://deb.nodesource.com/setup_${NODE_MAJOR}.x | bash - \
    && apt-get install -y --no-install-recommends nodejs && rm -rf /var/lib/apt/lists/*
WORKDIR /src
