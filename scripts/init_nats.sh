#!/usr/bin/env bash
set -euo pipefail
set +x

# Server configuration
NATS_PORT="${NATS_PORT:=4222}"
NATS_HTTP_PORT="${NATS_HTTP_PORT:=8222}"
NATS_CLUSTER_PORT="${NATS_CLUSTER_PORT:=6222}"
DOCKER_NETWORK="${DOCKER_NETWORK:=aviso-net}"
CONTAINER_NAME="${CONTAINER_NAME:=nats-jetstream}"
NATS_IMAGE="${NATS_IMAGE:-nats:2.14.6-alpine}"
NATS_BIND_ADDRESS="${NATS_BIND_ADDRESS:-127.0.0.1}"
NATS_VOLUME="${NATS_VOLUME:-${CONTAINER_NAME}-data}"
# Wildcard binds are not connection destinations. Bracket IPv6 for URLs and Docker.
case "${NATS_BIND_ADDRESS}" in
    \[*\]) ;;
    *:*) NATS_BIND_ADDRESS="[${NATS_BIND_ADDRESS}]" ;;
esac
case "${NATS_BIND_ADDRESS}" in
    0.0.0.0) NATS_CONNECT_HOST="127.0.0.1" ;;
    '[::]') NATS_CONNECT_HOST="[::1]" ;;
    *) NATS_CONNECT_HOST="${NATS_BIND_ADDRESS}" ;;
esac
export NATS_URL="nats://${NATS_CONNECT_HOST}:${NATS_PORT}"

# JetStream storage limits
MAX_MEMORY="${MAX_MEMORY:=5GB}"
MAX_STORAGE="${MAX_STORAGE:=10GB}"

# Authentication configuration
ENABLE_AUTH="${ENABLE_AUTH:=false}"
TOKEN="${TOKEN:-}"
if [[ "${ENABLE_AUTH}" == "true" ]]; then
    TOKEN="${TOKEN:-$(openssl rand -hex 32)}"
    if [[ ! "${TOKEN}" =~ ^[a-zA-Z0-9_-]+$ ]]; then
        printf 'TOKEN must contain only letters, digits, underscores or hyphens.\n' >&2
        exit 1
    fi
fi

# Create configuration directory
umask 077
CONFIG_DIR="${CONFIG_DIR:-${XDG_STATE_HOME:-${HOME}/.local/state}/aviso/${CONTAINER_NAME}}"
mkdir -p "${CONFIG_DIR}"
CONFIG_DIR="$(realpath "${CONFIG_DIR}")"
if [[ -z "${SKIP_DOCKER:-}" ]] && docker container inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
    printf 'Container %s already exists; stop/remove it explicitly or choose another CONTAINER_NAME.\n' "${CONTAINER_NAME}" >&2
    exit 1
fi

# Generate NATS server configuration
generate_nats_config() {
    local config_file="${CONFIG_DIR}/nats-server.conf"
    touch "${config_file}"
    chmod 600 "${config_file}"

    cat > "${config_file}" << EOF
# NATS Server Configuration for Aviso Server
port: 4222

# JetStream Configuration
jetstream {
    store_dir: "/data"
    max_memory_store: ${MAX_MEMORY}
    max_file_store: ${MAX_STORAGE}
}

# Logging
debug: false
trace: false
logtime: true

# Health check endpoint
http: "0.0.0.0:8222"

EOF

    if [[ "${ENABLE_AUTH}" == "true" ]]; then
        cat >> "${config_file}" << EOF
# Token Authentication
authorization {
    token: "${TOKEN}"
}
EOF
        echo "Authentication enabled (token is stored in the private configuration file)"
    else
        echo "# No authentication configured" >> "${config_file}"
        echo "Authentication disabled"
    fi

    echo "Generated NATS configuration: ${config_file}"
}

# Generate the configuration
generate_nats_config

