<#
.SYNOPSIS
    Writes Windows' own XML rendering of every event in the repository's
    sample logs.

.DESCRIPTION
    Needs Windows PowerShell 5.1. CI runs it from
    .github/workflows/windows-xml.yml. For every .evtx under -Samples it
    writes <OutDir>\<path relative to -Samples>.jsonl with one line per event,
    in the order wevtapi returns them (RenderProbe.cs):

      index       the event's position in that order, from 0
      record_id   the record number of the event's bookmark
      xml         EvtRender(EvtRenderEventXml), exactly as returned
      render_error, next_error, seek_error, query_error
                  the Win32 error when wevtapi could not query, read or
                  render an event, instead of xml

    It also writes windows-info.txt (the Windows version) and SHA256SUMS of
    the logs it rendered.
#>
param(
    [Parameter(Mandatory = $true)] [string] $Samples,
    [Parameter(Mandatory = $true)] [string] $OutDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function Write-Utf8([string] $path, [string] $text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false))
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
# Relative to the current location, not the process directory.
function Resolve-FullPath([string] $path) {
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path).TrimEnd('\', '/')
}
$Samples = Resolve-FullPath $Samples
$OutDir = Resolve-FullPath $OutDir
New-Item -ItemType Directory -Force $OutDir | Out-Null

Add-Type -Path (Join-Path $here 'RenderProbe.cs')

$os = Get-CimInstance Win32_OperatingSystem
Write-Utf8 (Join-Path $OutDir 'windows-info.txt') (
    "$($os.Caption) $($os.Version) (build $($os.BuildNumber))`n" +
    "UI culture: $((Get-UICulture).Name), culture: $((Get-Culture).Name)`n")

$sums = @()
foreach ($log in Get-ChildItem -LiteralPath $Samples -Filter '*.evtx' -Recurse -File | Sort-Object FullName) {
    $relative = $log.FullName.Substring($Samples.Length + 1)
    $out = Join-Path $OutDir "$relative.jsonl"
    New-Item -ItemType Directory -Force (Split-Path -Parent $out) | Out-Null
    $count = [RenderProbe]::Probe($log.FullName, $out)
    Write-Host "$relative -> $count events"
    $sums += '{0}  {1}' -f (Get-FileHash $log.FullName).Hash.ToLower(), $relative.Replace('\', '/')
}
Write-Utf8 (Join-Path $OutDir 'SHA256SUMS') (($sums -join "`n") + "`n")
