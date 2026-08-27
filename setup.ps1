#!/usr/bin/env pwsh
# Set Kotodex up on Windows, and say what is still missing when it ends.
#
#   powershell -ExecutionPolicy Bypass -File .\setup.ps1
#
# Through -ExecutionPolicy because Windows refuses to run an unsigned script by
# default. `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` allows it once
# and for good; a cloned file is not marked as downloaded, so it needs no
# Unblock-File.
#
# This is `setup.sh --core` for Windows: the ledger and the reader alone — the
# server, the dashboard, the dictionaries, and the reader in a browser. There is
# no overlay, no audio ring buffer and no Textractor source here, because all
# three are Linux-only today; text arrives from a source elsewhere, which is
# what the core tier is for (see sources/README.md).
#
# Re-runnable: every step checks before it acts, so a second run is a no-op and
# a run after installing something picks that up.
#
# Needs Rust (https://rustup.rs) with the MSVC toolchain. A VM behind a proxy
# that cannot reach a CRL endpoint fails the build with CRYPT_E_NO_REVOCATION_CHECK
# — `git config --global http.schannelCheckRevoke false` and `check-revoke = false`
# under `[http]` in %USERPROFILE%\.cargo\config.toml are the way out of that.

$ErrorActionPreference = 'Stop'
# Invoke-RestMethod draws a progress bar per chunk, which costs more than the
# download on a slow link.
$ProgressPreference = 'SilentlyContinue'

$Here = $PSScriptRoot
$SudachiUrl = 'http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict/sudachi-dictionary-latest-full.zip'

function Step($m) { Write-Host "`n==> $m" -ForegroundColor White }
function Say($m)  { Write-Host "    $m" }
function Good($m) { Write-Host "    " -NoNewline; Write-Host "OK " -ForegroundColor Green -NoNewline; Write-Host $m }
function Skip($m) { Write-Host "    " -NoNewline; Write-Host "-- " -ForegroundColor Yellow -NoNewline; Write-Host $m }
function Fail($m) { Write-Host "    " -NoNewline; Write-Host "XX " -ForegroundColor Red -NoNewline; Write-Host $m }

# A truncated download is worse than none: it looks installed and fails later.
#
# --ssl-no-revoke for the same reason cargo needs check-revoke = false: schannel
# treats a CRL endpoint it cannot reach as a certificate it must reject, and a VM
# behind a proxy frequently cannot reach one. The chain is still verified.
function Fetch($url, $dest, $minBytes, $label) {
    Say "downloading $label"
    $tmp = "$dest.part"
    curl.exe -fsSL --ssl-no-revoke --max-time 900 -o $tmp $url
    if (-not (Test-Path $tmp) -or (Get-Item $tmp).Length -lt $minBytes) {
        Remove-Item -Force $tmp -ErrorAction SilentlyContinue
        Fail "$label came back too small to be the real file"
        return $false
    }
    Move-Item -Force $tmp $dest
    Good $label
    return $true
}

# ------------------------------------------------------------------ build --

Step 'Binaries'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Fail 'no cargo — install Rust from https://rustup.rs and run this again'
    exit 1
}
Say 'building — the first time takes several minutes'
# The same three the Linux tarball ships. Not the whole workspace: yt-mine and
# manga-mine have no part in keeping the ledger, and they carry the expensive
# image codecs. Every one named with its own --bin, since the flag filters
# across the whole selection rather than per package.
& cargo build --release --manifest-path (Join-Path $Here 'Cargo.toml') `
    -p kotodex-server --bin kotodex-server `
    -p jp-core --bin jp-dict `
    -p jp-mine-core --bin anki-setup
if ($LASTEXITCODE -ne 0) { Fail 'build failed'; exit 1 }
Good 'built'

$JpDict = Join-Path $Here 'target\release\jp-dict.exe'
$Server = Join-Path $Here 'target\release\kotodex-server.exe'

# ----------------------------------------------------------- the tokenizer --

Step 'SudachiDict'
$Dic = Join-Path $Here 'system_full.dic'
if (Test-Path $Dic) {
    Good 'SudachiDict (system_full.dic)'
} else {
    $zip = Join-Path $Here 'sudachi-dict.zip'
    if (Fetch $SudachiUrl $zip 100000000 'SudachiDict full (~127 MB, Apache-2.0)') {
        $tmp = Join-Path $Here 'sudachi-tmp'
        Expand-Archive -Force -Path $zip -DestinationPath $tmp
        # The zip nests the dictionary under a dated directory, so it is found
        # rather than the path being guessed at.
        $found = Get-ChildItem -Recurse -Path $tmp -Filter 'system_full.dic' | Select-Object -First 1
        if ($found) { Move-Item -Force $found.FullName $Dic; Good 'SudachiDict unpacked' }
        else { Fail 'no system_full.dic in the zip' }
        Remove-Item -Recurse -Force $tmp, $zip
    }
}

# ------------------------------------------------------------ dictionaries --

Step 'Dictionaries'
$DictDir = Join-Path $Here 'dictionaries'
New-Item -ItemType Directory -Force -Path $DictDir | Out-Null

# Nothing here is redistributed: each is fetched from whoever publishes it, at
# the version they publish today. `source_path` is the cache key, so a second
# copy under a second name is a duplicate row rather than a no-op — which is why
# what is already imported counts as present.
$imported = if (Test-Path $JpDict) { (& $JpDict list 2>$null) -join "`n" } else { '' }

