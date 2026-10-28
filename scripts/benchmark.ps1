param(
    [string]$Source = 'video-clips-for-testing/pngtuber_speedrun.mkv',
    [string]$Binary = 'dist/acea.exe',
    [double]$Start = 10,
    [double]$Duration = 6,
    [string]$OutputDirectory = 'test-output/benchmark-native',
    [ValidateSet('auto','software')][string]$Acceleration = 'auto'
)
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
Push-Location $workspacePath
try {
    $binaryPath = (Resolve-Path -LiteralPath $Binary).Path
    $sourcePath = (Resolve-Path -LiteralPath $Source).Path
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $benchmarkPath = (Resolve-Path -LiteralPath $OutputDirectory).Path
    $runs = @()
    foreach ($scenario in @('zoom', 'combined')) {
        $effects = @(@{type='zoom';start=0;end=$Duration;from_zoom=1;to_zoom=1.7;target=@{x=0.5;y=0.5}})
        if ($scenario -eq 'combined') {
            $effects += @{type='spotlight';start=0;end=$Duration;rect=@{x=0.35;y=0.2;width=0.3;height=0.5};dim=0.55}
            $effects += @{type='blur';start=0;end=$Duration;rect=@{x=0;y=0;width=0.2;height=1};sigma=8}
        }
        $requestPath = Join-Path $benchmarkPath ($scenario + '.json')
        $stdoutPath = Join-Path $benchmarkPath ($scenario + '.result.json')
        $stderrPath = Join-Path $benchmarkPath ($scenario + '.progress.log')
        @{version=2;source=$sourcePath;output=(Join-Path $benchmarkPath ($scenario+'.mp4'));range=@{start=$Start;duration=$Duration};effects=$effects;settings=@{acceleration=$Acceleration};overwrite=$true} |
            ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $requestPath -Encoding utf8
        $timer = [System.Diagnostics.Stopwatch]::StartNew()
        $process = Start-Process -FilePath $binaryPath -ArgumentList @('render','--request',('"' + $requestPath + '"')) -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
        $peakBytes = 0L
        do {
            $workingBytes = 0L
            foreach ($observed in @(Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) {
                if ($observed.Id -eq $process.Id) {
                    try { $workingBytes += $observed.WorkingSet64 } catch { }
                }
            }
            $peakBytes = [Math]::Max($peakBytes, $workingBytes)
            Start-Sleep -Milliseconds 40
            $process.Refresh()
        } while (-not $process.HasExited)
        $process.WaitForExit()
        $timer.Stop()
        $renderResult = Get-Content -LiteralPath $stdoutPath -Raw | ConvertFrom-Json
        if (-not $renderResult.ok) { throw ($renderResult | ConvertTo-Json -Depth 10) }
        $runs += [ordered]@{
            scenario=$scenario
            width=$renderResult.width
            height=$renderResult.height
            duration=$renderResult.duration
            elapsed_seconds=[Math]::Round($timer.Elapsed.TotalSeconds,3)
            renderer_seconds=[Math]::Round($renderResult.processing_seconds,3)
            speed=[Math]::Round($renderResult.render_speed,2)
            sampled_peak_working_set_mib=[Math]::Round($peakBytes/1MB,1)
            output=$renderResult.output
            acceleration=$renderResult.acceleration
        }
    }
    $report = [ordered]@{
        source=$sourcePath
        measured_at=[DateTimeOffset]::Now.ToString('o')
        memory_method='40 ms samples of the standalone ACEA process working set. Includes shared pages; excludes GPU memory and out-of-process driver/service memory. Approximate sampled peak, not total system RAM usage.'
        runs=$runs
    }
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $benchmarkPath 'report.json') -Encoding utf8
    $report | ConvertTo-Json -Depth 10
} finally { Pop-Location }
