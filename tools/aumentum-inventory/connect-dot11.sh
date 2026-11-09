#!/usr/bin/env bash
# PRODUCTION Cadastre server (10.10.10.11, the editor on :8080). Same kit; this folder appears as \\tsclient\inventory.
# NTLM only: the plagis.com Kerberos servers in DNS resolve to unreachable public addresses, so Kerberos hangs.
# If the TLS handshake fails (legacy TLS only, as on .6 and .2), add /tls-seclevel:0:
#   RDP_EXTRA="/tls-seclevel:0 /auth-pkg-list:ntlm,!kerberos" ./connect-dot11.sh
exec env RDP_HOST=10.10.10.11 RDP_EXTRA="${RDP_EXTRA:-/auth-pkg-list:ntlm,!kerberos}" "$(dirname "$0")/connect.sh"
