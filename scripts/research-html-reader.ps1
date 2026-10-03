param([switch]$CoreInput, [switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$revision = '662ed3a6cbad72be8b9d76a8fc01e13faa8258e8'
$checkout = Join-Path (Get-Location) '.tooling/litehtml-research'
function Write-ResearchIfChanged([string]$path, [string]$content) {
    if (-not (Test-Path -LiteralPath $path) -or [IO.File]::ReadAllText($path).Replace("`r`n", "`n") -ne $content) {
        [IO.File]::WriteAllText($path, $content, [Text.UTF8Encoding]::new($false))
    }
}
if (-not (Test-Path -LiteralPath "$checkout/.git")) {
    git clone https://github.com/franzos/litehtml-rs.git $checkout
    if ($LASTEXITCODE -ne 0) { throw 'Could not obtain litehtml research source' }
    git -C $checkout checkout --detach $revision
    if ($LASTEXITCODE -ne 0) { throw 'Could not select the pinned revision' }
}
if ((git -C $checkout rev-parse HEAD) -ne $revision) { throw 'Research checkout is at a different revision; preserve it and choose the pinned revision manually' }
git -C $checkout submodule update --init --recursive
if ($LASTEXITCODE -ne 0) { throw 'Could not obtain the pinned litehtml C++ source' }

# Patch only the research copy of the binding's build script, using pristine
# pinned contents on every invocation. The C++ engine is unmodified.
$source = (git -C $checkout show "${revision}:litehtml-sys/build.rs") -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'Could not read pinned build script' }
$source = $source.Replace('    // Gumbo (C99)', '    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");' + "`n    // Gumbo (C99)")
$source = $source.Replace('.include(&gumbo_private_include)', '.include(&gumbo_private_include)' + "`n        .includes(if msvc { Some(gumbo_src.join(""visualc/include"")) } else { None })")
$source = $source.Replace('.std("c99")', '.std(if msvc { "c11" } else { "c99" })')
$source = $source.Replace('.std("c++17")', '.std("c++17")' + "`n        .flag_if_supported(""/utf-8"")`n        .flag_if_supported(""/permissive-"")")
$source = $source.Replace('    } else {' + "`n        println!(""cargo:rustc-link-lib=stdc++"");", '    } else if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {' + "`n        println!(""cargo:rustc-link-lib=stdc++"");")
Write-ResearchIfChanged "$checkout/litehtml-sys/build.rs" ($source + "`n")
$wrapper = (git -C $checkout show "${revision}:litehtml-sys/csrc/litehtml_c.cpp") -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'Could not read pinned C wrapper' }
# The engine explicitly instantiates stylesheet parsing for std::string, not
# const char*. The wrapper's old call leaves an unresolved MSVC symbol.
$wrapper = $wrapper.Replace('            css_text,', '            std::string(css_text),')
Write-ResearchIfChanged "$checkout/litehtml-sys/csrc/litehtml_c.cpp" ($wrapper + "`n")

$oldTarget = $env:CARGO_TARGET_DIR
$oldFlags = $env:RUSTFLAGS
try {
    $env:CARGO_TARGET_DIR = Join-Path (Get-Location) 'target/html-reader-probe'
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    $manifest = 'tests/html-reader-probe/Cargo.toml'
    $cargoArgs = @('build','--locked','--release','--manifest-path',$manifest)
    if ($CoreInput) { $cargoArgs += @('--features','core-input') }
    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'HTML probe build failed' }
    if (-not $BuildOnly) {
        $mode = if ($CoreInput) { 'core-input' } else { 'raw-input' }
        & "$env:CARGO_TARGET_DIR/release/lightmail-html-reader-probe.exe" "build/html-reader-research/$mode"
        if ($LASTEXITCODE -ne 0) { throw 'HTML probe failed; inspect its report' }
    }
} finally {
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:RUSTFLAGS = $oldFlags
}
