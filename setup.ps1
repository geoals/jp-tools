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
# This is `setup.sh` for Windows: the server, the dashboard, the dictionaries and
# the reader, and in a release the overlay and the Textractor source beside them,
# frozen. What is not here is the audio ring buffer, so a card gets no voiceline -
# that pipeline is Linux-only.
#
# It sets nothing up for those two itself: they are ordinary executables in the
# install, started by the launcher. This downloads what all of them need.
#
# Re-runnable: every step checks before it acts, so a second run is a no-op and
# a run after installing something picks that up.
#
# The release zip ships the binaries. A git checkout builds them instead, and
# needs Rust (https://rustup.rs) with the MSVC toolchain.
#
# A machine that cannot reach a certificate revocation list - a proxy, or a
# firewall allowing only 443, since a CRL is served over plain HTTP - fails the
# cargo build with CRYPT_E_NO_REVOCATION_CHECK. Cargo and git each need telling
# once:
#
#   git config --global http.schannelCheckRevoke false
#   # and check-revoke = false under [http] in %USERPROFILE%\.cargo\config.toml
#
# The downloads below handle it themselves, and say so when they do.

# -NoShortcut when the installer is running this: it owns the Start Menu entry, so
# that the uninstaller knows about it and takes it away again.
param([switch]$NoShortcut)

$ErrorActionPreference = 'Stop'
# Invoke-RestMethod draws a progress bar per chunk, which costs more than the
# download on a slow link.
$ProgressPreference = 'SilentlyContinue'
# So the Japanese and the dashes in what the server says reach the terminal
# intact. The decoding on the way in matters too - see GetJson. This file itself
# stays ASCII: Windows PowerShell reads an unsigned script with no byte-order
# mark as the machine's ANSI codepage, so a dash in a string here would mangle.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$Here = $PSScriptRoot
$SudachiUrl = 'http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict/sudachi-dictionary-latest-full.zip'

function Step($m) { Write-Host "`n==> $m" -ForegroundColor White }
function Say($m)  { Write-Host "    $m" }
function Good($m) { Write-Host "    " -NoNewline; Write-Host "OK " -ForegroundColor Green -NoNewline; Write-Host $m }
function Skip($m) { Write-Host "    " -NoNewline; Write-Host "-- " -ForegroundColor Yellow -NoNewline; Write-Host $m }
function Fail($m) { Write-Host "    " -NoNewline; Write-Host "XX " -ForegroundColor Red -NoNewline; Write-Host $m }

