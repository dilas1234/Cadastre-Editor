#!/usr/bin/env bash
# Opens a Remote Desktop session to the Aumentum server with THIS folder shared into it.
# Inside the session the folder appears as \\tsclient\inventory (also under "This PC").
# Usage: ./connect.sh            (prompts for the password)
set -euo pipefail
HOST="${RDP_HOST:-10.10.10.4}"
USER_NAME="${RDP_USER:-Administrator@plagis.com}"
DIR="$(cd "$(dirname "$0")" && pwd)"
read -rsp "Password for $USER_NAME@$HOST: " PASS; echo
echo "Connecting... inside the session open:  \\\\tsclient\\inventory\\RUN-ME.cmd"
EXTRA="${RDP_EXTRA:-}"
if command -v xfreerdp3 >/dev/null; then
  RDP=xfreerdp3
elif command -v xfreerdp >/dev/null; then
  RDP=xfreerdp
else
  echo "xfreerdp not found: sudo apt-get install -y freerdp3-x11" >&2
  exit 1
fi
# FreeRDP 3 (also when installed as plain `xfreerdp`): `/tls-seclevel:N` became `/tls:seclevel:N`.
if "$RDP" /version 2>&1 | grep -q 'version 3\.'; then
  EXTRA="${EXTRA//\/tls-seclevel:/\/tls:seclevel:}"
fi
# The title deed tools (tools/titledeed) are shared too, as \\tsclient\titledeed.
TD="$(cd "$DIR/../titledeed" 2>/dev/null && pwd || true)"
if [ -n "$TD" ]; then EXTRA="$EXTRA /drive:titledeed,$TD"; echo "Title deed tools in the session: \\\\tsclient\\titledeed"; fi
"$RDP" /v:"$HOST" /u:"$USER_NAME" /p:"$PASS" /cert:ignore \
  /drive:inventory,"$DIR" +clipboard /dynamic-resolution /size:1600x900 $EXTRA
