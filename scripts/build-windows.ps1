$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# GPUI's Direct3D shaders require the SDK compiler, not the Rust toolchain alone.
$fxc = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\fxc.exe" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $fxc) { throw 'Install the Windows 10/11 SDK with the Visual Studio C++ workload.' }
$env:PATH = "$($fxc.DirectoryName);$env:PATH"
# The GPUI Kit client is verified with Rust 1.97; CI pins 1.97.0.
$rust = (rustc --version) -replace '^rustc (\d+\.\d+).*$', '$1'
if ([version]$rust -lt [version]'1.97') { throw "Rust 1.97 or newer is required (found $rust); run rustup update." }
$env:CARGO_BUILD_JOBS = '3'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release -p lightmail-desktop
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
