FROM rust:1.85-bookworm AS builder

WORKDIR /app

# Install dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Copy Cargo files first for dependency caching
COPY backend/Cargo.toml ./

# Create dummy src to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Pin problematic dependency versions
RUN cargo update time --precise 0.3.36 && \
    cargo update time-core --precise 0.1.2 && \
    cargo update time-macros --precise 0.2.18

RUN cargo build --release
RUN rm -rf src

# Copy actual source code
COPY backend/src ./src

# Build the actual application
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install FFmpeg and runtime dependencies
RUN apt-get update && apt-get install -y \
    ffmpeg \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the built binary
COPY --from=builder /app/target/release/boids-server /app/boids-server

# Create output directory
RUN mkdir -p /app/output

EXPOSE 3000

CMD ["./boids-server"]
