#!/usr/bin/env bash
# PRODUCTION (10.10.10.2). Same read-only kit; the folder appears as \\tsclient\inventory.
# Production only offers legacy TLS, so OpenSSL's security level is lowered for this connection only.
exec env RDP_HOST=10.10.10.2 RDP_EXTRA="/tls-seclevel:0" "$(dirname "$0")/connect.sh"
