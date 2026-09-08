param(
    [string]$OutputDir = ""
)

$ErrorActionPreference = "Continue"
$cargoBin = Join-Path $HOME ".cargo\bin"
if (Test-Path $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$expected = "bdea94200ae743fc94ea76987cfd4f6927e0ff8d"
$expectedSource = "cfd2b4f5d82a1940ae567b89492ffe768901e843c77eb332107eceac318aec1d"
$actual = (git -C $repo rev-parse HEAD).Trim()
if ($actual -ne $expected) {
    throw "refusing baseline run: expected $expected, found $actual"
}
$sourceRevision = (python -B (Join-Path $repo "scripts\performance-source-revision.py")).Trim()
if ($sourceRevision -ne $expectedSource) {
    throw "refusing baseline run: crate source is $sourceRevision, expected $expectedSource"
}
$fixtureRevision = (python -B (Join-Path $repo "scripts\performance-fixture-revision.py")).Trim()
if (-not $OutputDir) {
    $stamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
    $OutputDir = Join-Path $repo "performance-results\baseline-$stamp"
} elseif (-not [IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $repo $OutputDir
}
$out = [IO.Path]::GetFullPath($OutputDir)
New-Item -ItemType Directory -Force -Path $out | Out-Null
$coldTarget = Join-Path ([IO.Path]::GetTempPath()) ("sdax-perf-target-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $coldTarget | Out-Null

try {
    $rustc = (rustc --version --verbose) -join ";"
    $metadata = @(
        "baseline_commit=$actual"
        "source_revision=$sourceRevision"
        "fixture_revision=$fixtureRevision"
        "captured_utc=$((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
        "hostname=$env:COMPUTERNAME"
        "os=$([Environment]::OSVersion.VersionString)"
        "rustc=$rustc"
        "cargo=$(cargo --version)"
        "profile=release(codegen-units=1,lto=thin)"
        "runtime=tokio-current-thread"
        "workers=1"
        "warmup=8"
        "timed_samples=40"
        "build_samples=20"
        "trace=per-workload(default-or-counting-observer)"
    )
    $metadata | Set-Content -Encoding utf8 (Join-Path $out "metadata.txt")
    Get-CimInstance Win32_Processor |
        Select-Object Name, NumberOfCores, NumberOfLogicalProcessors, MaxClockSpeed |
        ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $out "cpu.json")
    Get-CimInstance Win32_OperatingSystem |
        Select-Object Caption, Version, OSArchitecture, TotalVisibleMemorySize |
        ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $out "os.json")

    Set-Location $repo
    $env:CARGO_TARGET_DIR = $coldTarget
    python -B scripts/time-command.py (Join-Path $out "compile-cold.json") -- `
        cargo build --manifest-path performance-harness/Cargo.toml --release --locked --offline `
        *> (Join-Path $out "compile-cold.log")
    if ($LASTEXITCODE -ne 0) { throw "cold compilation failed" }
    python -B scripts/time-command.py (Join-Path $out "compile-warm.json") -- `
        cargo build --manifest-path performance-harness/Cargo.toml --release --locked --offline `
        *> (Join-Path $out "compile-warm.log")
    if ($LASTEXITCODE -ne 0) { throw "warm compilation failed" }

    $binary = Join-Path $coldTarget "release\sdax-performance-harness.exe"
    (Get-Item $binary).Length | Set-Content -Encoding ascii (Join-Path $out "artifact-bytes.txt")
    & $binary verify *> (Join-Path $out "verify.log")
    if ($LASTEXITCODE -ne 0) { throw "fixture verification failed" }
    $rawSamples = & $binary bench --samples 40 --warmup 8 --build-samples 20 `
        2> (Join-Path $out "bench.log")
    if ($LASTEXITCODE -ne 0) { throw "benchmark failed" }
    $rawSamples | Set-Content -Encoding ascii (Join-Path $out "raw-samples.csv")
    python -B scripts/summarize-performance.py `
        (Join-Path $out "raw-samples.csv") (Join-Path $out "summary.csv")
    if ($LASTEXITCODE -ne 0) { throw "summary failed" }

    $probeStdout = Join-Path $out "resident-probe-stdout.txt"
    $probeStderr = Join-Path $out "resident-probe-stderr.txt"
    $probeStart = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $binary `
        -ArgumentList @("resident-probe", "--seconds", "10") `
        -RedirectStandardOutput $probeStdout -RedirectStandardError $probeStderr `
        -NoNewWindow -PassThru
    $peakWorkingSet = 0L
    $processorSeconds = 0.0
    while (-not $process.HasExited) {
        $process.Refresh()
        if ($process.PeakWorkingSet64 -gt $peakWorkingSet) {
            $peakWorkingSet = $process.PeakWorkingSet64
        }
        $processorSeconds = $process.TotalProcessorTime.TotalSeconds
        Start-Sleep -Milliseconds 100
    }
    $process.WaitForExit()
    $process.Refresh()
    if ($process.PeakWorkingSet64 -gt $peakWorkingSet) {
        $peakWorkingSet = $process.PeakWorkingSet64
    }
    $processorSeconds = $process.TotalProcessorTime.TotalSeconds
    $probeStart.Stop()
    if (($null -ne $process.ExitCode -and $process.ExitCode -ne 0) -or
        -not (Select-String -Quiet -Path $probeStdout -Pattern "outcome=ok")) {
        throw "resident probe failed"
    }
    @(
        "wall_seconds=$($probeStart.Elapsed.TotalSeconds.ToString('R', [Globalization.CultureInfo]::InvariantCulture))"
        "process_cpu_seconds=$($processorSeconds.ToString('R', [Globalization.CultureInfo]::InvariantCulture))"
        "peak_working_set_bytes=$peakWorkingSet"
        "process_exit_code=$($process.ExitCode)"
    ) | Set-Content -Encoding ascii (Join-Path $out "resident-probe-native.txt")
    @(
        "boundary=whole optimized harness process including startup, readiness, 10-second resident wait, shutdown, report, and process exit"
        "counter_source=System.Diagnostics.Process TotalProcessorTime and PeakWorkingSet64"
        "cpu_interpretation=process CPU over the whole wall interval; not pure idle CPU"
        "memory_interpretation=native peak working set for the process; not engine-only bytes"
        "wakeup_count=unmeasured"
    ) | Set-Content -Encoding utf8 (Join-Path $out "resident-probe-metadata.txt")
    Write-Output $out
} finally {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    # Retain build artifacts unless the owner explicitly requests cleanup.
    $coldTarget | Set-Content -Encoding utf8 (Join-Path $out "retained-target.txt")
}
