# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is an OpenAI proxy/gateway service built with Rust using the Pingora framework. It acts as an HTTP proxy that routes requests to various AI service endpoints (OpenAI, Google, Ollama) while providing rate limiting, metrics collection, and request/response transformation.

## Architecture

- **Entry Point**: `src/main.rs` - CLI argument parsing, server initialization and service setup
- **Core HTTP Gateway**: `src/http_proxy/` - Modular proxy implementation
  - `config.rs` - Configuration structs and HttpGateway initialization
  - `proxy.rs` - Main Pingora proxy logic and request handling
  - `parsing.rs` - Request/response parsing and transformation
  - `types.rs` - Core data types (Peer, RoutingRule, model mappings)
  - `metrics.rs` - Prometheus metrics collection
- **Rate Limiting**: `src/rate_limiter.rs` - Sliding window rate limiter traits and implementations
- **Utilities**: `src/utils.rs` - Helper functions

The service uses a modular architecture where different AI providers are mapped through routing rules based on model names (e.g., gpt-* routes to OpenAI, gemini-* to Google).

## Development Commands

### Build and Run
```bash
# Development build
cargo build

# Optimized build for production
cargo build --release

# Run with environment variables
OPENAI_TLS=true OPENAI_PORT=443 OPENAI_DOMAIN="api.openai.com" cargo run --release

# Run with logging
RUST_LOG=info cargo run --release
```

### Testing
```bash
# Run all tests
cargo test

# Run tests with debug logging
RUST_LOG=debug cargo test
```

### Code Quality
```bash
# Format code (uses rustfmt.toml with max_width=100)
cargo fmt

# Check formatting without modifying files
cargo fmt -- --check
```

## Configuration

The service accepts configuration through CLI arguments or environment variables:

### Key Environment Variables
- `OPENAI_TLS`: Enable TLS for OpenAI endpoints (default: true)
- `OPENAI_PORT`: OpenAI endpoint port (default: 443)
- `OPENAI_DOMAIN`: OpenAI endpoint domain (default: "api.openai.com")
- `PROXY_PORT`: HTTP proxy port (default: "8080")
- `METRICS_PORT`: Prometheus metrics port (default: "9090")
- `RUST_LOG`: Logging level (use "info" or "debug")

### Rate Limiting Options
- `ENABLE_RATE_LIMITING`: Enable rate limiting (default: false)
- `RATE_LIMIT_WINDOW_MIN`: Rate limit window in minutes (default: 60)
- `MAX_TOKENS`: Max tokens per window (default: 1000)
- `USER_HEADER`: User header key for rate limiting (default: "user")

## Dependencies

- **External Dependency**: This crate depends on a sibling path `../ai-api-converter`. Ensure this directory exists when building.
- **Key Libraries**: Pingora (proxy framework), tiktoken-rs (tokenization), prometheus (metrics), clap (CLI)

## Testing the Service

After running the service, test with curl:

```bash
# Basic chat completion request
curl -X POST http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer sk-your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}],
    "stream": true
  }'
```

## Development Notes

- Uses Rust 2024 edition with latest language features
- Follows Conventional Commits for commit messages
- Code is formatted to 100 character line width
- Use `anyhow::Result` for error handling
- Logging via `env_logger` and `log` macros
- Tests use `tokio`, `httpmock`, and `testcontainers` frameworks