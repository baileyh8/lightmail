$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$version = [regex]::Match((Get-Content desktop/Cargo.toml -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
if (-not $version -or -not (Test-Path target/release/Lightmail.exe)) { throw 'Build the Windows app first.' }
$stage = "build/package-windows-$version"
New-Item -ItemType Directory -Force $stage, dist | Out-Null
Copy-Item target/release/Lightmail.exe, LICENSE, THIRD_PARTY_NOTICES.md $stage
Copy-Item packaging/windows-readme.txt "$stage/README.txt"
$zip = "dist/Lightmail-v$version-windows-x64.zip"
Compress-Archive -Path "$stage/*" -DestinationPath $zip -Force
$iscc = Get-Command ISCC.exe -ErrorAction SilentlyContinue
if (-not $iscc) { $iscc = Get-Item "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" -ErrorAction Stop }
& $iscc.FullName "/DAppVersion=$version" packaging/windows.iss
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
Get-ChildItem "dist/Lightmail-v$version-windows-x64*" | ForEach-Object { "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower())  $($_.Name)" } | Set-Content dist/SHA256SUMS-windows.txt -Encoding ascii
