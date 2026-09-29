# -SoakReads N also opens demo messages N times and records the working set per read.
param([switch]$InstallerOnly, [int]$SoakReads = 0)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$root = (New-Item -ItemType Directory -Force build/windows-acceptance).FullName
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WindowCapture {
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
'@
function Capture($process, $name) {
    $process.Refresh()
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) { return }
    $rect = New-Object WindowCapture+Rect
    if (-not [WindowCapture]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) { return }
    $width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) { return }
    $bitmap = New-Object System.Drawing.Bitmap($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    # PrintWindow renders only this window (full window rect, invisible resize
    # borders included), so another window above it never enters the evidence.
    try {
        $hdc = $graphics.GetHdc()
        try { $rendered = [WindowCapture]::PrintWindow($process.MainWindowHandle, $hdc, 2) } finally { $graphics.ReleaseHdc($hdc) }
        if ($rendered) { $bitmap.Save((Join-Path $root "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png) }
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function CheckReaderPixels($process, $name, $report) {
    $geometry = Get-Content "$report/reader-geometry.json" -Raw | ConvertFrom-Json
    $origin = New-Object WindowCapture+Point; $rect = New-Object WindowCapture+Rect
    if (-not [WindowCapture]::ClientToScreen($process.MainWindowHandle, [ref]$origin) -or -not [WindowCapture]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) { throw 'Reader screen bounds unavailable' }
    $bitmap = [System.Drawing.Bitmap]::new((Join-Path $root "$name.png"))
    try {
        $left = [int]($origin.X - $rect.Left + $geometry.x * $geometry.scale) + 8
        $top = [int]($origin.Y - $rect.Top + $geometry.y * $geometry.scale) + 8
        $right = [int]($left + $geometry.width * $geometry.scale) - 16
        $bottom = [int]($top + [Math]::Min(200, $geometry.height - 16) * $geometry.scale)
        if ($left -lt 0 -or $top -lt 0 -or $right -ge $bitmap.Width -or $bottom -ge $bitmap.Height -or $bottom -le $top) { throw 'Reader pixel region is outside the captured window' }
        $dark = 0
        for ($y = $top; $y -lt $bottom; $y += 2) { for ($x = $left; $x -lt $right; $x += 2) {
            $pixel = $bitmap.GetPixel($x, $y)
            if ($pixel.R -lt 180 -and $pixel.G -lt 180 -and $pixel.B -lt 180) { $dark++ }
        } }
        @{ darkSamples = $dark; region = @($left,$top,$right,$bottom); passed = ($dark -gt 100) } | ConvertTo-Json | Set-Content "$root/reader-visual.json"
        if ($dark -le 100) { throw "Reader text loaded but its pixels are blank: $dark dark samples" }
    } finally { $bitmap.Dispose() }
}
if (-not $InstallerOnly) {
    $data = Join-Path $root 'native-data'; $report = Join-Path $root 'native'
    # A report left by an earlier run would satisfy the waits below at once.
    Remove-Item -Recurse -Force $data, $report -ErrorAction SilentlyContinue
    if ($SoakReads -gt 0) { $env:LIGHTMAIL_SOAK_READS = "$SoakReads" }
    $process = Start-Process target/release/Lightmail.exe -ArgumentList @('--demo','--run-acceptance','--data-dir',"`"$data`"",'--acceptance-dir',"`"$report`"") -PassThru
    Remove-Item Env:LIGHTMAIL_SOAK_READS -ErrorAction SilentlyContinue
    $deadline = [DateTime]::UtcNow.AddSeconds(150 + 2 * $SoakReads); $peak = 0; $captured = $false
    $curve = [System.Collections.Generic.List[object]]::new(); $lastRead = 0; $rest = $null
    try {
        while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
            $process.Refresh(); $peak = [Math]::Max($peak, $process.WorkingSet64)
            if ($peak -gt 500MB) { throw "Native UI exceeded 500 MiB working set: $peak" }
            if ($SoakReads -gt 0 -and (Test-Path "$report/soak-progress.json")) {
                # The runner rewrites this file, so a sample may catch it half written.
                try { $read = (Get-Content "$report/soak-progress.json" -Raw | ConvertFrom-Json).read } catch { $read = $lastRead }
                if ($read -gt $lastRead) {
                    $lastRead = $read
                    $curve.Add([pscustomobject]@{ read = $read; workingSetMiB = [Math]::Round($process.WorkingSet64 / 1MB, 2); privateMiB = [Math]::Round($process.PrivateMemorySize64 / 1MB, 2) })
                }
            }
            if (Test-Path "$report/screenshot-request.txt") {
                $name = (Get-Content "$report/screenshot-request.txt" -Raw).Trim()
                if ($name -notmatch '^[a-z0-9-]+$') { throw 'Invalid screenshot name' }
                Capture $process $name
                if (-not (Test-Path "$root/$name.png")) { throw "Could not capture $name" }
                if ($name -eq 'windows-inbox') { CheckReaderPixels $process $name $report }
                Remove-Item "$report/screenshot-request.txt"
                Set-Content "$report/$name.captured" 'done'
            }
            if (Test-Path "$report/native-acceptance.json") {
                if ($null -eq $rest) { $rest = [Math]::Round($process.PrivateMemorySize64 / 1MB, 2) }
                if (-not $captured) { Capture $process 'native-reader'; $captured = $true }
            }
            Start-Sleep -Milliseconds 250
        }
        if (-not $process.HasExited) { throw 'Native acceptance exceeded its time limit' }
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "Native acceptance exited $($process.ExitCode)" }
        $result = Get-Content "$report/native-acceptance.json" -Raw | ConvertFrom-Json
        if (-not $result.passed) { throw "Native acceptance failed: $($result.error)" }
        $checks = @($result.checks) + @('reader-visible-pixels')
        @{ peakAppWorkingSetMiB = [Math]::Round($peak / 1MB, 2); checks = $checks; passed = $true; boundary = 'The native reader runs inside the app process, so its working set is the whole footprint; dedicated GPU memory excluded' } | ConvertTo-Json | Set-Content "$root/result.json"
        Write-Output "Native UI: $($result.checks.Count) checks passed; app peak $([Math]::Round($peak / 1MB, 2)) MiB"
        if ($SoakReads -gt 0) {
            if ($result.soakReads -ne $SoakReads -or $curve.Count -eq 0) { throw 'Soak run did not report its reads' }
            $curve | Export-Csv "$root/soak-memory.csv" -NoTypeInformation
            # Private memory after the first fifth, when caches have filled, against
            # the end: a steady climb means something is retained per read. The
            # working set is recorded too, but Windows trims it while the app idles.
            $fifth = [Math]::Max(1, $SoakReads / 5); $early = $curve[0].privateMiB
            foreach ($point in $curve) { if ($point.read -le $fifth) { $early = $point.privateMiB } }
            $late = $curve[-1].privateMiB
            @{ reads = $SoakReads; earlyPrivateMiB = $early; lastReadPrivateMiB = $late; growthMiB = [Math]::Round($late - $early, 2); restPrivateMiB = $rest; peakWorkingSetMiB = [Math]::Round($peak / 1MB, 2) } | ConvertTo-Json | Set-Content "$root/soak-summary.json"
            Write-Output "Soak: $SoakReads reads; private memory $early MiB after the first fifth, $late MiB at the end, $rest MiB after 30 s at rest"
            # Measured: a sawtooth that nets under 10 MiB over 1500 reads. A leak of
            # 100 KiB per read would cross this bound well before that.
            if ($late - $early -gt 64) { throw "Private memory grew $([Math]::Round($late - $early, 2)) MiB during the soak" }
        }
    } finally {
        @{ peakAppWorkingSetMiB = [Math]::Round($peak / 1MB, 2) } | ConvertTo-Json | Set-Content "$root/collector-metrics.json"
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
}
# Install the unmodified, already-packaged release binary into a disposable directory.
$installer = Get-ChildItem dist/*-windows-x64-setup.exe | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$install = Join-Path $root 'installed'
$setup = Start-Process $installer.FullName -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/DIR=`"$install`"") -Wait -PassThru
if ($setup.ExitCode -ne 0 -or -not (Test-Path "$install/Lightmail.exe")) { throw 'Installer failed' }
$data = Join-Path $root 'installed-data'; $report = Join-Path $root 'installed-report'
Remove-Item -Recurse -Force $data, $report -ErrorAction SilentlyContinue
$process = Start-Process "$install/Lightmail.exe" -ArgumentList @('--demo','--data-dir',"`"$data`"",'--acceptance-dir',"`"$report`"") -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path "$report/ui-state.json") -and -not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 250 }
    $state = Get-Content "$report/ui-state.json" -Raw | ConvertFrom-Json
    if ($state.accounts -ne 4 -or $state.messages -lt 7 -or -not $state.demo -or $process.HasExited) { throw 'Installed app did not open its demo database' }
    Start-Sleep -Seconds 2; Capture $process 'installed-app'
    $null = $process.CloseMainWindow()
    if (-not $process.WaitForExit(10000)) { throw 'Installed app did not close normally' }
} finally { if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force } }
$uninstall = Start-Process "$install/unins000.exe" -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -Wait -PassThru
if ($uninstall.ExitCode -ne 0 -or (Test-Path "$install/Lightmail.exe")) { throw 'Uninstall failed' }
if (-not (Test-Path "$data/Preview/mail.sqlite")) { throw 'Uninstall removed user data' }
Write-Output 'Installed release: startup, demo data, normal close and uninstall preservation passed'
