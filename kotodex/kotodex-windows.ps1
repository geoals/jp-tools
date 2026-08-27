#!/usr/bin/env pwsh
# Start Kotodex and open the reader. What the Start Menu entry runs.
#
#   powershell -ExecutionPolicy Bypass -File kotodex\kotodex-windows.ps1
#
# Three components, the same three the Qt launcher owns on Linux: kotodex-server,
# the Textractor source and the overlay. Capture is the fourth there and is not on
# this platform. What it keeps from that launcher is the rule that matters:
# **adopt, never duplicate**. Each is probed before it is started, so running this
# twice opens a second browser tab rather than a second server against a locked
# database, a second overlay drawn over the first, or a second source logging
# every line twice.
#
# It does not stop what it did not start, and it stops nothing: everything is left
# running so the reader can be reopened without a fresh boot. Stop-Process on the
# pids this prints, or log out.

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$Repo = Split-Path -Parent $PSScriptRoot
$Server = Join-Path $Repo 'target\release\kotodex-server.exe'
# Both frozen with PyInstaller, because a release ships no Python. Absent from a
# git checkout, where the .py beside them is what runs instead.
$Overlay = Join-Path $Repo 'overlay\kotodex-overlay.exe'
$Source = Join-Path $Repo 'source\kotodex-source.exe'
$Url = 'http://localhost:3200'
$LogDir = Join-Path $env:LOCALAPPDATA 'kotodex'
$Log = Join-Path $LogDir 'kotodex-server.log'

function Answering {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "$Url/api/reader/state" | Out-Null; return $true }
    catch { return $false }
}

# Adopt, never duplicate - the rule the Qt launcher keeps on Linux. A second
# overlay would draw over the first, and a second source would log every line
# twice. By its own name rather than a shared one: these two executables are this
# application's, unlike python.exe, which anything on the machine may be running.
function Start-Once($exe, $name) {
    if (-not (Test-Path $exe)) { return }
    if (Get-Process -Name $name -ErrorAction SilentlyContinue) {
        Write-Host "$name is already running"
        return
    }
    $log = Join-Path $LogDir "$name.log"
    Start-Process -WindowStyle Hidden -FilePath $exe `
        -RedirectStandardOutput $log -RedirectStandardError "$log.err"
    Write-Host "started $name, logging to $log"
}

if (Answering) {
    Write-Host "Kotodex is already running - opening $Url"
} else {
    if (-not (Test-Path $Server)) {
        Write-Host "No server at $Server - run setup.ps1 first" -ForegroundColor Red
        exit 1
    }
    New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
    $p = Start-Process -PassThru -WindowStyle Hidden -FilePath $Server `
        -RedirectStandardOutput $Log -RedirectStandardError "$Log.err"
    Write-Host "started kotodex-server (pid $($p.Id)), logging to $Log"
    # The first boot recounts the line stream and loads the tokenizer, so the
    # browser is held back rather than opening on a connection refused.
    foreach ($try in 1..30) {
        Start-Sleep -Seconds 1
        if ($p.HasExited) {
            Write-Host "the server exited - see $Log.err" -ForegroundColor Red
            Get-Content -Tail 15 "$Log.err" -ErrorAction SilentlyContinue
            exit 1
        }
        if (Answering) { break }
    }
    if (-not (Answering)) {
        Write-Host "the server did not answer in 30s - opening anyway, see $Log" -ForegroundColor Yellow
    }
}

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
# The source before the overlay: it is what puts lines in the feed, and it costs
# nothing while Textractor is absent - it retries the connection until one answers.
Start-Once $Source 'kotodex-source'
Start-Once $Overlay 'kotodex-overlay'

# The dashboard as well as the overlay, because the work being read and which
# window to track are set there, and the overlay has no page for either.
Start-Process $Url
