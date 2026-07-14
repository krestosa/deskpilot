# File purpose: Measures startup time, working set, CPU use, handles, and threads in an interactive Windows session.
[CmdletBinding()]
param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\x86_64-pc-windows-msvc\release\DeskPilot.exe'),
    [ValidateRange(5, 300)][int]$SampleSeconds = 15,
    [switch]$SafeMode
)

$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
$dataDir = Join-Path ([System.IO.Path]::GetTempPath()) ("DeskPilot-measure-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dataDir | Out-Null
$args = @('--data-dir', $dataDir, 'run', '--foreground', '--no-tray')
if ($SafeMode) { $args += @('--no-hook', '--no-dynamic') }

# Function purpose: Runs one DeskPilot command against the isolated measurement data directory and returns its exit code and captured output.
function Invoke-DeskPilotRaw {
    param([string[]]$Arguments)
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $exe
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.ArgumentList.Add('--data-dir')
    $start.ArgumentList.Add($dataDir)
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $probe = [System.Diagnostics.Process]::new()
    $probe.StartInfo = $start
    $null = $probe.Start()
    $stdout = $probe.StandardOutput.ReadToEnd()
    $stderr = $probe.StandardError.ReadToEnd()
    $probe.WaitForExit()
    return [pscustomobject]@{
        ExitCode = $probe.ExitCode
        Stdout = $stdout.TrimEnd()
        Stderr = $stderr.TrimEnd()
    }
}

$timer = [System.Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath $exe -ArgumentList $args -PassThru -WindowStyle Hidden
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 50
        $status = Invoke-DeskPilotRaw -Arguments @('status', '--json')
    } while ($status.ExitCode -ne 0 -and [DateTime]::UtcNow -lt $deadline)
    if ($status.ExitCode -ne 0) { throw "DeskPilot did not become IPC-ready: $($status.Stderr)" }
    $timer.Stop()

    $process.Refresh()
    $cpuStart = $process.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Seconds $SampleSeconds
    $process.Refresh()
    $cpuEnd = $process.TotalProcessorTime.TotalMilliseconds

    $exeSize = (Get-Item -LiteralPath $exe).Length
    $zip = Get-ChildItem (Join-Path $PSScriptRoot '..\dist') -Filter 'DeskPilot-portable-*.zip' -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    $result = [ordered]@{
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        startup_ms = $timer.ElapsedMilliseconds
        sample_seconds = $SampleSeconds
        cpu_time_ms_during_sample = [math]::Round($cpuEnd - $cpuStart, 3)
        average_cpu_percent_single_core = [math]::Round((($cpuEnd - $cpuStart) / ($SampleSeconds * 1000)) * 100, 4)
        working_set_bytes = $process.WorkingSet64
        private_memory_bytes = $process.PrivateMemorySize64
        thread_count = $process.Threads.Count
        executable_size_bytes = $exeSize
        zip_size_bytes = if ($zip) { $zip.Length } else { $null }
        watchdog_interval_ms = ((Get-Content (Join-Path $dataDir 'deskpilot.toml') -Raw | Select-String 'watchdog_interval_ms\s*=\s*(\d+)').Matches.Groups[1].Value -as [int])
        safe_mode = [bool]$SafeMode
    }
    $result | ConvertTo-Json -Depth 4
}
finally {
    $null = Invoke-DeskPilotRaw -Arguments @('shutdown')
    if (-not $process.HasExited) {
        $process.WaitForExit(5000) | Out-Null
    }
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    Remove-Item -Recurse -Force -LiteralPath $dataDir -ErrorAction SilentlyContinue
}
