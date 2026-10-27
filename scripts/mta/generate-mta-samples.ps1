<#
.SYNOPSIS
    Generates .MTA (LocaleMetaData) fixtures and Windows' own rendering of
    every event in them, which is the ground truth for evtx's MTA support.

.DESCRIPTION
    Needs Windows PowerShell 5.1, an elevated shell, and mc.exe, rc.exe and
    link.exe on PATH (a Visual Studio developer environment). CI runs it from
    .github/workflows/mta-samples.yml, once per phase, each in a fresh
    process so nothing is cached between them:

      -Phase Generate     install the test provider and classic sources,
                          write events, export and archive the scenarios,
                          probe them, then uninstall everything we installed
      -Phase Uninstalled  probe the archives again; Windows can now only use
                          the .MTA files

    Scenarios:
      custom          MtaTest-Provider/Operational: custom level, tasks,
                      task-specific and global opcodes, keywords, escapes,
                      non-ASCII text, an event without a message.
      custom-merged   Operational + Admin through a structured query, so two
                      events share each EventRecordID from 1 to 5.
      custom-bulk     custom plus 1000 more events (many EVT/MSG pages).
      classic         Application events from two classic sources, one whose
                      message file does not exist.
      system-runner   The first 300 events of this runner's System log.
      *-sample        Subsets of samples/*.evtx from this repository. The
                      runner has no Sysmon provider, so sysmon-sample covers
                      events whose publisher is not installed.

    Every .evtx is archived with `wevtutil al` (en-US; the custom ones also
    de-DE, and custom also fr-FR, which it has no strings for). MtaProbe.cs
    asks wevtapi to render every event, writing
    truth/<scenario>.<mode>-<lcid>.jsonl, where mode is:
      installed     publishers installed, archive not consulted
      archived      EvtOpenPublisherMetadata(LogFilePath = the .evtx)
      uninstalled   archived, after the test publishers were removed
#>
param(
    [Parameter(Mandatory = $true)] [ValidateSet('Generate', 'Uninstalled')] [string] $Phase,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [string] $RepoSamples
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

# Runs `wevtutil qe` and keeps its output as UTF-8. Redirecting through cmd
# with /uni:true is the only way to get the text without a lossy code page.
function Export-Query([string] $evtx, [string] $options, [string] $out) {
    $tmp = "$out.utf16"
    cmd /c "wevtutil qe `"$evtx`" /lf:true $options /uni:true > `"$tmp`""
    if ($LASTEXITCODE -ne 0) {
        throw "wevtutil qe $evtx $options exited with $LASTEXITCODE"
    }
    Write-Utf8 $out ([IO.File]::ReadAllText($tmp, [Text.Encoding]::Unicode))
    Remove-Item $tmp
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$OutDir = [IO.Path]::GetFullPath($OutDir)
$truth = Join-Path $OutDir 'truth'
$providerDir = 'C:\MtaTest'
$provider = 'MtaTest-Provider'
$scenarios = 'custom', 'custom-merged', 'custom-bulk', 'classic', 'system-runner', 'security-sample', 'system-sample', 'sysmon-sample'
$ours = 'custom', 'custom-merged', 'custom-bulk', 'classic'
$localized = 'custom', 'custom-merged', 'classic'

function Evtx([string] $name) { Join-Path $OutDir "$name.evtx" }

Add-Type -Path (Join-Path $here 'MtaProbe.cs') -ReferencedAssemblies 'System.Xml'

function Probe([string] $name, [string] $mode, [int] $lcid) {
    $path = Evtx $name
    $archive = if ($mode -eq 'installed') { '' } else { $path }
    $withXml = $name -ne 'custom-bulk'
    $count = [MtaProbe]::Probe($path, $archive, $lcid, (Join-Path $truth "$name.$mode-$lcid.jsonl"), $withXml)
    Write-Host "probe $name $mode $lcid -> $count events"
}

if ($Phase -eq 'Uninstalled') {
    foreach ($name in $ours) { Probe $name 'uninstalled' 1033 }
    foreach ($name in $localized) {
        Probe $name 'uninstalled' 1031
        Export-Query (Evtx $name) '/f:RenderedXml /e:Events' (Join-Path $truth "$name.rendered-uninstalled.xml")
    }
    Probe 'custom' 'uninstalled' 1036
    Export-Query (Evtx 'custom') '/f:text /l:de-DE' (Join-Path $truth 'custom.text-de-DE-uninstalled.txt')
    Export-Query (Evtx 'custom') '/f:text /l:en-US' (Join-Path $truth 'custom.text-en-US-uninstalled.txt')

    $sums = Get-ChildItem -Recurse $OutDir -File | Sort-Object FullName | ForEach-Object {
        '{0}  {1}' -f (Get-FileHash $_.FullName).Hash.ToLower(), $_.FullName.Substring($OutDir.Length + 1).Replace('\', '/')
    }
    Write-Utf8 (Join-Path $OutDir 'SHA256SUMS') (($sums -join "`n") + "`n")
    return
}

New-Item -ItemType Directory -Force $OutDir, $truth, $providerDir | Out-Null

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
Exec 'wevtutil.exe' @('gp', $provider, '/ge:true', '/gm:true', '/f:xml')

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
Exec 'wevtutil.exe' @('epl', 'System', (Evtx 'system-runner'), '/q:*[System[EventRecordID<=300]]')
Exec 'wevtutil.exe' @('epl', "$RepoSamples\security.evtx", (Evtx 'security-sample'), '/lf:true', '/q:*[System[EventRecordID<=120]]')
Exec 'wevtutil.exe' @('epl', "$RepoSamples\system.evtx", (Evtx 'system-sample'), '/lf:true', '/q:*[System[EventRecordID<=250]]')
Exec 'wevtutil.exe' @('epl', "$RepoSamples\sysmon.evtx", (Evtx 'sysmon-sample'), '/lf:true', '/q:*[System[EventRecordID<=1791]]')

1..1000 | ForEach-Object { Write-Custom 1 @(('bulk-{0:D4}' -f $_), $_) }
Start-Sleep -Seconds 5
Exec 'wevtutil.exe' @('epl', "$provider/Operational", (Evtx 'custom-bulk'))

foreach ($name in $scenarios) { Probe $name 'installed' 1033 }

# --- Archive -----------------------------------------------------------------

$hashesBefore = @{}
foreach ($name in $scenarios) { $hashesBefore[$name] = (Get-FileHash (Evtx $name)).Hash }

foreach ($name in $scenarios) { Exec 'wevtutil.exe' @('al', (Evtx $name), '/l:en-US') }
foreach ($name in $localized) { Exec 'wevtutil.exe' @('al', (Evtx $name), '/l:de-DE') }
Exec 'wevtutil.exe' @('al', (Evtx 'custom'), '/l:fr-FR')

$changed = @($scenarios | Where-Object { (Get-FileHash (Evtx $_)).Hash -ne $hashesBefore[$_] })
Write-Utf8 (Join-Path $truth 'archive-changed-evtx.txt') (($changed -join "`n") + "`n")

foreach ($name in $scenarios) { Probe $name 'archived' 1033 }
foreach ($name in $localized) {
    Probe $name 'archived' 1031
    Export-Query (Evtx $name) '/f:RenderedXml /e:Events' (Join-Path $truth "$name.rendered.xml")
}

# --- Remove what we installed ------------------------------------------------

Exec 'wevtutil.exe' @('um', "$providerDir\MtaTest.man")
Remove-EventLog -Source 'MtaTestClassic'
Remove-EventLog -Source 'MtaTestClassicMissing'
# This process may still have the resource DLL mapped, which blocks deleting
# it but not renaming it.
Rename-Item "$providerDir\MtaTest.dll" 'MtaTest.dll.removed'
