<#
.SYNOPSIS
    Generates the .MTA (LocaleMetaData) fixtures in samples/mta and Windows'
    own rendering of every event in them.

.DESCRIPTION
    Needs Windows PowerShell 5.1, an elevated shell, and mc.exe, rc.exe and
    link.exe on PATH (a Visual Studio developer environment). CI runs it from
    .github/workflows/mta-samples.yml, once per phase, each phase in a fresh
    process so nothing wevtapi cached survives into the probes:

      -Phase Generate  install a test provider and two classic sources, write
                       events, export and archive the scenarios below, then
                       uninstall everything it installed
      -Phase Probe     render every archived event through wevtapi into
                       windows/<scenario>_<lcid>.jsonl

    Scenarios:
      custom           MtaTest-Provider/Operational (see MtaTest.man): custom
                       level, tasks, task-specific and global opcodes,
                       keywords, %n/%t/%% escapes, non-ASCII text, events
                       without a message. Archived for en-US and de-DE.
      custom-merged    Operational + Admin through a structured query, so two
                       events share each EventRecordID from 1 to 5.
      classic          Application events from two classic sources, one whose
                       message file does not exist.
      system-runner    The last 50 events of this runner's System log.
      security-sample  Subsets of samples/*.evtx from this repository, each
      system-sample    small enough for one chunk. The runner has no Sysmon
      sysmon-sample    provider, so sysmon-sample covers events whose
                       publisher is not in the archive.

    The probe phase also renders samples/mta/setup-export.evtx, an Event
    Viewer export that was archived on another machine (from PR 284).

    The probe opens publisher metadata with LogFilePath set to the .evtx,
    which makes wevtapi read only the .MTA, even for publishers that are
    still installed. For the test provider and classic sources the publishers
    are gone by then anyway.
#>
param(
    [Parameter(Mandatory = $true)] [ValidateSet('Generate', 'Probe')] [string] $Phase,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $RepoSamples
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

function Write-Utf8([string] $path, [string] $text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false))
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$OutDir = [IO.Path]::GetFullPath($OutDir)
$providerDir = 'C:\MtaTest'
$provider = 'MtaTest-Provider'
$archives = @{
    'custom'          = 'en-US', 'de-DE'
    'custom-merged'   = @('en-US')
    'classic'         = @('en-US')
    'system-runner'   = @('en-US')
    'security-sample' = @('en-US')
    'system-sample'   = @('en-US')
    'sysmon-sample'   = @('en-US')
}

function Evtx([string] $name) { Join-Path $OutDir "$name.evtx" }

if ($Phase -eq 'Probe') {
    Add-Type -Path (Join-Path $here 'MtaProbe.cs') -ReferencedAssemblies 'System.Xml'
    $windows = Join-Path $OutDir 'windows'
    New-Item -ItemType Directory -Force $windows | Out-Null
    function Probe([string] $evtx, [int] $lcid) {
        $name = [IO.Path]::GetFileNameWithoutExtension($evtx)
        $count = [MtaProbe]::Probe($evtx, $lcid, (Join-Path $windows "$($name)_$lcid.jsonl"))
        Write-Host "probe $name $lcid -> $count events"
    }
    foreach ($name in $archives.Keys) {
        foreach ($culture in $archives[$name]) {
            Probe (Evtx $name) ([Globalization.CultureInfo]::GetCultureInfo($culture).LCID)
        }
    }
    Probe (Join-Path $RepoSamples 'mta\setup-export.evtx') 1033
    $sums = Get-ChildItem -Recurse $OutDir -File | Sort-Object FullName | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash $_.FullName).Hash.ToLower(), $_.FullName.Substring($OutDir.Length + 1).Replace('\', '/')
    }
    Write-Utf8 (Join-Path $OutDir 'SHA256SUMS') (($sums -join "`n") + "`n")
    return
}

New-Item -ItemType Directory -Force $OutDir, $providerDir | Out-Null

$os = Get-CimInstance Win32_OperatingSystem
Write-Utf8 (Join-Path $OutDir 'windows-info.txt') (
    "$($os.Caption) $($os.Version) (build $($os.BuildNumber))`n" +
    "UI culture: $((Get-UICulture).Name), culture: $((Get-Culture).Name)`n")

# --- Test provider -----------------------------------------------------------

$build = Join-Path $env:TEMP 'mta-build'
New-Item -ItemType Directory -Force $build | Out-Null
Copy-Item (Join-Path $here 'MtaTest.man') $providerDir
Push-Location $build
try {
    Exec 'mc.exe' @('-h', $build, '-r', $build, "$providerDir\MtaTest.man")
    Exec 'rc.exe' @('/nologo', '/fo', 'MtaTest.res', 'MtaTest.rc')
    Exec 'link.exe' @('/nologo', '/dll', '/noentry', '/machine:x64', "/out:$providerDir\MtaTest.dll", 'MtaTest.res')
} finally {
    Pop-Location
}
Exec 'icacls.exe' @($providerDir, '/grant', '*S-1-1-0:(OI)(CI)RX')
Exec 'wevtutil.exe' @('im', "$providerDir\MtaTest.man")

function Write-Custom([int] $id, [object[]] $payload) {
    if ($payload) {
        New-WinEvent -ProviderName $provider -Id $id -Payload $payload
    } else {
        New-WinEvent -ProviderName $provider -Id $id
    }
}

# Windows PowerShell reads this file as ANSI, so non-ASCII text is built from
# code points.
$unicode = 'payload ' + [char]0xFC + 'n' + [char]0xEF + 'c' + [char]0xF6 + 'd' + [char]0xE9 + ' ' + [char]::ConvertFromUtf32(0x1F600)

1..120 | ForEach-Object { Write-Custom 1 @(('host-{0:D3}' -f $_), (1000 + $_)) }
1..5 | ForEach-Object { Write-Custom 1 @('dup-host', 80) }
Write-Custom 2 @('alpha & <beta>')
Write-Custom 2 @($unicode)
Write-Custom 3 @('payload with a literal %1')
Write-Custom 3 @("multi`r`nline payload")
Write-Custom 4 $null
Write-Custom 4 $null
Write-Custom 6 @('server-a')
Write-Custom 6 @('server-b')
Write-Custom 7 @('x' * 3000)
Write-Custom 8 @($true, [guid]'01234567-89ab-cdef-0123-456789abcdef', [long]1234567890123)
Write-Custom 8 @($false, [guid]'fedcba98-7654-3210-fedc-ba9876543210', [long]-42)
Write-Custom 9 @('no task here')
1..5 | ForEach-Object { Write-Custom 5 @("admin error $_") }

# --- Classic sources ---------------------------------------------------------

New-EventLog -LogName Application -Source 'MtaTestClassic'
New-EventLog -LogName Application -Source 'MtaTestClassicMissing' -MessageResourceFile "$providerDir\does-not-exist.dll"
Write-EventLog -LogName Application -Source 'MtaTestClassic' -EventId 1000 -EntryType Information -Category 0 -Message 'Classic information message.'
Write-EventLog -LogName Application -Source 'MtaTestClassic' -EventId 1001 -EntryType Warning -Category 3 -Message "Classic warning with`r`ntwo lines."
Write-EventLog -LogName Application -Source 'MtaTestClassic' -EventId 1002 -EntryType Error -Category 0 -Message 'Classic error <&>.'
Write-EventLog -LogName Application -Source 'MtaTestClassicMissing' -EventId 42 -EntryType Information -Message 'The message file of this source does not exist.'

Start-Sleep -Seconds 5

# --- Exports -----------------------------------------------------------------

Exec 'wevtutil.exe' @('epl', "$provider/Operational", (Evtx 'custom'))

$structuredQuery = Join-Path $build 'merged-query.xml'
Write-Utf8 $structuredQuery @"
<QueryList>
  <Query Id="0">
    <Select Path="$provider/Operational">*</Select>
    <Select Path="$provider/Admin">*</Select>
  </Query>
</QueryList>
"@
Exec 'wevtutil.exe' @('epl', $structuredQuery, (Evtx 'custom-merged'), '/sq:true')

Exec 'wevtutil.exe' @('epl', 'Application', (Evtx 'classic'), "/q:*[System[Provider[@Name='MtaTestClassic' or @Name='MtaTestClassicMissing']]]")

$newest = (Get-WinEvent -LogName System -MaxEvents 1).RecordId
Exec 'wevtutil.exe' @('epl', 'System', (Evtx 'system-runner'), "/q:*[System[EventRecordID>$($newest - 50)]]")

function Export-Sample([string] $sample, [int] $first, [int] $last) {
    Exec 'wevtutil.exe' @('epl', "$RepoSamples\$sample.evtx", (Evtx "$sample-sample"), '/lf:true', "/q:*[System[EventRecordID>=$first and EventRecordID<=$last]]")
}
# The windows with the most distinct providers (system) and event IDs
# (security) that fit in one chunk.
Export-Sample 'security' 132 191
Export-Sample 'system' 164 243
Export-Sample 'sysmon' 1742 1761

foreach ($name in $archives.Keys) {
    foreach ($culture in $archives[$name]) {
        Exec 'wevtutil.exe' @('al', (Evtx $name), "/l:$culture")
    }
}

# --- Remove what we installed ------------------------------------------------

Exec 'wevtutil.exe' @('um', "$providerDir\MtaTest.man")
Remove-EventLog -Source 'MtaTestClassic'
Remove-EventLog -Source 'MtaTestClassicMissing'
# This process may still have the resource DLL mapped, which blocks deleting
# it but not renaming it.
Rename-Item "$providerDir\MtaTest.dll" 'MtaTest.dll.removed'
