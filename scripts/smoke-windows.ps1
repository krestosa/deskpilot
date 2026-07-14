[CmdletBinding()]
param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\x86_64-pc-windows-msvc\release\DeskPilot.exe'),
    [string]$ReportDirectory = (Join-Path $PSScriptRoot '..\artifacts\windows-smoke')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-DeskPilot {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
    $output = & $script:Exe --data-dir $script:DataDir @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "DeskPilot command failed ($LASTEXITCODE): $($Arguments -join ' ')`n$output"
    }
    return $output
}

function Get-Desktops {
    return @(Invoke-DeskPilot desktops list --json | ConvertFrom-Json)
}

function Wait-Until {
    param([scriptblock]$Condition, [string]$Failure, [int]$Seconds = 10)
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw $Failure
}

if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -notmatch 'AMD64') {
    throw 'Interactive smoke requires Windows x64.'
}
$os = Get-CimInstance Win32_OperatingSystem
$build = [int]$os.BuildNumber
if ($build -lt 26100) { throw "Interactive smoke requires Windows 11 24H2 or newer; build is $build." }
if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { throw 'Explorer is not running.' }
if (-not [Environment]::UserInteractive) { throw 'The runner session is not interactive.' }

$script:Exe = (Resolve-Path -LiteralPath $Executable).Path
$script:DataDir = Join-Path ([System.IO.Path]::GetTempPath()) ("DeskPilot-smoke-" + [guid]::NewGuid().ToString('N'))
$report = [ordered]@{
    started_utc = [DateTime]::UtcNow.ToString('o')
    windows_build = $build
    executable = $script:Exe
    checks = [ordered]@{}
}
New-Item -ItemType Directory -Force -Path $ReportDirectory, $script:DataDir | Out-Null
$process = $null
$notepad = $null

try {
    & $script:Exe status --json *> $null
    if ($LASTEXITCODE -eq 0) { throw 'A DeskPilot instance is already running; refusing to disturb it.' }

    $process = Start-Process -FilePath $script:Exe -ArgumentList @('--data-dir', $script:DataDir, 'run', '--foreground', '--no-tray') -PassThru -WindowStyle Hidden
    Wait-Until { & $script:Exe --data-dir $script:DataDir status --json *> $null; $LASTEXITCODE -eq 0 } 'DeskPilot did not become IPC-ready.'
    $report.checks.ipc = 'PASS'

    $doctor = Invoke-DeskPilot doctor --json | ConvertFrom-Json
    $doctor | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8NoBOM (Join-Path $ReportDirectory 'doctor-before.json')
    if (-not $doctor.backend.compatible) { throw "Backend is incompatible: $($doctor.backend.error)" }
    $report.checks.backend = 'PASS'

    $initial = Get-Desktops
    $initial | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8NoBOM (Join-Path $ReportDirectory 'desktops-initial.json')
    if ($initial.Count -ne 1) {
        throw "Dedicated smoke runner must begin with exactly one desktop; observed $($initial.Count)."
    }
    $initialId = $initial[0].id

    Invoke-DeskPilot desktops create --json | Out-Null
    Invoke-DeskPilot desktops next --json | Out-Null
    $notepad = Start-Process notepad.exe -PassThru
    Start-Sleep -Seconds 2
    Invoke-DeskPilot reconcile | Out-Null
    Wait-Until { (Get-Desktops).Count -eq 2 } 'DeskPilot did not converge to occupied + trailing spare.' 15
    $report.checks.trailing_spare = 'PASS'

    Invoke-DeskPilot desktops create --json | Out-Null
    if ((Get-Desktops).Count -ne 3) { throw 'Unable to create a controlled extra empty desktop.' }
    Invoke-DeskPilot reconcile | Out-Null
    Wait-Until { (Get-Desktops).Count -eq 2 } 'DeskPilot did not remove the duplicate trailing empty desktop.' 15
    $report.checks.trailing_compaction = 'PASS'

    $beforeNavigation = (Invoke-DeskPilot desktops current --json | ConvertFrom-Json).id
    Invoke-DeskPilot desktops next --json | Out-Null
    $afterNext = (Invoke-DeskPilot desktops current --json | ConvertFrom-Json).id
    Invoke-DeskPilot desktops previous --json | Out-Null
    $afterPrevious = (Invoke-DeskPilot desktops current --json | ConvertFrom-Json).id
    if ($beforeNavigation -eq $afterNext -or $afterPrevious -ne $beforeNavigation) {
        throw 'CLI navigation did not move next and return previous.'
    }
    $report.checks.cli_navigation = 'PASS'

    Add-Type @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class DeskPilotInputSmoke {
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    public const uint KEYUP = 0x0002;
    public const uint WHEEL = 0x0800;
    public static void WinWheelDown() {
        keybd_event(0x5B, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(100);
        mouse_event(WHEEL, 0, 0, unchecked((uint)-120), UIntPtr.Zero);
        System.Threading.Thread.Sleep(150);
        keybd_event(0x5B, 0, KEYUP, UIntPtr.Zero);
    }
    public static string ForegroundProcess() {
        uint pid;
        GetWindowThreadProcessId(GetForegroundWindow(), out pid);
        try { return Process.GetProcessById((int)pid).ProcessName; } catch { return ""; }
    }
}
'@
    Invoke-DeskPilot desktops previous --json *> $null
    $hookBefore = (Invoke-DeskPilot desktops current --json | ConvertFrom-Json).id
    [DeskPilotInputSmoke]::WinWheelDown()
    Wait-Until { (Invoke-DeskPilot desktops current --json | ConvertFrom-Json).id -ne $hookBefore } 'Synthetic Win+wheel did not change desktops.' 5
    $foreground = [DeskPilotInputSmoke]::ForegroundProcess()
    if ($foreground -match 'StartMenuExperienceHost') { throw 'Start opened after Win+wheel.' }
    $report.checks.win_wheel = 'PASS'
    $report.checks.start_suppression = 'PASS'

    Stop-Process -Id $notepad.Id -ErrorAction Stop
    $notepad = $null
    Start-Sleep -Seconds 2
    Invoke-DeskPilot reconcile | Out-Null
    Wait-Until { (Get-Desktops).Count -eq 1 } 'DeskPilot did not restore a single empty desktop after closing its test application.' 15
    if ((Get-Desktops)[0].id -eq $null) { throw 'Final desktop state is invalid.' }
    $report.checks.cleanup = 'PASS'

    $after = Invoke-DeskPilot doctor --json | ConvertFrom-Json
    $after | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8NoBOM (Join-Path $ReportDirectory 'doctor-after.json')
    $report.result = 'PASS'
}
catch {
    $report.result = 'FAIL'
    $report.error = $_.Exception.Message
    throw
}
finally {
    if ($notepad -and -not $notepad.HasExited) { Stop-Process -Id $notepad.Id -Force -ErrorAction SilentlyContinue }
    if ($process) {
        & $script:Exe --data-dir $script:DataDir shutdown *> $null
        if (-not $process.HasExited) { $process.WaitForExit(5000) | Out-Null }
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
    }
    $report.finished_utc = [DateTime]::UtcNow.ToString('o')
    $report | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8NoBOM (Join-Path $ReportDirectory 'smoke-report.json')
    if (Test-Path (Join-Path $script:DataDir 'logs')) {
        Copy-Item -Recurse -Force (Join-Path $script:DataDir 'logs') (Join-Path $ReportDirectory 'logs')
    }
    Remove-Item -Recurse -Force -LiteralPath $script:DataDir -ErrorAction SilentlyContinue
}
