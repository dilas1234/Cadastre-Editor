#!/usr/bin/env bash
# Run a Registry settings script against LRS43 on 10.10.10.5 from this PC.
#   bash tools/aumentum-inventory/registry-settings.sh check    # show the three settings (read-only)
#   bash tools/aumentum-inventory/registry-settings.sh switch   # point the Registry at the new Cadastre (.9)
#   bash tools/aumentum-inventory/registry-settings.sh revert   # back to Aumentum (.6), exact saved values
# Login: REGISTRY_LRS_SQL_USER / REGISTRY_LRS_SQL_PASSWORD from the repo's .env (never printed).
# Uses sqlcmd in the local cadastre-mssql-test container, with TLS 1.0 allowed for this client only
# (the Registry's SQL Server offers nothing newer).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
mode="${1:-check}"
get() { grep -E "^$1=" "$repo/.env" | head -1 | cut -d= -f2- | sed -e 's/^["'\'']//' -e 's/["'\'']$//'; }
user="$(get REGISTRY_LRS_SQL_USER)"; pass="$(get REGISTRY_LRS_SQL_PASSWORD)"
[ -n "$user" ] && [ -n "$pass" ] || { echo "REGISTRY_LRS_SQL_USER / REGISTRY_LRS_SQL_PASSWORD missing in .env" >&2; exit 1; }
c=cadastre-mssql-test
tool="$(docker exec $c sh -c 'ls /opt/mssql-tools18/bin/sqlcmd /opt/mssql-tools/bin/sqlcmd 2>/dev/null | head -1')"
docker exec $c sh -c 'printf "openssl_conf = d\n[d]\nssl_conf = s\n[s]\nsystem_default = sd\n[sd]\nMinProtocol = TLSv1\nCipherString = DEFAULT@SECLEVEL=0\n" > /tmp/legacy-tls.cnf'
case "$mode" in
  check)  q="SET NOCOUNT ON; SELECT category, name, value FROM LRSAdmin.lr_setting WHERE (category='mcintegration' AND name IN ('GISServer','LWAMapURL')) OR (category='multicadastre' AND name='mcServiceAddress')"
          args=(-Q "$q") ;;
  switch) docker cp "$here/registry-settings-switch-to-new-cadastre.sql" $c:/tmp/registry-settings.sql >/dev/null; args=(-i /tmp/registry-settings.sql) ;;
  revert) docker cp "$here/registry-settings-revert-to-aumentum.sql" $c:/tmp/registry-settings.sql >/dev/null; args=(-i /tmp/registry-settings.sql) ;;
  *) echo "usage: $0 check|switch|revert" >&2; exit 2 ;;
esac
docker exec -e SQLCMDPASSWORD="$pass" -e OPENSSL_CONF=/tmp/legacy-tls.cnf $c "$tool" -S 10.10.10.5,1433 -U "$user" -d LRS43 -C -No -l 15 -b -W -s ' | ' "${args[@]}"
[ "$mode" = check ] || echo "Done ($mode). Registry desktop users: log out and in again."
