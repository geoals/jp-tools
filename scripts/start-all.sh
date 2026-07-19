#!/usr/bin/env bash
#
# start-all.sh — start the full jp-tools stack with one command.
#
#   scripts/start-all.sh              start everything (asks before restarting
#                                     services that are already running)
#   scripts/start-all.sh status       show what is running
#   scripts/start-all.sh stop         stop everything
#   scripts/start-all.sh restart      restart everything, no prompts
#
# Any command takes service names to act on just those, e.g.
#   scripts/start-all.sh restart read-stats
#   scripts/start-all.sh stop yt-mine manga-mine
# A named service is restarted without asking (naming it is the confirmation).
#
# Options (for start):
#   -r, --restart    restart already-running services without asking
#   -k, --keep       never restart; leave running services alone
#   --cpu            use the CPU whisper compose file instead of GPU
#   --release        build/run the Rust servers in release mode
#
# Services managed (aliases in parens):
#   manga-ocr-service  uvicorn (.venv)            :8200  (ocr)
#   whisper-service    docker compose (gpu|cpu)   :8100  (whisper)
#   yt-mine            cargo-built binary         :3000  (yt)
#   manga-mine         cargo-built binary         :3100  (manga)
#   read-stats         cargo-built binary         :3200  (stats)
#
# Logs for the native services go to logs/<name>.log; whisper logs live in
# docker (docker logs -f whisper-service).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_DIR="$REPO_ROOT/logs"

# ---------------------------------------------------------------- settings --
MODE="ask"          # ask | restart | keep
WHISPER_FLAVOR="gpu"
CARGO_PROFILE="debug"
COMMAND="start"
SELECTED=()         # empty = all services

# Canonical service name for an argument, or empty if it isn't one.
canonical_service() {
  case "$1" in
    manga-ocr-service|ocr)   echo "manga-ocr-service" ;;
    whisper-service|whisper) echo "whisper-service" ;;
    yt-mine|yt)              echo "yt-mine" ;;
    manga-mine|manga)        echo "manga-mine" ;;
    read-stats|stats)        echo "read-stats" ;;
  esac
}

for arg in "$@"; do
  case "$arg" in
    start|stop|status) COMMAND="$arg" ;;
    restart)           COMMAND="start"; MODE="restart" ;;
    -r|--restart)      MODE="restart" ;;
    -k|--keep)         MODE="keep" ;;
    --cpu)             WHISPER_FLAVOR="cpu" ;;
    --release)         CARGO_PROFILE="release" ;;
    -h|--help)         awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' \
                         "${BASH_SOURCE[0]}"; exit 0 ;;
    *)
      svc="$(canonical_service "$arg")"
      if [[ -z "$svc" ]]; then
        echo "unknown argument: $arg (try --help)" >&2; exit 2
      fi
      SELECTED+=("$svc") ;;
  esac
done

