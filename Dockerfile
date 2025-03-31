# Dockerfile for fastgif

# ---- Builder Stage ----
# Use a specific Rust version on Alpine for reproducible builds
FROM rust:1.85-alpine AS builder

# Install build dependencies if needed (e.g., for linking)
# RUN apk add --no-cache musl-dev

WORKDIR /usr/src/fastgif

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Build dependencies first to leverage Docker cache
# Create a dummy main.rs to build only dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy the actual source code
COPY src ./src

# Build the application
# Ensure the target directory exists for the final binary
RUN rm -f target/release/deps/fastgif* # Remove dummy build artifacts
RUN cargo build --release

# ---- Final Stage ----
FROM alpine:latest

# Add community repository for gifski
RUN echo "http://dl-cdn.alpinelinux.org/alpine/edge/community" >> /etc/apk/repositories

# Install runtime dependencies: ffmpeg, gifski, and ca-certificates for HTTPS requests
RUN apk add --no-cache ffmpeg gifski ca-certificates

# Set up a non-root user for security
RUN addgroup -S appgroup && adduser -S appuser -G appgroup
USER appuser

# Set working directory
WORKDIR /app

# Copy the compiled binary from the builder stage
COPY --from=builder /usr/src/fastgif/target/release/fastgif .

# Expose the application port (default 3000, but can be overridden by PORT env var)
EXPOSE 3000

# Command to run the application
CMD ["./fastgif"] 