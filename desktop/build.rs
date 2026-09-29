fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    // gpui-pre lays out nested views recursively. Unoptimized builds overflow the
    // default 1 MiB main-thread stack when the settings page opens. (gpui-pre
    // already embeds the DPI-aware application manifest.)
    if windows && msvc {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
}
