# Repository Guidelines

## Project Structure & Module Organization

Lightmail pairs a native macOS interface with shared Rust services through UniFFI.

- `Sources/Lightmail/`: SwiftUI/AppKit views, presentation state, OS adapters, and Swift test harnesses.
- `core/`: application services, OAuth, translation, composition, protocols, SQLite, caching, and Rust tests.
- `examples/headless.rs`: portable service consumer without a UI.
- `Generated/FFI/` and `Sources/Lightmail/Generated/`: generated bindings; regenerate rather than edit manually.
- `tests/`, `scripts/`: Python protocol fixtures, builds, validation, and packaging.
- `Resources/`, `docs/`: assets, architecture, screenshots, and release notes.
- `third_party/imap-proto/`: vendored compatibility patch; preserve licenses and patch documentation.

## Build, Test, and Development Commands

The app requires Apple Silicon, macOS 15+, Swift 6+, Rust stable, and Python 3. Core-only work requires Rust.

- `bash scripts/bootstrap-rust.sh`: install project-local Rust if needed.
- `bash scripts/build.sh`: build Rust, regenerate bindings, compile and sign `dist/轻邮.app`.
- `bash scripts/cargo.sh test --lib`: run Rust tests.
- `python3 scripts/check.py`: run isolated checks after building.
- `cargo run --locked --example headless -- build/headless-demo`: exercise shared services without Swift.
- `bash scripts/package.sh`: generate ZIP and checksums.

## Architecture Boundaries

Keep sync scheduling, retries, cache windows, outbox timers, OAuth, LLM translation, and exports in Rust `MailApplication`. Frontends issue commands and consume events; they own selection, layout, and dialogs. Implement `PlatformServices` for credentials and proxies. Keep Apple Translation, Keychain, clipboard, and WebKit integration in native adapters. Do not duplicate core records or business algorithms in Swift or GPUI. Call `stop()` before disposing of services or changing databases.

## Coding Style & Naming Conventions

Use two-space Swift indentation and four-space Rust/Python indentation. Types use `UpperCamelCase`; Swift members use `lowerCamelCase`; Rust/Python functions use `snake_case`. Run rustfmt on edited Rust. No Swift linter is configured. Avoid unrelated formatting changes.

## Testing Guidelines

Add behavior-focused Rust regression tests and synthetic fixtures. CI tests the core on Windows/Linux; macOS also validates Swift, WebKit, protocols, and memory. There is no numeric coverage threshold. Fixture success does not prove provider delivery or sustained memory stability.

## Commit & Pull Request Guidelines

History favors concise imperative `fix:` and `docs:` commits. Explain behavior, linked issues, checks, limitations, and relevant screenshots in PRs.

## Security & Resource Discipline

Never commit credentials, real mail, account databases, or private screenshots. Real sending requires authorization. Preserve per-account latest-20 body caches and retained summaries. Stop fixtures and remove task-owned temporary data and unused intermediates; preserve sources, toolchains, current releases, and verification records.
