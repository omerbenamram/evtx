# Helpers for the scripts in this folder and in ../mta, which dot-source this
# file. Windows PowerShell 5.1.

function Exec([string] $exe, [string[]] $argv) {
    Write-Host "> $exe $($argv -join ' ')"
    & $exe @argv | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "$exe exited with $LASTEXITCODE"
    }
}

function Write-Utf8([string] $path, [string] $text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding $false))
}

# Relative to the current location, not the process directory.
function Resolve-FullPath([string] $path) {
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($path).TrimEnd('\', '/')
}

# <dir>\windows-info.txt: the Windows version and cultures of this machine.
function Write-WindowsInfo([string] $dir) {
    $os = Get-CimInstance Win32_OperatingSystem
    Write-Utf8 (Join-Path $dir 'windows-info.txt') (
        "$($os.Caption) $($os.Version) (build $($os.BuildNumber))`n" +
        "UI culture: $((Get-UICulture).Name), culture: $((Get-Culture).Name)`n")
}

# <dir>\SHA256SUMS: one line per file, with its path relative to $base.
function Write-Sha256Sums([string] $dir, [string] $base, [IO.FileInfo[]] $files) {
    $sums = foreach ($file in $files) {
        '{0}  {1}' -f (Get-FileHash $file.FullName).Hash.ToLower(), $file.FullName.Substring($base.Length + 1).Replace('\', '/')
    }
    Write-Utf8 (Join-Path $dir 'SHA256SUMS') (($sums -join "`n") + "`n")
}

# Builds the resource DLL that a manifest names (mc, rc, link), puts it and a
# copy of the manifest in the DLL's folder, which every account can read, and
# installs the manifest. Returns the installed copy, for `wevtutil um`.
function Install-Manifest([string] $manifest, [string] $build) {
    $name = [IO.Path]::GetFileNameWithoutExtension($manifest)
    $dll = ([xml](Get-Content -Raw $manifest)).instrumentationManifest.instrumentation.events.provider.resourceFileName
    $dir = Split-Path -Parent $dll
    $installed = Join-Path $dir "$name.man"
    New-Item -ItemType Directory -Force $dir, $build | Out-Null
    Copy-Item $manifest $installed
    Push-Location $build
    try {
        Exec 'mc.exe' @('-h', $build, '-r', $build, $installed)
        Exec 'rc.exe' @('/nologo', '/fo', "$name.res", "$name.rc")
        Exec 'link.exe' @('/nologo', '/dll', '/noentry', '/machine:x64', "/out:$dll", "$name.res")
    } finally {
        Pop-Location
    }
    Exec 'icacls.exe' @($dir, '/grant', '*S-1-1-0:(OI)(CI)RX')
    Exec 'wevtutil.exe' @('im', $installed)
    $installed
}
