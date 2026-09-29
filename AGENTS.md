# Repository Guidelines

## Project Structure & Module Organization

Lightmail pairs a native macOS/Windows interfaces with shared Rust services through UniFFI on macOS.

- `Sources/Lightmail/`: SwiftUI/AppKit views, presentation state, OS adapters, and Swift test harnesses.
- `desktop/`: Windows GPUI views, WebView2 host, OS adapters, and CI acceptance.
- `core/`: application services, OAuth, translation, composition, protocols, SQLite, caching, and Rust tests.
- `examples/headless.rs`: portable service consumer without a UI.
- `Generated/FFI/` and `Sources/Lightmail/Generated/`: generated bindings; regenerate rather than edit manually.
- `tests/`, `scripts/`: Python protocol fixtures, builds, validation, and packaging.
- `Resources/`, `docs/`: assets, architecture, screenshots, and release notes.
- `third_party/`: vendored IMAP/GPUI compatibility patches; preserve licenses.

## Build, Test, and Development Commands

macOS needs Apple Silicon, macOS 15+, Swift 6+, Rust, and Python 3. Windows needs Rust, MSVC, Windows SDK, and WebView2.

- `bash scripts/bootstrap-rust.sh`: install project-local Rust if needed.
- `bash scripts/build.sh`: build Rust, regenerate bindings, compile and sign `dist/轻邮.app`.
- `bash scripts/cargo.sh test --lib`: run Rust tests.
- `python3 scripts/check.py`: run isolated checks after building.
- `cargo run --locked --example headless -- build/headless-demo`: exercise shared services without Swift.
- `./scripts/build-windows.ps1`: build the Windows client.
- `./scripts/package-windows.ps1`: create installer and portable ZIP.
- `bash scripts/package.sh`: generate ZIP and checksums.

## Architecture Boundaries

Keep sync scheduling, retries, cache windows, outbox timers, OAuth, LLM translation, and exports in Rust `MailApplication`. Frontends issue commands and consume events; they own selection, layout, and dialogs. Implement `PlatformServices` for credentials and proxies. Keep Apple Translation, Keychain, clipboard, and WebKit integration in native adapters. Do not duplicate core records or business algorithms in Swift or GPUI. Call `stop()` before disposing of services or changing databases.

## Coding Style & Naming Conventions

Use two-space Swift indentation and four-space Rust/Python indentation. Types use `UpperCamelCase`; Swift members use `lowerCamelCase`; Rust/Python functions use `snake_case`. Run rustfmt on edited Rust. No Swift linter is configured.

## Testing Guidelines

Add behavior-focused Rust regression tests and synthetic fixtures. CI validates Windows/Linux core, native interfaces, protocols, and memory. No numeric coverage threshold applies. Fixtures do not prove provider delivery or long-run stability.

## Commit & Pull Request Guidelines

Use imperative `fix:` and `docs:` commits. PRs describe behavior, checks, limitations, and screenshots.

## Security & Resource Discipline

Never commit credentials, real mail, account databases, or private screenshots. Real sending requires authorization. Preserve per-account latest-20 body caches and retained summaries. Stop fixtures and remove task-owned temporary data and unused intermediates; preserve sources, toolchains, current releases, and verification records.
