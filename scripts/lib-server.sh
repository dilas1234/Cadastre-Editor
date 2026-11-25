# Shared by restart-server.sh and install-service.sh.
UNIT=cadastre-server.service
UNIT_FILE=/etc/systemd/system/$UNIT
ENV_FILE=/etc/cadastre/cadastre-server.env
LIB_DIR=/usr/local/lib/cadastre

say() { printf '\n== %s\n' "$*"; }

# The /health address of a CADASTRE_BIND value: bound to one address (10.10.10.11) the server does
# not answer on 127.0.0.1.
health_url() {
  local bind=${1:-0.0.0.0:8080} host
  host=${bind%:*}
  case "$host" in ''|0.0.0.0|'[::]') host=127.0.0.1 ;; esac
  echo "http://$host:${bind##*:}/health"
}

wait_healthy() { # url, seconds
  local i
  for ((i = 0; i < $2; i++)); do
    curl -fs --noproxy '*' -m 2 "$1" >/dev/null && return 0
    sleep 1
  done
  return 1
}

# The main process of the service, or 0.
service_pid() { systemctl show "$UNIT" -p MainPID --value 2>/dev/null || echo 0; }

# cadastre-server processes that are not the service's.
stray_pids() {
  local main p
  main=$(service_pid)
  for p in $(pgrep -x cadastre-server || true); do [[ "$p" == "$main" ]] || echo "$p"; done
}

# The server's settings among a process's environment (NUL-separated, as in /proc/PID/environ).
read_settings() { # pid; fills the array `settings`
  local kv
  settings=()
  while IFS= read -r -d '' kv; do
    case "$kv" in CADASTRE_*|RUST_LOG=*|RUST_BACKTRACE=*|DATABASE_URL=*) settings+=("$kv") ;; esac
  done < "/proc/$1/environ"
}

# One setting as a line of a systemd EnvironmentFile.
env_line() {
  local k=${1%%=*} v=${1#*=}
  [[ "$v" != *$'\n'* ]] || { echo "$k has a line break: cannot be stored" >&2; return 1; }
  if [[ "$v" != *"'"* && "$v" != *\\* ]]; then
    printf "%s='%s'\n" "$k" "$v"
  else
    v=${v//\\/\\\\}; v=${v//\"/\\\"}; v=${v//\$/\\\$}; v=${v//\`/\\\`}
    printf '%s="%s"\n' "$k" "$v"
  fi
}
