#!/usr/bin/env bash
# PRODUCTION (10.10.10.6). Same read-only kit; the folder appears as \\tsclient\inventory.
# Production only offers legacy TLS, so OpenSSL's security level is lowered for this connection only.
# NTLM only: the plagis.com Kerberos servers in DNS resolve to unreachable public addresses, so Kerberos hangs.
exec env RDP_HOST=10.10.10.6 RDP_EXTRA="/tls-seclevel:0 /auth-pkg-list:ntlm,!kerberos" "$(dirname "$0")/connect.sh"
