#!/usr/bin/env pwsh
# Start Kotodex and open the reader. What the Start Menu entry runs.
#
#   powershell -ExecutionPolicy Bypass -File kotodex\kotodex-windows.ps1
#
# The Linux launcher owns three components; this owns one, because capture and
# the overlay are not here. What it keeps from that launcher is the rule that
# matters: **adopt, never duplicate**. The server is probed before it is started,
# so running this twice opens a second browser tab and not a second server -
# which would find the database locked by the first.
#
# It does not stop what it did not start, and it stops nothing: the server is
# left running so the reader can be reopened without a fresh boot. Close it from
# the tray of its own console window, or Stop-Process on the pid this prints.

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$Repo = Split-Path -Parent $PSScriptRoot
$Server = Join-Path $Repo 'target\release\kotodex-server.exe'
$Url = 'http://localhost:3200'
$Log = Join-Path $env:LOCALAPPDATA 'kotodex\kotodex-server.log'

function Answering {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 "$Url/api/reader/state" | Out-Null; return $true }
    catch { return $false }
}

if (Answering) {
    Write-Host "Kotodex is already running - opening $Url"
} else {
    if (-not (Test-Path $Server)) {
        Write-Host "No server at $Server - run setup.ps1 first" -ForegroundColor Red
        exit 1
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $Log) | Out-Null
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

Start-Process $Url