# Windows PowerShell decodes a response body as ISO-8859-1 unless the server
# names a charset in Content-Type, and axum sends application/json without one -
# so the em dash in the server's own fix lines arrives as mojibake. Setting the
# console encoding cannot help, because the string is already wrong by the time
# it is printed. Decoded from the bytes here instead.
function GetJson($url, $timeout) {
    $r = Invoke-WebRequest -UseBasicParsing -TimeoutSec $timeout $url
    $bytes = if ($r.RawContentStream) { $r.RawContentStream.ToArray() }
             else { [System.Text.Encoding]::UTF8.GetBytes($r.Content) }
    [System.Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
}

# A truncated download is worse than none: it looks installed and fails later.
function Fetch($url, $dest, $minBytes, $label) {
    Say "downloading $label"
    $tmp = "$dest.part"
    curl.exe -fsSL --max-time 900 -o $tmp $url
    # 35 is curl's TLS handshake failure, which on Windows is usually schannel
    # refusing a certificate whose revocation list it could not reach - a CRL is
    # served over plain HTTP, and a firewall allowing only 443 breaks the lookup
    # for everything. Retried without that one question rather than passing
    # --ssl-no-revoke from the start: the signature, the chain, the hostname and
    # the expiry are all still checked either way, but every other machine keeps
    # the revocation check this one cannot make.
    if ($LASTEXITCODE -eq 35) {
        Skip 'TLS handshake failed - retrying without the certificate revocation check'
        Say 'the network cannot reach a CRL endpoint (a proxy, or port 80 blocked)'
        curl.exe -fsSL --ssl-no-revoke --max-time 900 -o $tmp $url
    }
    if ($LASTEXITCODE -ne 0) {
        Remove-Item -Force $tmp -ErrorAction SilentlyContinue
        Fail "$label did not download - curl exit $LASTEXITCODE"
        return $false
    }
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
$JpDict = Join-Path $Here 'target\release\jp-dict.exe'
$Server = Join-Path $Here 'target\release\kotodex-server.exe'

# A checkout builds whenever cargo is here, not only when the exes are missing:
# one that has moved on leaves exes that still run and answer with stale
# behaviour, which is harder to see than a missing one. Cargo is a no-op when
# they are current.
#
# Cargo.toml is what tells a checkout from the zip. The zip ships the exes and
# no source, so asking for cargo alone would run a build with nothing to build
# on any machine that happens to have rustup.
$HasCargo = [bool](Get-Command cargo -ErrorAction SilentlyContinue)
if ((Test-Path (Join-Path $Here 'Cargo.toml')) -and $HasCargo) {
    Say 'building - the first time takes several minutes'
    # The same three the Linux tarball ships. Not the whole workspace: yt-mine
    # and manga-mine have no part in keeping the ledger, and they carry the
    # expensive image codecs. Every one named with its own --bin, since the flag
    # filters across the whole selection rather than per package.
    & cargo build --release --manifest-path (Join-Path $Here 'Cargo.toml') `
        -p kotodex-server --bin kotodex-server `
        -p jp-core --bin jp-dict `
        -p jp-mine-core --bin anki-setup
    if ($LASTEXITCODE -ne 0) { Fail 'build failed'; exit 1 }
    Good 'built'
} elseif ((Test-Path $Server) -and (Test-Path $JpDict)) {
    Good 'shipped binaries'
} elseif (Test-Path (Join-Path $Here 'Cargo.toml')) {
    Fail 'no binaries, and no cargo to build this checkout with'
    Say 'install Rust: https://rustup.rs'
    exit 1
} else {
    Fail 'this release is missing its binaries'
    Say 'download the zip again, or build from a git checkout'
    exit 1
}

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
# copy under a second name is a duplicate row rather than a no-op - which is why
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

# Pitch is the one thing a monolingual gloss does not carry, and the zip is a
# megabyte. Pinned to no version because the repo is archived - its one release
# is what latest/download resolves to.
if (Want 'kanjium_pitch_accents.zip' 'pitch' 'A pitch dictionary') {
    Fetch 'https://github.com/toasted-nutbread/yomichan-pitch-accent-dictionary/releases/latest/download/kanjium_pitch_accents.zip' `
        (Join-Path $DictDir 'kanjium_pitch_accents.zip') 500000 `
        'Kanjium pitch accents (~1 MB, CC BY-SA 4.0)' | Out-Null
}

# Asked of the directory rather than of sync's exit code: sync succeeds with
# nothing to do, so reporting off the exit code would call a failed download an
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

# ------------------------------------------------------------- application --

Step 'Start Menu entry'
# The same Qt launcher Linux runs, packaged into an .exe: it owns the server, the
# source and the overlay, and leaves a tray icon behind. `kotodex\host_windows.py`
# is the whole of what it does differently here.
$Launcher = Join-Path $Here 'app\kotodex.exe'
if ($NoShortcut) {
    Skip 'the installer owns the Start Menu entry'
} elseif (-not (Test-Path $Launcher)) {
    # The zip carries no packaged .exes - they are the installer's. What is here
    # still serves the reader in a browser.
    Skip 'no launcher in this tree - start target\release\kotodex-server.exe'
} else {
    $StartMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
    $Lnk = Join-Path $StartMenu 'Kotodex.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $sc = $shell.CreateShortcut($Lnk)
    $sc.TargetPath = $Launcher
    $sc.WorkingDirectory = $Here
    $sc.Description = 'Kotodex - the ledger, the reader and the overlay'
    $sc.Save()
    Good "Kotodex in the Start Menu"
    Say 'it starts the server, the source and the overlay, and sits in the tray'
}

# ------------------------------------------------------------------ doctor --

Step 'Checking with the server'
# The probes are answered by the server, so they are unanswerable while it is
# down - and a fresh install has never started it. Started only for this check
# and stopped again, leaving the machine as this run found it.
$state = $null
$startedHere = $null
$log = Join-Path $Here 'setup-server.log'
# Numeric, not `localhost`: that name resolves to `::1` first here and the server
# binds IPv4, so every probe below would spend its whole timeout against a server
# that is answering. The launcher's own probe is numeric for the same reason.
try { $state = GetJson 'http://127.0.0.1:3200/api/reader/state' 3 } catch {}
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
        try { $state = GetJson 'http://127.0.0.1:3200/api/reader/state' 3; break } catch {}
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
