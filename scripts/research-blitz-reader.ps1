param([string]$PrivateDirectory)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$target = Join-Path (Get-Location) 'target/blitz-reader-probe'
$env:CARGO_TARGET_DIR = $target
$args = @('build','--locked','--release','--manifest-path','tests/blitz-reader-probe/Cargo.toml')
cargo @args
if ($LASTEXITCODE -ne 0) { throw 'Blitz probe did not build on this Windows MSVC toolchain' }
$output = if ($PrivateDirectory) { Join-Path $PrivateDirectory 'blitz' } else { 'build/blitz-reader-research/synthetic' }
if ($PrivateDirectory) {
    if (-not $output.Contains('private')) { throw 'Private output must contain private in its path' }
    & "$target/release/lightmail-blitz-reader-probe.exe" --private $PrivateDirectory $output
} else {
    & "$target/release/lightmail-blitz-reader-probe.exe" $output
}
if ($LASTEXITCODE -ne 0) { throw 'Blitz probe failed' }
