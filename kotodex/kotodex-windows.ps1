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
# Numeric for the probe, and the name only for the browser. `localhost` resolves
# to `::1` first here, the server binds IPv4, and a connection to `::1` on this
# platform times out rather than being refused - while Invoke-WebRequest, unlike
# a browser or python, never tries the second address. Every probe therefore spent
# its whole timeout and the wait below ran to its limit against a server that had
# been answering for ten seconds. The tab keeps the name Yomitan is configured
# against.
$Probe = 'http://127.0.0.1:3200'
$LogDir = Join-Path $env:LOCALAPPDATA 'kotodex'
$Log = Join-Path $LogDir 'kotodex-server.log'

function Answering {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "$Probe/api/reader/state" | Out-Null; return $true }
    catch { return $false }
}

# Adopt, never duplicate - the rule the Qt launcher keeps on Linux. A second
# overlay would draw over the first, and a second source would log every line
# twice. By its own name rather than a shared one: these two executables are this
# application's, unlike python.exe, which anything on the machine may be running.
function Start-Once($exe, $name) {
    if (-not (Test-Path $exe)) {
        Write-Host "$name is missing at $exe" -ForegroundColor Red
        return
    }
    if (Get-Process -Name $name -ErrorAction SilentlyContinue) {
        Write-Host "$name is already running"
        return
    }
    Start-Hidden $exe (Join-Path $LogDir "$name.log")
    $name
}

# Whether what was launched is still there, because the interesting failure is a
# component that starts and dies: without this the launcher reports success and
# the reader sees nothing at all.
#
# Both are asked once, after a wait they share. A wait each, in series, was three
# seconds of the launcher doing nothing - and the wait for the server below is
# already longer than the one this needs.
function Survived($names) {
    foreach ($name in $names) {
        $log = Join-Path $LogDir "$name.log"
        if (Get-Process -Name $name -ErrorAction SilentlyContinue) {
            Write-Host "started $name, logging to $log"
        } else {
            Write-Host "$name exited immediately - see $log.err" -ForegroundColor Red
            Get-Content -Tail 10 "$log.err" -ErrorAction SilentlyContinue |
                ForEach-Object { Write-Host "  $_" }
        }
    }
}

# Through cmd, and never with Start-Process's own redirection: redirecting makes
# PowerShell start the child without ShellExecute, which hands it *this* console -
# so closing the launcher's window sends the whole set a close event and stops the
# server. cmd gets its own hidden console and does the redirection inside it.
function Start-Hidden($exe, $log) {
    Start-Process -WindowStyle Hidden -FilePath 'cmd.exe' `
        -ArgumentList "/d /s /c """"$exe"" > ""$log"" 2>&1"""
}

New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

$serverWasUp = Answering
if ($serverWasUp) {
    Write-Host "Kotodex is already running - opening $Url"
} elseif (-not (Test-Path $Server)) {
    Write-Host "No server at $Server - run setup.ps1 first" -ForegroundColor Red
    exit 1
} else {
    Start-Hidden $Server $Log
    Write-Host "started kotodex-server, logging to $Log"
}

# The other two before the wait below, not after it. The overlay is a frozen Qt
# application over half a gigabyte of Chromium, and on a cold first run Defender
# reads all of it - a minute is normal. Starting it while the server is still
# counting its line stream spends both waits at once, and the overlay is built for
# a server that is not up yet: it retries the page, backing off, rather than
# showing an error. Each is a no-op when it is already running.
$launched = @(Start-Once $Source 'kotodex-source'; Start-Once $Overlay 'kotodex-overlay')

if (-not $serverWasUp) {
    # The first boot recounts the line stream and loads the tokenizer, so the
    # browser is held back rather than opening on a connection refused.
    # By name, because what was started is cmd's child rather than this script's -
    # the price of not handing it this console. So absence has two meanings, and
    # only the second is a failure: cmd has not spawned it yet, or it has died.
    # Treating the first as the second exited here before the overlay was started.
    $seen = $false
    foreach ($try in 1..30) {
        Start-Sleep -Seconds 1
        if (Get-Process -Name 'kotodex-server' -ErrorAction SilentlyContinue) {
            $seen = $true
        } elseif ($seen) {
            Write-Host "the server exited - see $Log.err" -ForegroundColor Red
            Get-Content -Tail 15 "$Log.err" -ErrorAction SilentlyContinue
            exit 1
        }
        if (Answering) { break }
    }
    if (-not (Answering)) {
        Write-Host "the server did not answer in 30s - opening anyway, see $Log" -ForegroundColor Yellow
    }
} elseif ($launched) {
    Start-Sleep -Milliseconds 1500
}

Survived $launched

# The dashboard as well as the overlay, because the work being read and which
# window to track are set there, and the overlay has no page for either.
Start-Process $Url