# Launch NATS with JetStream using Docker
if [[ -z "${SKIP_DOCKER:-}" ]]; then
    # Ensure shared docker network exists for NATS ecosystem tools (e.g. NUI)
    docker network inspect "${DOCKER_NETWORK}" >/dev/null 2>&1 || \
        docker network create "${DOCKER_NETWORK}" >/dev/null

    # Create volumes for persistence
    docker volume create "${NATS_VOLUME}" >/dev/null

    echo "Starting NATS JetStream server..."

    # Launch NATS with custom configuration
    docker run \
        --name "${CONTAINER_NAME}" \
        --network "${DOCKER_NETWORK}" \
        --publish "${NATS_BIND_ADDRESS}:${NATS_PORT}:4222" \
        --publish "${NATS_BIND_ADDRESS}:${NATS_HTTP_PORT}:8222" \
        --publish "${NATS_BIND_ADDRESS}:${NATS_CLUSTER_PORT}:6222" \
        --volume "${CONFIG_DIR}:/config:ro" \
        --volume "${NATS_VOLUME}:/data" \
        --detach \
        "${NATS_IMAGE}" \
        --config /config/nats-server.conf

    # Wait for NATS to be ready
    echo "Waiting for NATS server to be ready..."
    sleep 3

    # Set authentication for the readiness check.
    if [[ "${ENABLE_AUTH}" == "true" ]]; then
        export NATS_TOKEN="${TOKEN}"
    fi

    # Test connection with retry logic
    MAX_RETRIES=30
    RETRY_COUNT=0

    while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
        if nats --server "${NATS_URL}" server check connection 2>/dev/null; then
            echo "NATS server is ready!"
            break
        else
            echo "NATS is still starting up - sleeping (attempt $((RETRY_COUNT + 1))/$MAX_RETRIES)"
            sleep 2
            RETRY_COUNT=$((RETRY_COUNT + 1))
        fi
    done

    if [ $RETRY_COUNT -eq $MAX_RETRIES ]; then
        echo "ERROR: NATS server failed to start within expected time"
        docker logs "${CONTAINER_NAME}"
        exit 1
    fi
fi

if [[ -n "${SKIP_DOCKER:-}" ]]; then
    echo "Configuration generated; Docker launch skipped."
    exit 0
fi

echo ""
echo "=== NATS JetStream is ready! ==="
echo "Server URL: ${NATS_URL}"
echo "In-network URL: nats://${CONTAINER_NAME}:4222"
echo "HTTP Monitoring: http://${NATS_CONNECT_HOST}:${NATS_HTTP_PORT}"
echo "Docker network: ${DOCKER_NETWORK}"
echo "JetStream enabled with ${MAX_MEMORY} memory and ${MAX_STORAGE} file storage"

if [[ "${ENABLE_AUTH}" == "true" ]]; then
    echo "Authentication: Token-based"
    echo "Set NATS_TOKEN securely using the token in ${CONFIG_DIR}/nats-server.conf"
else
    echo "Authentication: Disabled"
fi

echo ""
echo "=== Configuration ==="
echo "Configuration saved in: ${CONFIG_DIR}/nats-server.conf"
echo ""
echo "=== Useful Commands (If nats cli is installed in the system) ==="
echo "  nats stream ls                                    # List streams (created by your app)"
echo "  nats stream info <STREAM_NAME>                   # Stream details"
echo "  nats consumer ls <STREAM_NAME>                   # List consumers"
echo "  nats sub 'diss.>'                               # Subscribe to all dissemination events"
echo "  nats sub 'mars.>'                               # Subscribe to all MARS events"
echo "  nats sub 'bench.>'                              # Subscribe to benchmark events"
echo ""
echo "=== Management ==="
echo "To stop: docker stop ${CONTAINER_NAME}"
echo "To remove: docker rm ${CONTAINER_NAME} && docker volume rm ${NATS_VOLUME}"
echo "To restart: docker start ${CONTAINER_NAME}"
echo ""
echo "=== Environment Variables for Aviso Server ==="
echo "export AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream"
