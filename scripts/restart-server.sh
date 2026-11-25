#!/usr/bin/env bash
# Rebuild and restart the cadastre server on a Linux host (10.10.10.11). Where it is a systemd service
# (scripts/install-service.sh) the service is restarted (asks for sudo); elsewhere it is run as
# `nohup ./target/<profile>/cadastre-server >> server.log 2>&1 &`.
#
#   scripts/restart-server.sh            # git pull, build, restart, check /health
#   scripts/restart-server.sh --no-pull  # build and restart what is checked out
#
# Builds BEFORE stopping, so a failed build leaves the running server alone and the gap is seconds.
# The new server gets the running one's settings (its environment, read from /proc) and working
# directory; with no server running it falls back to ./.env and refuses to start without one.
# Not for development: use scripts/run-server.sh there.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(pwd)

pull=1
for a in "$@"; do
  case "$a" in
    --no-pull) pull=0 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

. scripts/lib-server.sh

if [[ -e "$UNIT_FILE" ]]; then
  strays=$(stray_pids)
  [[ -z "$strays" ]] || { echo "a cadastre-server outside the service is running (pid $strays): stop it first" >&2; exit 1; }
  if (( pull )); then say "git pull"; git pull --ff-only; fi
  say "build (debug), the running server keeps serving"
  cargo build -p cadastre-server --bin cadastre-server
  say "restart $UNIT"
  bind=$(sudo sed -nE "s/^CADASTRE_BIND=['\"]?([^'\"]*)['\"]?$/\1/p" "$ENV_FILE")
  if [[ -e "$LIB_DIR/cadastre-server" ]]; then sudo cp -p "$LIB_DIR/cadastre-server" "$LIB_DIR/cadastre-server.previous"; fi
  sudo install -m 755 target/debug/cadastre-server "$LIB_DIR/cadastre-server"
  since=$(date '+%Y-%m-%d %H:%M:%S')
  sudo systemctl reset-failed "$UNIT" || true
  sudo systemctl restart "$UNIT" || true
  if wait_healthy "$(health_url "$bind")" 60 && systemctl is-active -q "$UNIT"; then
    echo "up: pid $(service_pid), log: journalctl -u $UNIT"
    exit 0
  fi
  echo "NOT answering after 60 s:" >&2
  sudo journalctl -u "$UNIT" --since "$since" -n 40 --no-pager >&2 || true
  echo "back to the previous build: sudo cp -p $LIB_DIR/cadastre-server.previous $LIB_DIR/cadastre-server && sudo systemctl restart $UNIT" >&2
  exit 1
fi

# The running server, if any: by process name, not command line (an editor or a shell whose
# command line merely mentions the path must not be taken for it).
pid=$(pgrep -x cadastre-server | head -1 || true)
profile=debug
settings=()
workdir=$repo
if [[ -n "$pid" ]]; then
  exe="$(readlink "/proc/$pid/exe" 2>/dev/null || true) $(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null || true)"
  [[ "$exe" == */target/release/* ]] && profile=release
  [[ -r "/proc/$pid/environ" ]] || { echo "cannot read /proc/$pid/environ: run as the user the server runs as" >&2; exit 1; }
  # Its settings stay in memory; nothing with passwords is written to disk.
  read_settings "$pid"
  workdir=$(readlink "/proc/$pid/cwd" 2>/dev/null || echo "$repo")
  echo "running: pid $pid, $profile build, ${#settings[@]} settings, in $workdir"
else
  [[ -f .env ]] || { echo "no cadastre-server running and no .env here: not starting without production settings" >&2; exit 1; }
  while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    line=${line#export }
    settings+=("$line")
  done < .env
  echo "no cadastre-server running; starting a $profile build with ${#settings[@]} settings from .env"
fi
[[ " ${settings[*]} " == *" CADASTRE_DATABASE_URL="* || " ${settings[*]} " == *" DATABASE_URL="* ]] \
  || { echo "no database setting found: refusing to start" >&2; exit 1; }

if (( pull )); then
  say "git pull"
  git pull --ff-only
fi
say "build ($profile), the running server keeps serving"
flag=(); [[ $profile == release ]] && flag=(--release)
cargo build -p cadastre-server --bin cadastre-server "${flag[@]}"
bin="$repo/target/$profile/cadastre-server"

if [[ -n "$pid" ]]; then
  say "stop pid $pid"
  kill "$pid"
  for _ in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
  if kill -0 "$pid" 2>/dev/null; then echo "still running after 20 s, killing"; kill -9 "$pid"; fi
fi

say "start"
log="$workdir/server.log"
start_line=1; [[ -f "$log" ]] && start_line=$(( $(wc -l < "$log") + 1 ))
( cd "$workdir" && env "${settings[@]}" nohup "$bin" >> "$log" 2>&1 & )

bind=0.0.0.0:8080
for kv in "${settings[@]}"; do [[ $kv == CADASTRE_BIND=* ]] && bind=${kv#CADASTRE_BIND=}; done
health=$(health_url "$bind")
say "check $health"
if wait_healthy "$health" 60; then
  echo "up: pid $(pgrep -x cadastre-server | head -1), log $log"
  tail -n +"$start_line" "$log" | grep -E 'registry-admin|ERROR|WARN' || true
  exit 0
fi
echo "NOT answering after 60 s. Log since start ($log):" >&2
tail -n +"$start_line" "$log" | tail -40 >&2
exit 1
