#!/usr/bin/env bash
# Set up sign-in for the PLAGIS CADASTRE Editor in Keycloak (container cadastre-keycloak):
#   1. the "cadastre" realm and the "cadastre-editor" client (cadastre-realm.json), if missing;
#   2. one Keycloak user per active Aumentum user (cadastre.principal), same login name, with a
#      one-time password that must be changed at first sign-in, and roles from their Aumentum
#      groups (admins -> admin + approver + reviewer + editor; others -> editor + reviewer);
#   3. optionally (--ldap <file>) AD federation so Plagis.com Windows passwords work.
# Initial passwords are written to ~/cadastre-import/keycloak-initial-passwords.txt (chmod 600),
# for the administrator to hand out. Re-runnable: existing users are left as they are.
# Env: KC_ADMIN / KC_ADMIN_PASSWORD (default admin/admin, the dev container's), PSQL.
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
KC="docker exec -i cadastre-keycloak /opt/keycloak/bin/kcadm.sh"
# Inside the user loop kcadm must not read the loop's input (docker exec -i would swallow it).
KCQ="docker exec cadastre-keycloak /opt/keycloak/bin/kcadm.sh"
PSQL="${PSQL:-docker exec -i cadastre-db psql -U cadastre -d cadastre -At -F|}"
LDAP=""; [ "${1:-}" = "--ldap" ] && LDAP="${2:?--ldap needs a file}"
$KC config credentials --server http://localhost:8080 --realm master --user "${KC_ADMIN:-admin}" --password "${KC_ADMIN_PASSWORD:-admin}" >/dev/null
if ! $KC get realms/cadastre >/dev/null 2>&1; then
  $KC create realms -f - < "$DIR/cadastre-realm.json" && echo "realm cadastre created"
else echo "realm cadastre exists"; fi
OUT=~/cadastre-import/keycloak-initial-passwords.txt; mkdir -p ~/cadastre-import; touch "$OUT"; chmod 600 "$OUT"
created=0; existing=0
while IFS='|' read -r code name email admin; do
  [ -n "$code" ] || continue
  code="$(printf '%s' "$code" | tr 'A-Z' 'a-z')" # Keycloak keeps usernames in lower case; the server matches case-insensitively
  if [ -n "$($KCQ get users -r cadastre -q "username=$code" -q exact=true --fields id 2>/dev/null | grep -o '"id"')" ]; then existing=$((existing+1)); continue; fi
  pw="$(openssl rand -base64 12 | tr -d '/+=' | cut -c1-12)"
  first="${name%% *}"; last="${name#* }"; [ "$last" = "$name" ] && last=""
  $KCQ create users -r cadastre -s "username=$code" -s enabled=true -s "firstName=$first" -s "lastName=$last" ${email:+-s "email=$email"} >/dev/null
  $KCQ set-password -r cadastre --username "$code" --new-password "$pw" --temporary >/dev/null
  roles=(--rolename editor --rolename reviewer); [ "$admin" = "t" ] && roles+=(--rolename admin --rolename approver)
  $KCQ add-roles -r cadastre --uusername "$code" "${roles[@]}" >/dev/null
  printf '%s\t%s\t%s\n' "$code" "$pw" "$name" >> "$OUT"; created=$((created+1))
done < <($PSQL -c "SELECT p.code, COALESCE(p.name, p.code), COALESCE(p.email, ''),
  (EXISTS (SELECT 1 FROM cadastre.principal_member m JOIN cadastre.principal g ON g.sid = m.group_sid WHERE m.member_sid = p.sid AND lower(g.code) = 'admins')
   OR EXISTS (SELECT 1 FROM cadastre.principal_role r WHERE r.sid = p.sid AND r.role = 'ds_gral_admin' AND r.allow))
  FROM cadastre.principal p WHERE p.is_user AND NOT p.is_locked AND p.code ~ '^[A-Za-z0-9._@-]+\$' ORDER BY p.code")
echo "users: $created created, $existing already present; initial passwords in $OUT"
if [ -n "$LDAP" ]; then
  rid=$($KC get realms/cadastre --fields id | grep -o '"id" : "[^"]*"' | cut -d'"' -f4)
  sed "s/\"_comment\"[^,]*,//" "$LDAP" | python3 -c "import sys,json; d=json.load(sys.stdin); d['parentId']='$rid'; print(json.dumps(d))" | $KC create components -r cadastre -f - && echo "AD federation added"
fi
