$ErrorActionPreference = 'Stop'

function Say([string] $Message) {
  Write-Host $Message
}

if (Get-Command cargo -ErrorAction SilentlyContinue) {
  Say "[worktree] Prefetching Rust deps (root crate)..."
  try {
    cargo fetch --locked --features fast-alloc | Out-Null
  } catch {
    try {
      cargo fetch --locked | Out-Null
    } catch {
      cargo fetch | Out-Null
    }
  }

  if (Test-Path "evtx-wasm/Cargo.toml") {
    Say "[worktree] Prefetching Rust deps (evtx-wasm)..."
    Push-Location "evtx-wasm"
    try {
      cargo fetch --locked | Out-Null
    } catch {
      cargo fetch | Out-Null
    }
    Pop-Location
  }
} else {
  Say "[worktree] cargo not found; skipping Rust dependency prefetch."
}

$viewerDir = "evtx-wasm/evtx-viewer"
$skipViewerSetup = $false
if ($env:CURSOR_SKIP_VIEWER_SETUP) { $skipViewerSetup = $true }
if (Test-Path ".cursor/skip-viewer-setup") { $skipViewerSetup = $true }

if ($skipViewerSetup) {
  Say "[worktree] Skipping viewer deps install (CURSOR_SKIP_VIEWER_SETUP or .cursor/skip-viewer-setup present)."
} elseif (Test-Path "$viewerDir/package.json") {
  if (Get-Command bun -ErrorAction SilentlyContinue) {
    Say "[worktree] Installing viewer deps with bun ($viewerDir)..."
    Push-Location $viewerDir
    try {
      bun install --frozen-lockfile | Out-Null
    } catch {
      bun install | Out-Null
    }
    Pop-Location
  } else {
    Say "[worktree] bun not found; skipping viewer deps install ($viewerDir)."
  }
}

Say "[worktree] Setup complete."


