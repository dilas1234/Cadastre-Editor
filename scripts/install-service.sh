#!/usr/bin/env bash
# Make the running cadastre server a systemd service (once per host, e.g. 10.10.10.11): it starts at
# boot, and scripts/restart-server.sh then restarts the service. Run as the user the server runs as,
# from the checkout to deploy, while the server is running: its settings are taken from it.
#
#   scripts/install-service.sh          # show what would be done
#   scripts/install-service.sh --yes    # do it (asks for sudo); a short interruption for users
#
# If the service does not answer, it is removed and the old server is started again.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(pwd)
. scripts/lib-server.sh
case "${1:-}" in --yes|'') ;; *) sed -n '2,9p' "$0"; exit 2 ;; esac

[[ $EUID -ne 0 ]] || { echo "run as the user the server runs as, not root" >&2; exit 1; }
[[ ! -e "$UNIT_FILE" ]] || { echo "$UNIT_FILE exists already: use scripts/restart-server.sh" >&2; exit 1; }
mapfile -t pids < <(pgrep -x cadastre-server || true)
[[ ${#pids[@]} -eq 1 ]] || { echo "need exactly one running cadastre-server, found ${#pids[@]}: ${pids[*]:-none}" >&2; exit 1; }
pid=${pids[0]}
[[ -r "/proc/$pid/environ" ]] || { echo "cannot read /proc/$pid/environ: run as the user the server runs as" >&2; exit 1; }
read_settings "$pid"
workdir=$(readlink "/proc/$pid/cwd")
exe=$(readlink "/proc/$pid/exe"); profile=debug; [[ "$exe" == */target/release/* ]] && profile=release
bind=0.0.0.0:8080 db=
for kv in "${settings[@]}"; do
  case "$kv" in CADASTRE_BIND=*) bind=${kv#*=} ;; CADASTRE_DATABASE_URL=*|DATABASE_URL=*) db=1 ;; esac
done
[[ -n "$db" ]] || { echo "the running server has no database setting: stopping here" >&2; exit 1; }
health=$(health_url "$bind")
# The old per-user service: retired, never run beside this one.
old_unit=$HOME/.config/systemd/user/$UNIT
grep -q 'user@[0-9]*\.service/.*cadastre-server\.service' "/proc/$pid/cgroup" && { echo "the running server belongs to the per-user service: disable that first (systemctl --user disable --now $UNIT)" >&2; exit 1; }
if [[ -e "$HOME/.config/systemd/user/default.target.wants/$UNIT" ]]; then
  echo "the per-user service is still enabled: systemctl --user disable --now $UNIT" >&2; exit 1
fi

echo "running server : pid $pid ($profile), in $workdir"
echo "settings       : ${#settings[@]} -> $ENV_FILE (root only): $(printf '%s\n' "${settings[@]%%=*}" | sort | tr '\n' ' ')"
echo "program        : built from $repo ($(git log -1 --format='%h %s')) -> $LIB_DIR/cadastre-server"
echo "service        : $UNIT_FILE, as $(id -un), started at boot"
echo "health         : $health"
[[ ! -e "$old_unit" ]] || echo "old user unit  : $old_unit -> renamed .retired"
[[ "${1:-}" == --yes ]] || { echo; echo "nothing done; run again with --yes"; exit 0; }

for kv in "${settings[@]}"; do env_line "$kv" >/dev/null; done
say "build ($profile), the running server keeps serving"
flag=(); [[ $profile == release ]] && flag=(--release)
# Kept to put back; same file name, as the scripts find the server by its process name.
before=$HOME/.cadastre-before-service
mkdir -p "$before"; cp -f "/proc/$pid/exe" "$before/cadastre-server"; chmod 700 "$before/cadastre-server"
cargo build -p cadastre-server --bin cadastre-server "${flag[@]}"

say "install"
sudo install -d -m 755 "$LIB_DIR"
sudo install -d -m 700 "$(dirname "$ENV_FILE")"
sudo install -m 755 "target/$profile/cadastre-server" "$LIB_DIR/cadastre-server"
# Through a pipe: the passwords are never on a command line or in a readable file.
for kv in "${settings[@]}"; do env_line "$kv"; done | sudo sh -c "umask 077; cat > '$ENV_FILE'"
sed -e "s|@USER@|$(id -un)|" -e "s|@WORKDIR@|$workdir|" deploy/linux/cadastre-server.service | sudo sh -c "cat > '$UNIT_FILE'"
sudo systemctl daemon-reload

say "stop pid $pid, start the service"
back() {
  echo "putting the old server back" >&2
  sudo systemctl disable --now "$UNIT" || true
  sudo rm -f "$UNIT_FILE"; sudo systemctl daemon-reload || true
  ( cd "$workdir" && exec env "${settings[@]}" nohup "$before/cadastre-server" ) >> "$workdir/server.log" 2>&1 < /dev/null &
  if wait_healthy "$health" 60; then echo "the old server is up again" >&2; else echo "THE OLD SERVER DID NOT START: see $workdir/server.log" >&2; fi
}
kill "$pid"
for _ in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid"; sleep 1; fi
if ! sudo systemctl enable --now "$UNIT" || ! wait_healthy "$health" 60; then
  echo "the service is NOT answering:" >&2
  sudo journalctl -u "$UNIT" -n 30 --no-pager >&2 || true
  back
  exit 1
fi
if [[ -e "$old_unit" ]]; then mv "$old_unit" "$old_unit.retired"; fi
rm -rf "$before"
echo "up: pid $(service_pid), log: journalctl -u $UNIT"
