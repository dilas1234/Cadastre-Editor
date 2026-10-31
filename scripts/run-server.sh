#!/usr/bin/env bash
# Start PostGIS (Docker) and the cadastre server with the editor/viewer.
# Open http://localhost:8080/editor  (or http://<this-host-LAN-IP>:8080/editor on the network).
set -euo pipefail
cd "$(dirname "$0")/.."

docker compose up -d db

export CADASTRE_DATABASE_URL="${CADASTRE_DATABASE_URL:-postgres://cadastre:cadastre@localhost:5432/cadastre}"
export CADASTRE_BIND="${CADASTRE_BIND:-0.0.0.0:8080}"
export CADASTRE_AUTH="${CADASTRE_AUTH:-disabled}"   # dev only; enable Keycloak before real exposure

echo "editor:  http://localhost:8080/editor"
echo "viewer:  http://localhost:8080/viewer"
echo "api:     http://localhost:8080/swagger-ui"
exec cargo run -p cadastre-server --bin cadastre-server
