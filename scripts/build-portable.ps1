[CmdletBinding()]
param(
    [string]$Configuration = 'release',
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$OutputRoot = (Join-Path $PSScriptRoot '..\dist'),
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
& (Join-Path $PSScriptRoot 'verify-license.ps1') | Write-Host

if (-not $SkipBuild) {
    Push-Location $root
    try {
        cargo build --release --locked --target $Target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    }
    finally { Pop-Location }
}

$exe = Join-Path $root "target\$Target\$Configuration\DeskPilot.exe"
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Release executable not found: $exe"
}

$versionLine = & $exe --version
if ($LASTEXITCODE -ne 0 -or $versionLine -notmatch '^DeskPilot\s+(?<version>\d+\.\d+\.\d+)') {
    throw "Unable to determine DeskPilot version from $exe"
}
$version = $Matches.version
$packageName = "DeskPilot-portable-$version"
$packageDirectory = Join-Path $OutputRoot 'DeskPilot'
$zipPath = Join-Path $OutputRoot "$packageName.zip"

if (Test-Path -LiteralPath $OutputRoot) { Remove-Item -Recurse -Force -LiteralPath $OutputRoot }
New-Item -ItemType Directory -Force -Path $packageDirectory | Out-Null

$files = [ordered]@{
    $exe = 'DeskPilot.exe'
    (Join-Path $root 'deskpilot.example.toml') = 'deskpilot.example.toml'
    (Join-Path $root 'LICENSE.md') = 'LICENSE.md'
    (Join-Path $root 'README.md') = 'README.md'
    (Join-Path $root 'THIRD_PARTY_NOTICES.md') = 'THIRD_PARTY_NOTICES.md'
}
foreach ($entry in $files.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Key -PathType Leaf)) {
        throw "Package input is missing: $($entry.Key)"
    }
    Copy-Item -LiteralPath $entry.Key -Destination (Join-Path $packageDirectory $entry.Value)
}

$checksumLines = Get-ChildItem -LiteralPath $packageDirectory -File |
    Sort-Object Name |
    ForEach-Object {
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
        "$hash  $($_.Name)"
    }
$checksumPath = Join-Path $packageDirectory 'checksums.sha256'
[System.IO.File]::WriteAllLines($checksumPath, $checksumLines, [System.Text.UTF8Encoding]::new($false))

Compress-Archive -Path (Join-Path $packageDirectory '*') -DestinationPath $zipPath -CompressionLevel Optimal
$zipHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash.ToLowerInvariant()
$zipSize = (Get-Item -LiteralPath $zipPath).Length

$report = [ordered]@{
    version = $version
    executable = $exe
    executable_size_bytes = (Get-Item -LiteralPath $exe).Length
    package_directory = $packageDirectory
    zip = $zipPath
    zip_size_bytes = $zipSize
    zip_sha256 = $zipHash
    license_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $packageDirectory 'LICENSE.md')).Hash.ToLowerInvariant()
}
$reportPath = Join-Path $OutputRoot 'package-report.json'
$report | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8NoBOM -LiteralPath $reportPath
$report | ConvertTo-Json -Depth 4
