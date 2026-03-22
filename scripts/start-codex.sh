#!/usr/bin/env bash
# Start a local Codex (Logos Storage) node for integration testing.
# Usage: ./scripts/start-codex.sh [start|stop|status]
#
# API will be available at http://127.0.0.1:${CODEX_PORT:-8090}
# Upload:   POST /api/storage/v1/data  (Content-Type: application/octet-stream)
# Download: GET  /api/storage/v1/data/{cid}/network/stream

set -euo pipefail

CONTAINER_NAME="${CODEX_CONTAINER:-codex-test}"
IMAGE="${CODEX_IMAGE:-codexstorage/nim-codex:latest}"
HOST_PORT="${CODEX_PORT:-8090}"

case "${1:-start}" in
  start)
    if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
      echo "Codex already running (${CONTAINER_NAME})"
      exit 0
    fi
    # Clean up stopped container if exists
    docker rm -f "${CONTAINER_NAME}" 2>/dev/null || true

    echo "Starting Codex node (${IMAGE})..."
    docker run --rm -d \
      --name "${CONTAINER_NAME}" \
      -p "${HOST_PORT}:8080" \
      --entrypoint /usr/local/bin/storage \
      "${IMAGE}" \
      --data-dir=/data \
      --listen-addrs=/ip4/0.0.0.0/tcp/8070 \
      --api-bindaddr=0.0.0.0 \
      --api-port=8080

    # Wait for API to be ready
    echo -n "Waiting for API..."
    for i in $(seq 1 30); do
      if curl -sf "http://127.0.0.1:${HOST_PORT}/api/storage/v1/debug/info" >/dev/null 2>&1; then
        echo " ready!"
        curl -s "http://127.0.0.1:${HOST_PORT}/api/storage/v1/debug/info" | python3 -m json.tool 2>/dev/null || true
        exit 0
      fi
      echo -n "."
      sleep 1
    done
    echo " TIMEOUT"
    docker logs "${CONTAINER_NAME}" 2>&1 | tail -10
    exit 1
    ;;
  stop)
    docker rm -f "${CONTAINER_NAME}" 2>/dev/null && echo "Stopped ${CONTAINER_NAME}" || echo "Not running"
    ;;
  status)
    if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
      echo "Running"
      curl -s "http://127.0.0.1:${HOST_PORT}/api/storage/v1/debug/info" | python3 -m json.tool 2>/dev/null || true
    else
      echo "Not running"
    fi
    ;;
  *)
    echo "Usage: $0 [start|stop|status]"
    exit 1
    ;;
esac
