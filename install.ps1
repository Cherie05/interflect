# Interflect installer for Windows.
#
#     irm https://raw.githubusercontent.com/Cherie05/interflect/main/install.ps1 | iex
#
# Downloads a prebuilt binary from GitHub Releases. No Rust toolchain needed.
# Short enough to read before running -- and if you would rather not, download
# from the Releases page instead:
#     https://github.com/Cherie05/interflect/releases
#
# Options (environment variables):
#     $env:INTERFLECT_VERSION = "0.1.0"        install a specific version
#     $env:INTERFLECT_BIN_DIR = "C:\tools"     install location

$ErrorActionPreference = "Stop"

$Repo   = "Cherie05/interflect"
$BinDir = if ($env:INTERFLECT_BIN_DIR) { $env:INTERFLECT_BIN_DIR }
          else { Join-Path $env:LOCALAPPDATA "Programs\interflect" }

function Fail($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

# --- architecture ------------------------------------------------------------
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($arch -ne "X64") {
    Fail "unsupported architecture: $arch. Only x64 Windows has a prebuilt binary. Build from source with: cargo install interflect"
}
$target = "x86_64-pc-windows-msvc"

# --- version -----------------------------------------------------------------
if ($env:INTERFLECT_VERSION) {
    $version = $env:INTERFLECT_VERSION
} else {
    Write-Host "Finding the latest release..."
    try {
        $rel = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" `
                                 -Headers @{ "User-Agent" = "interflect-installer" }
        $version = $rel.tag_name -replace '^v', ''
    } catch {
        Fail "could not reach the GitHub API. Download manually from https://github.com/$Repo/releases"
    }
}
if (-not $version) { Fail "could not determine the latest version" }

$name = "interflect-$target-v$version"
$url  = "https://github.com/$Repo/releases/download/v$version/$name.zip"

Write-Host "Installing interflect v$version ($target)"

# --- download ----------------------------------------------------------------
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("interflect-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
$zip = Join-Path $tmp "pkg.zip"

try {
    # Faster than the progress-rendering default by a wide margin.
    $ProgressPreference = "SilentlyContinue"
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
} catch {
    Fail "download failed: $url`nThat version or platform may not exist. See https://github.com/$Repo/releases"
}

# --- verify ------------------------------------------------------------------
try {
    $sumFile = Join-Path $tmp "pkg.sha256"
    Invoke-WebRequest -Uri "$url.sha256" -OutFile $sumFile -UseBasicParsing
    $expected = ((Get-Content $sumFile -Raw).Trim() -split '\s+')[0]
    $actual   = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected.ToLower()) {
        Fail "checksum mismatch -- refusing to install.`n  expected $expected`n  got      $actual"
    }
    Write-Host "Checksum verified."
} catch {
    # A mismatching checksum is fatal; a missing one is not -- but say so, so a
    # skipped check is never silent.
    if ($_.Exception.Message -like "*checksum mismatch*") { throw }
    Write-Host "WARNING: no checksum published for this release; skipping verification." -ForegroundColor Yellow
}

Expand-Archive -Path $zip -DestinationPath $tmp -Force

# --- install -----------------------------------------------------------------
New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
Copy-Item (Join-Path $tmp "$name\interflect.exe") (Join-Path $BinDir "interflect.exe") -Force
Copy-Item (Join-Path $tmp "$name\compare.exe")    (Join-Path $BinDir "interflect-compare.exe") -Force

# Example scenes and the offline scene builder -- the binary needs a scene.
$share = Join-Path $env:LOCALAPPDATA "interflect"
New-Item -ItemType Directory -Path $share -Force | Out-Null
Copy-Item (Join-Path $tmp "$name\scenes") $share -Recurse -Force -ErrorAction SilentlyContinue
Copy-Item (Join-Path $tmp "$name\tools")  $share -Recurse -Force -ErrorAction SilentlyContinue

Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Installed to $BinDir\interflect.exe"
Write-Host "Example scenes: $share\scenes"
Write-Host "Scene builder:  $share\tools\scene-builder.html"

# --- PATH --------------------------------------------------------------------
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$BinDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$BinDir", "User")
    Write-Host ""
    Write-Host "Added $BinDir to your PATH." -ForegroundColor Green
    Write-Host "Open a new terminal, then:"
} else {
    Write-Host ""
    Write-Host "Try it:"
}
Write-Host "  interflect render `"$share\scenes\product.rad`" -o out.png"