# Naming a service is itself the confirmation to restart it.
if (( ${#SELECTED[@]} > 0 )) && [[ "$MODE" == "ask" ]]; then
  MODE="restart"
fi

# True if a service should be acted on: no selection means all of them.
selected() { # name
  (( ${#SELECTED[@]} == 0 )) && return 0
  local s
  for s in "${SELECTED[@]}"; do [[ "$s" == "$1" ]] && return 0; done
  return 1
}

WHISPER_COMPOSE="$REPO_ROOT/whisper-service/docker-compose.${WHISPER_FLAVOR}.yml"

# Ports (keep in sync with .env / service defaults)
PORT_ocr=8200
PORT_whisper=8100
PORT_ytmine=3000
PORT_mangamine=3100
PORT_readstats=3200

# ----------------------------------------------------------------- helpers --
info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m ✓ \033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m ! \033[0m %s\n' "$*"; }
fail()  { printf '\033[1;31m ✗ \033[0m %s\n' "$*" >&2; }

# True if something is listening on the TCP port. Works even when the
# listener belongs to another user (e.g. a root-owned docker process),
# where ss hides the pid.
port_listening() {
  ss -tln 2>/dev/null | awk -v p=":$1" '$4 ~ p"$" { found=1; exit } END { exit !found }'
}

# PID of the process listening on a TCP port; empty if none is visible
# (not listening, or owned by another user).
port_pid() {
  ss -tlnp 2>/dev/null | awk -v p=":$1" '
    $4 ~ p"$" { if (match($0, /pid=[0-9]+/)) { print substr($0, RSTART+4, RLENGTH-4); exit } }'
}

port_proc_name() {
  local pid; pid="$(port_pid "$1")"
  [[ -n "$pid" ]] && ps -p "$pid" -o comm= 2>/dev/null || true
}

wait_for_port() { # name port timeout_seconds
  local name="$1" port="$2" timeout="$3" waited=0
  while ! port_listening "$port"; do
    if (( waited == 15 )); then
      info "still waiting for $name on :$port..."
    fi
    if (( waited >= timeout )); then
      fail "$name did not open port $port within ${timeout}s — check its log"
      return 1
    fi
    sleep 1
    (( waited += 1 ))
  done
  ok "$name listening on :$port"
}

# Returns 0 if we should (re)start, 1 if we should leave it alone.
# A service that is not running always gets started.
should_start() { # name port
  local name="$1" port="$2" pid
  port_listening "$port" || return 0
  pid="$(port_pid "$port")"
  case "$MODE" in
    keep)    ok "$name already running on :$port (pid ${pid:-?}) — keeping"; return 1 ;;
    restart) return 0 ;;
    ask)
      if [[ ! -t 0 ]]; then
        ok "$name already running on :$port (pid ${pid:-?}) — keeping (non-interactive)"
        return 1
      fi
      local reply
      read -r -p "$name is already running on :$port (pid ${pid:-?}${pid:+, $(port_proc_name "$port")}). Restart it? [y/N] " reply
      [[ "$reply" =~ ^[Yy] ]] && return 0
      ok "keeping $name"
      return 1 ;;
  esac
}

stop_port() { # name port  — SIGTERM the listener, escalate to SIGKILL
  local name="$1" port="$2" pid tries=0
  port_listening "$port" || return 0
  pid="$(port_pid "$port")"
  if [[ -z "$pid" ]]; then
    warn "cannot stop $name: :$port is held by a process we can't see (another user?)"
    return 1
  fi
  info "stopping $name (pid $pid)"
  kill "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do
    (( tries += 1 ))
    if (( tries > 10 )); then
      warn "$name did not exit after 5s — sending SIGKILL"
      kill -9 "$pid" 2>/dev/null || true
      break
    fi
    sleep 0.5
  done
  # wait for the port to actually free up
  tries=0
  while port_listening "$port" && (( tries < 10 )); do sleep 0.5; (( tries += 1 )); done
}

start_native() { # name port workdir cmd...
  local name="$1" port="$2" workdir="$3"; shift 3
  local log="$LOG_DIR/$name.log"
  info "starting $name (log: ${log#"$REPO_ROOT"/})"
  ( cd "$workdir" && nohup "$@" >>"$log" 2>&1 & )
}

# ---------------------------------------------------------------- services --
whisper_running() {
  [[ "$(docker inspect -f '{{.State.Running}}' whisper-service 2>/dev/null)" == "true" ]]
}

start_whisper() {
  if ! command -v docker >/dev/null; then
    fail "docker not found — skipping whisper-service (yt-mine transcription will fail)"
    return 0
  fi
  if [[ "$WHISPER_FLAVOR" == "gpu" ]] && ! command -v nvidia-smi >/dev/null; then
    warn "nvidia-smi not found — falling back to CPU compose file"
    WHISPER_FLAVOR="cpu"
    WHISPER_COMPOSE="$REPO_ROOT/whisper-service/docker-compose.cpu.yml"
  fi

  if whisper_running; then
    if should_start "whisper-service" "$PORT_whisper"; then
      info "restarting whisper-service container"
      docker compose -f "$WHISPER_COMPOSE" down
    else
      return 0
    fi
  elif port_listening "$PORT_whisper"; then
    # Port busy but not our container — don't fight over it.
    local holder; holder="$(port_proc_name "$PORT_whisper")"
    warn "port $PORT_whisper is in use by ${holder:-an unknown process} (not the whisper-service container) — skipping"
    return 0
  fi

  info "starting whisper-service ($WHISPER_FLAVOR) via docker compose"
  docker compose -f "$WHISPER_COMPOSE" up -d
  # First run builds the image and downloads the model; be generous.
  wait_for_port "whisper-service" "$PORT_whisper" 300 || \
    warn "follow progress with: docker logs -f whisper-service"
}

start_ocr() {
  local dir="$REPO_ROOT/manga-ocr-service"
  if [[ ! -x "$dir/.venv/bin/uvicorn" ]]; then
    fail "manga-ocr-service/.venv missing — create it: python -m venv .venv && .venv/bin/pip install -r requirements.txt"
    return 0
  fi
  should_start "manga-ocr-service" "$PORT_ocr" || return 0
  stop_port "manga-ocr-service" "$PORT_ocr"
  start_native "manga-ocr-service" "$PORT_ocr" "$dir" \
    .venv/bin/uvicorn main:app --host 0.0.0.0 --port "$PORT_ocr"
  # Model load happens before the port opens; allow time on cold start.
  wait_for_port "manga-ocr-service" "$PORT_ocr" 120
}

start_rust() { # name port binary
  local name="$1" port="$2" bin="$REPO_ROOT/target/$CARGO_PROFILE/$3"
  should_start "$name" "$port" || return 0
  stop_port "$name" "$port"
  start_native "$name" "$port" "$REPO_ROOT" "$bin"
  wait_for_port "$name" "$port" 30
}

build_rust() {
  local pkgs=() names=() name
  for name in yt-mine manga-mine read-stats; do
    selected "$name" && { pkgs+=(-p "$name"); names+=("$name"); }
  done
  (( ${#pkgs[@]} == 0 )) && return 0
  info "building ${names[*]} ($CARGO_PROFILE)"
  local flags=()
  [[ "$CARGO_PROFILE" == "release" ]] && flags+=(--release)
  ( cd "$REPO_ROOT" && cargo build "${pkgs[@]}" "${flags[@]}" )
}

# ---------------------------------------------------------------- commands --
print_status() {
  local name port pid
  printf '%-20s %-7s %-10s %s\n' "SERVICE" "PORT" "PID" "STATE"
  for entry in "manga-ocr-service:$PORT_ocr" "whisper-service:$PORT_whisper" \
               "yt-mine:$PORT_ytmine" "manga-mine:$PORT_mangamine" \
               "read-stats:$PORT_readstats"; do
    name="${entry%%:*}" port="${entry##*:}"
    if port_listening "$port"; then
      pid="$(port_pid "$port")"
      printf '%-20s %-7s %-10s \033[1;32m%s\033[0m %s\n' \
        "$name" "$port" "${pid:-?}" "running" "${pid:+($(port_proc_name "$port"))}"
    else
      printf '%-20s %-7s %-10s \033[1;31m%s\033[0m\n' "$name" "$port" "-" "stopped"
    fi
  done
}

stop_all() {
  selected "read-stats"        && stop_port "read-stats" "$PORT_readstats"
  selected "manga-mine"        && stop_port "manga-mine" "$PORT_mangamine"
  selected "yt-mine"           && stop_port "yt-mine" "$PORT_ytmine"
  selected "manga-ocr-service" && stop_port "manga-ocr-service" "$PORT_ocr"
  if selected "whisper-service" && whisper_running; then
    info "stopping whisper-service container"
    docker compose -f "$WHISPER_COMPOSE" down
  fi
  if (( ${#SELECTED[@]} == 0 )); then
    ok "all services stopped"
  else
    ok "stopped: ${SELECTED[*]}"
  fi
}

start_all() {
  mkdir -p "$LOG_DIR"
  build_rust
  selected "whisper-service"   && start_whisper
  selected "manga-ocr-service" && start_ocr
  selected "yt-mine"    && start_rust "yt-mine" "$PORT_ytmine" "yt-mine"
  selected "manga-mine" && start_rust "manga-mine" "$PORT_mangamine" "manga-mine"
  selected "read-stats" && start_rust "read-stats" "$PORT_readstats" "read-stats"
  echo
  print_status
  echo
  selected "yt-mine"    && ok "yt-mine:     http://localhost:$PORT_ytmine"
  selected "manga-mine" && ok "manga-mine:  http://localhost:$PORT_mangamine"
  selected "read-stats" && ok "read-stats:  http://localhost:$PORT_readstats"
  return 0
}

case "$COMMAND" in
  start)  start_all ;;
  stop)   stop_all ;;
  status) print_status ;;
esac
