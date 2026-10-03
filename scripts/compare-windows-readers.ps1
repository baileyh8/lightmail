param(
    [Parameter(Mandatory = $true)][string]$PrivateDirectory,
    [string]$OutputDirectory = ('build/reader-performance-' + (Get-Date -Format 'yyyyMMdd-HHmmss')),
    [ValidateRange(1, 10)][int]$Rounds = 3,
    [switch]$CaptureChecks
)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path $PSScriptRoot -Parent
Push-Location $workspace
$previousTarget = $env:CARGO_TARGET_DIR
try {
    $inputPath = (Resolve-Path -LiteralPath $PrivateDirectory).Path
    $buildRoot = [IO.Path]::GetFullPath((Join-Path $workspace 'build')) + [IO.Path]::DirectorySeparatorChar
    $outputPath = [IO.Path]::GetFullPath((Join-Path $workspace $OutputDirectory))
    if (-not $outputPath.StartsWith($buildRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'All profiles, metrics and optional private previews must stay under build/'
    }
    if (Test-Path -LiteralPath $outputPath) { throw 'Choose a fresh output directory for independent runtime profiles' }
    if (@(Get-ChildItem -LiteralPath $inputPath -Filter '*.html' -File).Count -lt 20) { throw 'Need twenty existing cached HTML files' }
    $revision = 'ed03fe183a9f129965ead6435c0caa8e9c49c401'
    $checkout = Join-Path $workspace '.tooling/blitz-research'
    if (-not (Test-Path -LiteralPath "$checkout/.git")) {
        git clone --filter=blob:none https://github.com/DioxusLabs/blitz.git $checkout
        if ($LASTEXITCODE -ne 0) { throw 'Could not obtain Blitz research source' }
        git -C $checkout checkout --detach $revision
        if ($LASTEXITCODE -ne 0) { throw 'Could not select pinned Blitz revision' }
    }
    if ((git -C $checkout rev-parse HEAD) -ne $revision) { throw 'Preserve existing research checkout; use the pinned revision before measuring' }
    $env:CARGO_TARGET_DIR = Join-Path $workspace 'target/blitz-reader-probe'
    cargo build --locked --release --manifest-path tests/reader-performance-probe/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'Reader comparison did not build' }
    $probe = Join-Path $env:CARGO_TARGET_DIR 'release/lightmail-reader-performance-probe.exe'
    $reports = @()
    for ($round = 1; $round -le $Rounds; $round++) {
        # Alternate order, run sequentially, and create a fresh WebView2 profile.
        $engines = if ($round % 2) { @('blitz', 'webview2') } else { @('webview2', 'blitz') }
        foreach ($engine in $engines) {
            $run = Join-Path $outputPath "$engine-$round"
            & $probe $engine $inputPath $run
            if ($LASTEXITCODE -ne 0) { throw "$engine comparison failed" }
            $report = Get-Content -LiteralPath "$run/report.json" -Raw | ConvertFrom-Json
            if ($report.reads.Count -ne 20 -or $report.captureCheck -or $report.released.processCount -ne 1) {
                throw 'Incomplete run or probe descendants did not exit after controller release'
            }
            $reports += $report
        }
    }
    function Median($values) {
        $sorted = @($values | Sort-Object)
        $half = [int][Math]::Floor($sorted.Count / 2)
        if ($sorted.Count % 2) { return [double]$sorted[$half] }
        return ([double]$sorted[$half - 1] + [double]$sorted[$half]) / 2
    }
    function Range($values) {
        $measure = $values | Measure-Object -Minimum -Maximum
        return @([Math]::Round($measure.Minimum, 2), [Math]::Round($measure.Maximum, 2))
    }
    $summary = [ordered]@{ measuredAtUtc = [DateTime]::UtcNow.ToString('o'); rounds = $Rounds; samplesPerRound = 20; engines = @() }
    foreach ($engine in @('blitz', 'webview2')) {
        $runs = @($reports | Where-Object engine -eq $engine)
        $times = @($runs | ForEach-Object { $_.reads | ForEach-Object { $_.loadMs } } | Sort-Object)
        $p95 = [int][Math]::Ceiling($times.Count * 0.95) - 1
        $summary.engines += [ordered]@{
            engine = $engine
            loadMedianMs = [Math]::Round((Median $times), 2)
            loadP95Ms = [Math]::Round($times[$p95], 2)
            readerReadyRangeMs = Range ($runs.readerReadyMs)
            afterReadsPrivateRangeMiB = Range ($runs.afterReads.treePrivateMiB)
            sampledPeakPrivateRangeMiB = Range ($runs.afterReads.peakTreePrivateMiB)
            idlePrivateRangeMiB = Range ($runs.idle.treePrivateMiB)
            hiddenPrivateRangeMiB = Range ($runs.hidden.treePrivateMiB)
            releasedPrivateRangeMiB = Range ($runs.released.treePrivateMiB)
            afterReadsWorkingSetRangeMiB = Range ($runs.afterReads.treeWorkingSetMiB)
            afterReadsProcessRange = Range ($runs.afterReads.processCount)
            runtimeVersions = @($runs.runtimeVersion | Where-Object { $_ } | Sort-Object -Unique)
            suspendSuccessful = @($runs.suspendSuccessful)
        }
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath "$outputPath/summary.json" -Encoding utf8
    if ($CaptureChecks) {
        foreach ($engine in @('blitz', 'webview2')) {
            & $probe $engine $inputPath (Join-Path $outputPath "private-$engine-check") --capture-check
            if ($LASTEXITCODE -ne 0) { throw "$engine capture check failed" }
        }
    }
    $summary | ConvertTo-Json -Depth 8
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
