#!/usr/bin/env bash
# Rebuild and restart the cadastre server on a Linux host run the production way (10.10.10.11):
# `nohup ./target/<profile>/cadastre-server >> server.log 2>&1 &` from the repo root.
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
    -h|--help) sed -n '2,11p' "$0"; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

say() { printf '\n== %s\n' "$*"; }

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
  while IFS= read -r -d '' kv; do
    case "$kv" in CADASTRE_*|RUST_LOG=*|RUST_BACKTRACE=*|DATABASE_URL=*) settings+=("$kv") ;; esac
  done < "/proc/$pid/environ"
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
port=${bind##*:}
say "check http://127.0.0.1:$port/health"
for _ in $(seq 1 60); do
  if curl -fs -m 2 "http://127.0.0.1:$port/health" >/dev/null; then
    echo "up: pid $(pgrep -x cadastre-server | head -1), log $log"
    tail -n +"$start_line" "$log" | grep -E 'registry-admin|ERROR|WARN' || true
    exit 0
  fi
  sleep 1
done
echo "NOT answering after 60 s. Log since start ($log):" >&2
tail -n +"$start_line" "$log" | tail -40 >&2
exit 1
