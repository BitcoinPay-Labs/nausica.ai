# Build stage
FROM rust:latest as builder

WORKDIR /app

# Copy the source code
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install necessary runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /app/target/release/upfile-protocol /app/upfile-protocol

# Copy static files and templates
COPY static ./static
COPY templates ./templates

# Create data directory
RUN mkdir -p /app/data

# Set environment variables
ENV HOST=0.0.0.0
ENV PORT=8080
ENV DATABASE_PATH=/app/data/upfile.db

EXPOSE 8080

CMD ["/app/upfile-protocol"]
