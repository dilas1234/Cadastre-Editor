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
xfreerdp /v:"$HOST" /u:"$USER_NAME" /p:"$PASS" /cert:ignore \
  /drive:inventory,"$DIR" +clipboard /dynamic-resolution /size:1600x900 ${RDP_EXTRA:-}
