#!/usr/bin/env pwsh
# Build the Windows release zip.
#
#   pwsh -File scripts\build-release.ps1 [version]
#
# The Windows half of build-release.sh, and the same bargain: it ships the exes,
# because the friend this is for runs visual novels and not rustup.
#
# This file stays pure ASCII. Windows PowerShell reads an unsigned script with no
# byte-order mark as the machine's ANSI codepage, and a BOM is worse to carry
# than the restriction.

param([string]$Version)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = Split-Path -Parent $PSScriptRoot
if (-not $Version) { $Version = (Get-Date -Format 'yyyy.MM.dd') }
$Name = "kotodex-$Version-windows-x86_64"
$Out = Join-Path $Repo 'target\release-artifact'
$Stage = Join-Path $Out $Name

function Say($m) { Write-Host "==> $m" -ForegroundColor White }

Say 'building'
Push-Location $Repo
try {
    & cargo build --release `
        -p kotodex-server --bin kotodex-server `
        -p jp-core --bin jp-dict `
        -p jp-mine-core --bin anki-setup
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location }

if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
# target\release, not bin\: it is where setup.ps1, kotodex-windows.ps1 and
# install_root() already look, and one layout for a checkout and a zip alike is
# one thing to be wrong about.
New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'target\release') | Out-Null
# Empty, because no dictionary here is redistributable. setup.ps1 fetches them.
New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'dictionaries') | Out-Null

Say 'collecting'
foreach ($bin in 'kotodex-server', 'jp-dict', 'anki-setup') {
    Copy-Item (Join-Path $Repo "target\release\$bin.exe") (Join-Path $Stage "target\release\$bin.exe")
}

function Take($rel) {
    $dest = Join-Path $Stage $rel
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dest) | Out-Null
    Copy-Item -Recurse -Force (Join-Path $Repo $rel) $dest
}

Take 'setup.ps1'
Take 'README.md'
Take 'LICENSE'
Take 'THIRD-PARTY.md'
Take 'web-shared'
Take 'kotodex-server\static'
Take 'kotodex-server\overlay'
Take 'kotodex\kotodex-windows.ps1'
# The icon, which every shortcut and both frozen executables point at.
Take 'kotodex\icons\kotodex.ico'
# Left out on purpose: capture\, layer-overlay\, scripts\lib, kotodex-doctor.sh,
# docs\ and every Python file. None of it runs on Windows, so shipping it is a
# handful of failures for things that were never included - the same reasoning
# that keeps start-all.sh out of the Linux tarball. SudachiDict (127 MB) is out
# too; setup.ps1 fetches it and skips what is already there.
#
# kotodex-server\templates is not here either: spa.html is include_str!'d into
# the binary.

Say 'packing'
$Zip = Join-Path $Out "$Name.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Stage -DestinationPath $Zip

# Named the same way as the Linux .sha256, so `sha256sum -c` next to the
# download works on either.
$hash = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLower()
"$hash  $Name.zip" | Set-Content -Encoding ascii "$Zip.sha256"
Write-Host ("{0:N1} MB  {1}" -f ((Get-Item $Zip).Length / 1MB), $Zip)
