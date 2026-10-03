# Explicit live, no-auth checks. Does not read account databases or send mail.
param([ValidateRange(1,65535)][int]$ProxyPort = 10808,
    [string]$OutputDirectory = '')
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$workspace = (Get-Location).Path
if (-not $OutputDirectory) { $OutputDirectory = "build/windows-live-proxy-$ProxyPort-" + (Get-Date -Format 'yyyyMMdd-HHmmss') }
$output = [IO.Path]::GetFullPath((Join-Path $workspace $OutputDirectory))
$buildRoot = (Join-Path $workspace 'build') + [IO.Path]::DirectorySeparatorChar
if (-not $output.StartsWith($buildRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Diagnostic output must be under this workspace build directory' }
if (Test-Path -LiteralPath (Join-Path $output 'result.json')) { throw 'This directory contains a verification record; choose a fresh output directory' }
New-Item -ItemType Directory -Force -Path $output | Out-Null
$previousPort = $env:LIGHTMAIL_DIAGNOSTIC_PROXY_PORT
$previousReport = $env:LIGHTMAIL_DIAGNOSTIC_REPORT_DIR
try {
    $env:LIGHTMAIL_DIAGNOSTIC_PROXY_PORT = [string]$ProxyPort
    $env:LIGHTMAIL_DIAGNOSTIC_REPORT_DIR = $output
    & cargo test --locked --lib live_ -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Live proxy diagnostics failed' }
} finally {
    $env:LIGHTMAIL_DIAGNOSTIC_PROXY_PORT = $previousPort
    $env:LIGHTMAIL_DIAGNOSTIC_REPORT_DIR = $previousReport
}
$checks = @((Get-Content (Join-Path $output 'mail-tls.json') -Raw | ConvertFrom-Json)) +
    @((Get-Content (Join-Path $output 'google-http.json') -Raw | ConvertFrom-Json))
if ($checks.Count -ne 12 -or @($checks | Where-Object passed -ne $true).Count -ne 0) { throw 'Incomplete or failed diagnostic report' }
@{ passed=$true; proxyHost='127.0.0.1'; proxyPort=$ProxyPort; checks=$checks.Count;
    protocols=@('http','socks5'); credentialsUsed=$false; actualAuthentication=$false;
    realMailSent=$false; checkedAt=(Get-Date).ToUniversalTime().ToString('o') } |
    ConvertTo-Json | Set-Content (Join-Path $output 'result.json')
Write-Output "Live proxy 127.0.0.1:$ProxyPort : 12 checks passed; no credentials or mail submission. Reports: $output"
