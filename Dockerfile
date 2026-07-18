# ---- Stage 1: Rust 서버 빌드 ----
FROM rust:1.85-bookworm AS server-builder
WORKDIR /build
COPY server/Cargo.toml server/Cargo.lock ./
COPY server/src ./src
RUN cargo build --release

# ---- Stage 2: 클라이언트 빌드 ----
FROM node:22-bookworm-slim AS client-builder
WORKDIR /build
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/index.html client/vite.config.ts client/tsconfig.json ./
COPY client/src ./src
RUN npm run build

# ---- Stage 3: 런타임(슬림) ----
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=server-builder /build/target/release/server ./server
COPY --from=client-builder /build/dist ./client-dist

ENV GAMEROBOTFACTORY_BIND_ADDR=0.0.0.0:8080
ENV GAMEROBOTFACTORY_STATIC_DIR=/app/client-dist
ENV GAMEROBOTFACTORY_DB_PATH=/data/gamerobotfactory.sqlite3
ENV RUST_LOG=info

EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["./server"]
