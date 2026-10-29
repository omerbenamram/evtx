<#
.SYNOPSIS
    Generates types.evtx: events whose payloads cover every manifest input
    type and a set of strings XML cannot hold as they are, written by Windows
    itself.

.DESCRIPTION
    Needs Windows PowerShell 5.1, an elevated shell, and mc.exe, rc.exe and
    link.exe on PATH (a Visual Studio developer environment). CI runs it from
    .github/workflows/windows-xml.yml.

    It installs the test provider in TypesTest.man, writes the events listed
    in TypesWriter.cs through EventWrite, waits until the provider's channel
    holds all of them, exports it to <OutDir>\types.evtx with `wevtutil epl`,
    and uninstalls the provider. render-samples.ps1 then gives Windows' own
    XML for each event.
#>
param(
    [Parameter(Mandatory = $true)] [string] $OutDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2

function Exec([string] $exe, [string[]] $argv) {
    Write-Host "> $exe $($argv -join ' ')"
    & $exe @argv
    if ($LASTEXITCODE -ne 0) {
        throw "$exe exited with $LASTEXITCODE"
    }
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
# Relative to the current location, not the process directory.
$OutDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
$providerDir = 'C:\EvtxTypes'
$manifest = Join-Path $here 'TypesTest.man'
$provider = ([xml](Get-Content -Raw $manifest)).instrumentationManifest.instrumentation.events.provider
$channel = "$($provider.name)/Operational"
$build = Join-Path $env:TEMP 'types-build'
New-Item -ItemType Directory -Force $OutDir, $providerDir, $build | Out-Null

Copy-Item $manifest $providerDir
Push-Location $build
try {
    Exec 'mc.exe' @('-h', $build, '-r', $build, "$providerDir\TypesTest.man")
    Exec 'rc.exe' @('/nologo', '/fo', 'TypesTest.res', 'TypesTest.rc')
    Exec 'link.exe' @('/nologo', '/dll', '/noentry', '/machine:x64', "/out:$providerDir\TypesTest.dll", 'TypesTest.res')
} finally {
    Pop-Location
}
Exec 'icacls.exe' @($providerDir, '/grant', '*S-1-1-0:(OI)(CI)RX')
Exec 'wevtutil.exe' @('im', "$providerDir\TypesTest.man")
try {
    # Events left by an earlier run would be exported too.
    Exec 'wevtutil.exe' @('cl', $channel)

    # mc's header has each event's EVENT_DESCRIPTOR, including the channel and the
    # channel keyword it assigned.
    $header = [IO.File]::ReadAllText((Join-Path $build 'TypesTest.h'))
    $descriptors = @{}
    foreach ($m in [regex]::Matches($header, 'const EVENT_DESCRIPTOR (\w+) = \{([^}]*)\};')) {
        $fields = foreach ($field in $m.Groups[2].Value.Split(',')) {
            $field = $field.Trim()
            if ($field.StartsWith('0x')) { [Convert]::ToUInt64($field.Substring(2), 16) } else { [Convert]::ToUInt64($field) }
        }
        $descriptors[$m.Groups[1].Value] = [uint64[]] $fields
        Write-Host "$($m.Groups[1].Value) = $($m.Groups[2].Value)"
    }

    Add-Type -Path (Join-Path $here 'TypesWriter.cs')
    $count = [TypesWriter]::WriteAll($provider.guid, $descriptors)
    Write-Host "wrote $count events"

    # The service logs them asynchronously.
    for ($i = 0; ; $i++) {
        $m = [regex]::Match((& wevtutil.exe gli $channel) -join "`n", 'numberOfLogRecords:\s*(\d+)')
        $logged = if ($m.Success) { [int] $m.Groups[1].Value } else { -1 }
        if ($logged -eq $count) { break }
        if ($i -eq 60) { throw "$channel has $logged of $count events" }
        Start-Sleep -Seconds 1
    }

    Exec 'wevtutil.exe' @('epl', $channel, (Join-Path $OutDir 'types.evtx'), '/ow:true')
} finally {
    & wevtutil.exe um "$providerDir\TypesTest.man"
}
