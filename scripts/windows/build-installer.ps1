#!/usr/bin/env pwsh
# Build the Windows installer: the zip's tree, plus the three Python components
# packaged into .exes, wrapped by Inno Setup.
#
#   pwsh -File scripts\windows\build-installer.ps1 [version]
#
# Needs Rust, Python with PySide6 + websockets + pyinstaller, and Inno Setup
# (`choco install innosetup`).
#
# Why packaged rather than shipping Python: the overlay and the Textractor source
# are Python, and a friend downloading one installer has neither an interpreter
# nor PySide6. PyInstaller is what makes them ordinary executables, so the
# launcher starts all three components the same way.
#
# What goes into them, and what is left out, is kotodex.spec.
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

# ------------------------------------------------------ build the two halves --

# Cargo and PyInstaller share no input and no output directory, so they run at the
# same time. Cargo's output is held in a log and printed once it is done, rather
# than interleaved with PyInstaller's.
#
# build-release.ps1 already knows what a release is made of, and leaves the tree
# beside the zip. Reused rather than restated: two lists of what ships would
# disagree, and the zip is the one that gets tested. It owns $Stage, so nothing
# here may touch $Stage until it has finished.
Say 'building the release tree and packaging the Python components'
New-Item -ItemType Directory -Force -Path $Work | Out-Null
$rustLog = Join-Path $Work 'cargo.log'

# A process of its own rather than `Start-Job`: cargo writes its progress to
# stderr, and inside a job that arrives as an error record, which build-release's
# own `$ErrorActionPreference = 'Stop'` turns into a failure on the first
# `Compiling` line. A separate process keeps its exit code as the only verdict.
# `$PID`'s own path, so this runs under whichever PowerShell started it.
$rust = Start-Process -FilePath (Get-Process -Id $PID).Path -PassThru -NoNewWindow `
    -RedirectStandardOutput $rustLog -RedirectStandardError "$rustLog.err" `
    -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', (Join-Path $Repo 'scripts\build-release.ps1'), $Version
    )
# Reading `.Handle` while it is alive is what keeps `.ExitCode` readable once it is
# not. Without it Windows PowerShell leaves ExitCode empty and every build looks
# like a failure.
$null = $rust.Handle

try {
    & $Python -m PyInstaller --noconfirm `
        --distpath (Join-Path $Work 'dist') --workpath (Join-Path $Work 'work') `
        (Join-Path $PSScriptRoot 'kotodex.spec')
    if ($LASTEXITCODE -ne 0) { throw 'PyInstaller failed' }
} finally {
    # Waited for even when packaging threw, so a cargo error is not lost behind it.
    $rust.WaitForExit()
    Get-Content $rustLog, "$rustLog.err" -ErrorAction SilentlyContinue
}
if ($rust.ExitCode -ne 0) { throw 'build-release.ps1 failed' }

$Stage = Join-Path $Out "kotodex-$Version-windows-x86_64"
if (-not (Test-Path $Stage)) { throw "build-release.ps1 left no tree at $Stage" }

Say 'collecting into the tree'
# One directory holding all three .exes and the single copy of Qt they share. It
# sits one level under the install root, which is what `host_windows.ROOT`
# resolves by taking the launcher's parent's parent.
$app = Join-Path $Stage 'app'
if (Test-Path $app) { Remove-Item -Recurse -Force $app }
Copy-Item -Recurse (Join-Path $Work 'dist\app') $app

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