function Want($zipName, $match, $label) {
    $zip = Join-Path $script:DictDir $zipName
    if (Test-Path $zip) { Good "$label - already in dictionaries\"; return $false }
    if ($script:imported -imatch $match) { Good "$label - already imported"; return $false }
    return $true
}

# Both are free and neither is optional in practice: with no definitions the
# popup is empty, and with no ranks nothing is underlined or ordered.
if (Want 'jitendex-yomitan.zip' 'jitendex' 'Jitendex') {
    # The releases/latest/download redirect rather than the API: unauthenticated
    # api.github.com allows 60 requests an hour per address, and a VM behind a
    # company NAT shares that with everyone else on it. Still resolved rather
    # than pinned - a stale pin is a dictionary that quietly stops existing.
    Fetch 'https://github.com/stephenmk/stephenmk.github.io/releases/latest/download/jitendex-yomitan.zip' `
        (Join-Path $DictDir 'jitendex-yomitan.zip') 10000000 `
        'Jitendex - Japanese-English (~39 MB, CC BY-SA 4.0)' | Out-Null
}

# jiten.moe ranks the media people actually read.
if (Want 'jiten-frequency.zip' 'frequency' 'A frequency list') {
    Fetch 'https://api.jiten.moe/api/frequency-list/download' `
        (Join-Path $DictDir 'jiten-frequency.zip') 3000000 `
        'Jiten frequency list - ranks fiction (~8 MB)' | Out-Null
}

# Asked of the directory rather than of sync's exit code: sync succeeds with
# nothing to do, so reporting off the exit code called a failed download an
# import.
$zips = @(Get-ChildItem -Path $DictDir -Filter '*.zip' -ErrorAction SilentlyContinue)
if ($zips.Count -eq 0 -and -not $imported) {
    Fail 'no dictionaries - the popup will be empty and nothing will be ranked'
    Say 'download them by hand into dictionaries\ and run this again:'
    Say '  https://jitendex.org'
    Say '  https://api.jiten.moe/api/frequency-list/download'
} else {
    Say 'importing what is in dictionaries\ - the first import takes a few minutes'
    & $JpDict sync
    if ($LASTEXITCODE -ne 0) { Fail 'jp-dict sync failed' } else { Good 'dictionaries imported' }
}

# ------------------------------------------------------------------ doctor --

Step 'Checking with the server'
# The probes are answered by the server, so they are unanswerable while it is
# down — and a fresh install has never started it. Started only for this check
# and stopped again, leaving the machine as this run found it.
$state = $null
$startedHere = $null
$log = Join-Path $Here 'setup-server.log'
try { $state = Invoke-RestMethod -TimeoutSec 3 'http://localhost:3200/api/reader/state' } catch {}
if ($state) {
    Good 'already running'
} else {
    # Output kept, because a server that never answers has said why somewhere and
    # a hidden window throws it away.
    $startedHere = Start-Process -PassThru -WindowStyle Hidden -FilePath $Server `
        -RedirectStandardOutput $log -RedirectStandardError "$log.err"
    Say 'waiting for the first boot - it recounts the line stream and loads the tokenizer'
    foreach ($try in 1..30) {
        Start-Sleep -Seconds 2
        if ($startedHere.HasExited) { break }
        try { $state = Invoke-RestMethod -TimeoutSec 3 'http://localhost:3200/api/reader/state'; break } catch {}
    }
    if ($state) {
        Good 'started for the check'
    } else {
        Fail 'the server did not answer in a minute'
        Say "its output is in $log and $log.err"
        Get-Content -Tail 15 "$log.err" -ErrorAction SilentlyContinue | ForEach-Object { Say "  $_" }
    }
}

if ($state -and $state.capabilities) {
    foreach ($name in ($state.capabilities.PSObject.Properties.Name | Sort-Object)) {
        $c = $state.capabilities.$name
        if ($c.ok) { Good "$name - $($c.detail)" }
        else {
            Skip "$name - $($c.detail)"
            if ($c.fix) { Say "  $($c.fix)" }
        }
    }
}

# Stopped by the handle this script owns, never by matching a process name: the
# name is shared with any other instance on this machine.
if ($startedHere -and -not $startedHere.HasExited) {
    Stop-Process -Id $startedHere.Id -Force
    Say 'stopped the server again'
}

Step 'Done'
Say 'start it with:'
Say "  $Server"
Say 'then open http://localhost:3200'
Say ''
Say 'no text arrives on its own here - a source has to post to POST /api/lines.'
Say 'see sources\README.md.'
