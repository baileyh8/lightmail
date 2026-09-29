$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# GPUI's Direct3D shaders require the SDK compiler, not the Rust toolchain alone.
$fxc = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\fxc.exe" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $fxc) { throw 'Install the Windows 10/11 SDK with the Visual Studio C++ workload.' }
$env:PATH = "$($fxc.DirectoryName);$env:PATH"
$env:CARGO_BUILD_JOBS = '3'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release -p lightmail-desktop
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
