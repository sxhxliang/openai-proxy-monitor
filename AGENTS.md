# Repository Guidelines

## Project Structure & Module Organization
- Source: `src/` — `main.rs` (entry), `http_proxy.rs` (Pingora HTTP gateway), `rate_limiter.rs` (sliding-window traits + dummy impl), `utils.rs` (helpers).
- Config: `Cargo.toml`, `rustfmt.toml` (formatting, `max_width = 100`), `.env` (local overrides).
- Docs & scripts: `README.md`, `openai_stream.py`, `test.py` (examples/helpers).
- Build output: `target/` (ignored).

## Build, Test, and Development Commands
- Build: `cargo build` (add `--release` for optimized binary).
- Run locally: `OPENAI_TLS=true OPENAI_PORT=443 OPENAI_DOMAIN=api.openai.com RUST_LOG=info cargo run --release`
- Test: `cargo test` (unit tests live inline with modules).
- Format: `cargo fmt` (uses `rustfmt.toml`).

Note: This crate depends on a sibling path `../ai-api-converter`. Ensure it exists when building.

## Coding Style & Naming Conventions
- Rust edition: 2024. Use `anyhow::Result` for fallible paths where practical.
- Indentation: 4 spaces; keep lines ≤ 100 chars (enforced by rustfmt).
- Naming: modules/files `snake_case`; types/traits `PascalCase`; functions/vars `snake_case`.
- Logging: prefer `log` macros with `env_logger` (`RUST_LOG=info` for local runs).

## Testing Guidelines
- Frameworks: `tokio` for async tests; `httpmock`/`testcontainers` available for HTTP/container tests.
- Location: unit tests under `#[cfg(test)]` modules beside code.
- Run: `cargo test` (set `RUST_LOG=debug` to troubleshoot).
- Keep tests hermetic; avoid external services unless using `testcontainers`.

## Commit & Pull Request Guidelines
- Commits: follow Conventional Commits (e.g., `feat(http_proxy): add request parsing`).
- PRs: include a clear description, linked issues, and screenshots or `curl` logs when changing behavior.
- CI expectations: code compiles, tests pass, `cargo fmt -- --check` is clean.

## Security & Configuration Tips
- Provide API keys at call-time; do not commit secrets. Use `.env` only for local development.
- Key env vars: `OPENAI_TLS`, `OPENAI_PORT`, `OPENAI_DOMAIN`, `RUST_LOG`.
- Quick smoke test:
  - Run: `cargo run --release`
  - Call: `curl -X POST http://127.0.0.1:8080/v1/chat/completions -H 'Authorization: Bearer <API_KEY>' -d '{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}],"stream":true}'`

