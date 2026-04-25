FROM rust:1.88-bookworm AS rojo
RUN cargo install --locked --version 7.6.1 rojo
WORKDIR /app
COPY studio-plugin/ studio-plugin/
RUN mkdir -p studio-plugin/dist && \
    cd studio-plugin && \
    rojo build --output dist/Roblox-Player-Role.rbxm

FROM rust:1.88-bookworm AS builder
WORKDIR /app

# Cache dependencies in a separate layer
COPY Cargo.toml ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    mkdir -p studio-plugin/dist && touch studio-plugin/dist/Roblox-Player-Role.rbxm && \
    cargo build --release && \
    rm -rf src target/release/roblox-player-role target/release/deps/roblox_player_role*

# Build actual source
COPY src/ src/
COPY migrations/ migrations/
COPY favicon.ico ./
COPY --from=rojo /app/studio-plugin/dist/Roblox-Player-Role.rbxm studio-plugin/dist/Roblox-Player-Role.rbxm
RUN cargo build --release && strip target/release/roblox-player-role

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/roblox-player-role /usr/local/bin/
EXPOSE 8089
CMD ["roblox-player-role"]
