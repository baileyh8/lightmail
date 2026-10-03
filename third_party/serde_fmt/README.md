# serde_fmt compatibility patch

This is `serde_fmt` 1.1.0 from the upstream crate, licensed under
Apache-2.0 OR MIT. The source is kept locally because Stylo's `ToCss` derives
cannot compile in the combined desktop dependency graph while the upstream
`impl From<serde_fmt::Error> for std::fmt::Error` exists. The compatibility
copy preserves the public formatting API and removes only that reverse
conversion; `value-bag` continues to use `to_debug` unchanged.

Upstream: https://crates.io/crates/serde_fmt/1.1.0
