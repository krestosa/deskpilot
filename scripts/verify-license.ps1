# File purpose: Verifies that LICENSE.md remains byte-identical to the approved PolyForm Strict license text.
[CmdletBinding()]
param(
    [string]$Path = (Join-Path $PSScriptRoot '..\LICENSE.md')
)

$ErrorActionPreference = 'Stop'
$expected = 'e2361f52ad5be22b937a6e983c824a534c5cffa454b6c34af2f8ce0c2cdf7c1a'
$resolved = (Resolve-Path -LiteralPath $Path).Path
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $resolved).Hash.ToLowerInvariant()
if ($actual -ne $expected) {
    throw "LICENSE.md SHA-256 mismatch. Expected $expected, got $actual."
}
Write-Output "LICENSE.md SHA-256: $actual"
