<#
.SYNOPSIS
    Writes Windows' own XML rendering of every event in the repository's
    sample logs.

.DESCRIPTION
    Needs Windows PowerShell 5.1. CI runs it from
    .github/workflows/windows-xml.yml. For every .evtx under -Samples it
    writes <OutDir>\<path relative to -Samples>.jsonl with one line per event,
    in the order wevtapi returns them (RenderProbe.cs; samples/windows/README.md
    lists the fields). It also writes windows-info.txt (the Windows version)
    and SHA256SUMS of the logs it rendered.
#>
param(
    [Parameter(Mandatory = $true)] [string] $Samples,
    [Parameter(Mandatory = $true)] [string] $OutDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $here 'Common.ps1')

$Samples = Resolve-FullPath $Samples
$OutDir = Resolve-FullPath $OutDir
New-Item -ItemType Directory -Force $OutDir | Out-Null

Add-Type -Path (Join-Path $here 'Wevtapi.cs'), (Join-Path $here 'RenderProbe.cs')

Write-WindowsInfo $OutDir

$logs = Get-ChildItem -LiteralPath $Samples -Filter '*.evtx' -Recurse -File | Sort-Object FullName
foreach ($log in $logs) {
    $relative = $log.FullName.Substring($Samples.Length + 1)
    $out = Join-Path $OutDir "$relative.jsonl"
    New-Item -ItemType Directory -Force (Split-Path -Parent $out) | Out-Null
    $count = [RenderProbe]::Probe($log.FullName, $out)
    Write-Host "$relative -> $count events"
}
Write-Sha256Sums $OutDir $Samples $logs
