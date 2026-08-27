#!/usr/bin/env pwsh
# Build the Windows installer: the zip's tree, plus the two Python components
# frozen, wrapped by Inno Setup.
#
#   pwsh -File scripts\windows\build-installer.ps1 [version]
#
# Needs Rust, Python with PySide6 + websockets + pyinstaller, and Inno Setup
# (`choco install innosetup`).
#
# Why frozen rather than shipping Python: the overlay and the Textractor source
# are Python, and a friend downloading one installer has neither an interpreter
# nor PySide6. PyInstaller is what makes them ordinary executables, so the
# launcher starts all three components the same way.
#
# This file stays pure ASCII - see build-release.ps1.

param([string]$Version)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $Version) { $Version = (Get-Date -Format 'yyyy.MM.dd') }
$Out = Join-Path $Repo 'target\release-artifact'
$Work = Join-Path $Repo 'target\installer-work'

function Say($m) { Write-Host "==> $m" -ForegroundColor White }

function Find-Python {
    foreach ($c in @(
        "$env:LOCALAPPDATA\Programs\Python\Python312\python.exe",
        'python.exe'
    )) {
        $p = Get-Command $c -ErrorAction SilentlyContinue
        if ($p) { return $p.Source }
    }
    throw 'no python found'
}

function Find-ISCC {
    # Three places because the two ways of installing it disagree: choco puts it
    # under Program Files, which is what CI has, and winget installs it per-user.
    foreach ($c in @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        'ISCC.exe'
    )) {
        $p = Get-Command $c -ErrorAction SilentlyContinue
        if ($p) { return $p.Source }
    }
    throw 'no ISCC.exe found - choco install innosetup, or winget install JRSoftware.InnoSetup'
}

$Python = Find-Python
$ISCC = Find-ISCC

# ------------------------------------------------------------------- stage --

Say 'staging the release tree'
# build-release.ps1 already knows what a release is made of, and leaves the tree
# beside the zip. Reused rather than restated: two lists of what ships would
# disagree, and the zip is the one that gets tested.
& (Join-Path $Repo 'scripts\build-release.ps1') $Version
$Stage = Join-Path $Out "kotodex-$Version-windows-x86_64"
if (-not (Test-Path $Stage)) { throw "build-release.ps1 left no tree at $Stage" }

# --------------------------------------------------------------- freeze it --

# PyInstaller's PySide6 hook collects every Qt module it can find, and excluding
# the Python bindings does not drop the payload - the DLLs and the Chromium
# resources are collected as data. So the pruning below happens after the build,
# by deleting what is provably unused, rather than by asking PyInstaller not to
# take it.
$Icon = Join-Path $Repo 'kotodex\icons\kotodex.ico'

function Freeze($name, $entry, $mode, $extra) {
    Say "freezing $name"
    $args = @(
        '-m', 'PyInstaller', '--noconfirm', '--onedir', $mode,
        '--name', $name, '--icon', $Icon,
        '--distpath', (Join-Path $Work 'dist'),
        '--workpath', (Join-Path $Work 'work'),
        '--specpath', $Work
    ) + $extra + @($entry)
    & $Python @args
    if ($LASTEXITCODE -ne 0) { throw "$name did not freeze" }
}

# --windowed: it draws its own window and Qt handles its own shutdown.
Freeze 'kotodex-overlay' (Join-Path $Repo 'kotodex-server\overlay\vn-overlay.py') '--windowed' @(
    '--paths', (Join-Path $Repo 'layer-overlay'),
    '--add-data', ((Join-Path $Repo 'layer-overlay\Overlay.qml') + ';.'),
    '--add-data', ((Join-Path $Repo 'layer-overlay\OverlayWindow.qml') + ';.'),
    # Imported behind `if BACKEND == backend.WINDOWS`, which PyInstaller's static
    # analysis does see - named anyway, because losing either is a crash on the
    # first line rather than a build error.
    '--hidden-import', 'wininput',
    '--hidden-import', 'winwatch'
)

# --console, not --windowed, and the launcher hides the window: a process with no
# console receives no CTRL_C_EVENT or CTRL_BREAK_EVENT at all, and those are the
# only warning this gets that it is being shut down. It needs one to send
# Textractor's WebSocket plugin a proper close frame, which is what keeps an
# abortive disconnect from crashing Textractor itself.
Freeze 'kotodex-source' (Join-Path $Repo 'sources\textractor\vn-ws-logger.py') '--console' @()

Say 'pruning the frozen overlay'
$internal = Join-Path $Work 'dist\kotodex-overlay\_internal\PySide6'
# Chromium's DevTools resources, which are only reachable through remote
# debugging. The debug pak alone is 72 MB.
Get-ChildItem (Join-Path $internal 'resources') -Filter 'qtwebengine_devtools_resources*.pak' `
    -ErrorAction SilentlyContinue | Remove-Item -Force
# Qt's own dialog strings, in every language Qt ships. The page is Japanese and
# English and supplies its own text; what is left here is the language of a file
# picker nothing opens.
$keep = @('en', 'ja')
Get-ChildItem (Join-Path $internal 'translations') -File -ErrorAction SilentlyContinue |
    Where-Object { $keep -notcontains ($_.BaseName -replace '^.*_', '') } |
    Remove-Item -Force
Get-ChildItem (Join-Path $internal 'translations\qtwebengine_locales') -File -ErrorAction SilentlyContinue |
    Where-Object { $keep -notcontains $_.BaseName } | Remove-Item -Force

Say 'collecting into the tree'
foreach ($pair in @(@('kotodex-overlay', 'overlay'), @('kotodex-source', 'source'))) {
    $from = Join-Path $Work "dist\$($pair[0])"
    $to = Join-Path $Stage $pair[1]
    if (Test-Path $to) { Remove-Item -Recurse -Force $to }
    Copy-Item -Recurse $from $to
}

$size = [math]::Round(((Get-ChildItem -Recurse $Stage | Measure-Object Length -Sum).Sum / 1MB), 1)
Say "tree is $size MB"

# ---------------------------------------------------------------- package --

Say 'building the installer'
& $ISCC "/DStage=$Stage" "/DVersion=$Version" "/DOut=$Out" `
    (Join-Path $PSScriptRoot 'kotodex.iss')
if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }

$exe = Join-Path $Out "kotodex-$Version-windows-setup.exe"
$hash = (Get-FileHash -Algorithm SHA256 $exe).Hash.ToLower()
"$hash  $(Split-Path -Leaf $exe)" | Set-Content -Encoding ascii "$exe.sha256"
Write-Host ("{0:N1} MB  {1}" -f ((Get-Item $exe).Length / 1MB), $exe)
