#!/usr/bin/env bash
set -x
set -eo pipefail

# Check if nats CLI is installed
if ! [ -x "$(command -v nats)" ]; then
  echo >&2 "Error: nats CLI is not installed."
  echo >&2 "Use:"
  echo >&2 "  curl -sf https://binaries.nats.dev/nats-io/natscli/nats@latest | sh"
  echo >&2 "or"
  echo >&2 "  go install github.com/nats-io/natscli/nats@latest"
  echo >&2 "to install it."
  exit 1
fi

# Server configuration
NATS_PORT="${NATS_PORT:=4222}"
NATS_HTTP_PORT="${NATS_HTTP_PORT:=8222}"
NATS_CLUSTER_PORT="${NATS_CLUSTER_PORT:=6222}"

# Stream configuration
STREAM_NAME="${STREAM_NAME:="diss"}"
MAX_MEMORY="${MAX_MEMORY:=1GB}"
MAX_STORAGE="${MAX_STORAGE:=10GB}"
STORAGE_TYPE="${STORAGE_TYPE:=file}"
RETENTION_POLICY="${RETENTION_POLICY:=limits}"

# Authentication configuration
ENABLE_AUTH="${ENABLE_AUTH:=false}"
TOKEN="${TOKEN:=aviso_secure_token_$(date +%s)}"

# Kubernetes/Cluster configuration
NATS_CLUSTER_SIZE="${NATS_CLUSTER_SIZE:=1}"
STREAM_REPLICAS="${STREAM_REPLICAS:=${NATS_CLUSTER_SIZE}}"

# Create configuration directory
CONFIG_DIR="./nats_config"
mkdir -p "${CONFIG_DIR}"

# Generate NATS server configuration
generate_nats_config() {
    local config_file="${CONFIG_DIR}/nats-server.conf"

    cat > "${config_file}" << EOF
# NATS Server Configuration for Aviso Server
port: 4222
http_port: 8222

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
        echo "Authentication enabled with token: ${TOKEN}"
    else
        echo "# No authentication configured" >> "${config_file}"
        echo "Authentication disabled"
    fi

    echo "Generated NATS configuration: ${config_file}"
}

# Generate the configuration
generate_nats_config

# Launch NATS with JetStream using Docker
if [[ -z "${SKIP_DOCKER}" ]]; then
    CONTAINER_NAME="nats-jetstream"

    # Stop and remove existing container if it exists
    docker stop "${CONTAINER_NAME}" 2>/dev/null || true
    docker rm "${CONTAINER_NAME}" 2>/dev/null || true

    # Create volumes for persistence
    docker volume create nats-jetstream-data 2>/dev/null || true

    echo "Starting NATS JetStream server..."

    # Launch NATS with custom configuration
    docker run \
        --name "${CONTAINER_NAME}" \
        --publish "${NATS_PORT}":4222 \
        --publish "${NATS_HTTP_PORT}":8222 \
        --publish "${NATS_CLUSTER_PORT}":6222 \
        --volume "$(pwd)/${CONFIG_DIR}:/config:ro" \
        --volume nats-jetstream-data:/data \
        --detach \
        nats:latest \
        --config /config/nats-server.conf

    # Wait for NATS to be ready
    echo "Waiting for NATS server to be ready..."
    sleep 3

    # Set connection URL and authentication
    if [[ "${ENABLE_AUTH}" == "true" ]]; then
        export NATS_URL="nats://localhost:${NATS_PORT}"
        export NATS_TOKEN="${TOKEN}"
    else
        export NATS_URL="nats://localhost:${NATS_PORT}"
    fi

    # Test connection with retry logic
    MAX_RETRIES=30
    RETRY_COUNT=0

    while [ $RETRY_COUNT -lt $MAX_RETRIES ]; do
        if nats server check connection 2>/dev/null; then
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

# Create the main stream for Aviso notifications
# Use wildcard to capture all your base subjects (diss.*, mars.*, etc.)
echo "Creating JetStream stream: ${STREAM_NAME}"
nats stream add "${STREAM_NAME}" \
    --subjects="*" \
    --storage="${STORAGE_TYPE}" \
    --retention="${RETENTION_POLICY}" \
    --max-msgs=1000000 \
    --max-bytes=1GB \
    --max-age=7d \
    --max-msg-size=-1 \
    --discard=old \
    --dupe-window=2m \
    --replicas="${STREAM_REPLICAS}" \
    --no-ack || echo "Stream might already exist"

# Create a monitoring consumer (optional, useful for debugging)
echo "Creating monitoring consumer"
nats consumer add "${STREAM_NAME}" MONITOR \
    --filter="*" \
    --ack=explicit \
    --pull \
    --deliver=all \
    --max-deliver=-1 \
    --replay=instant || echo "Consumer might already exist"

# Show stream info
echo ""
echo "=== Stream Information ==="
nats stream info "${STREAM_NAME}"

echo ""
echo "=== NATS JetStream is ready! ==="
echo "Server URL: ${NATS_URL}"
echo "HTTP Monitoring: http://localhost:${NATS_HTTP_PORT}"
echo "Stream: ${STREAM_NAME}"
echo "Subject pattern: * (captures all: diss.*, mars.*, etc.)"
echo "Storage: ${STORAGE_TYPE}"
echo "Replicas: ${STREAM_REPLICAS}"

if [[ "${ENABLE_AUTH}" == "true" ]]; then
    echo "Authentication: Token-based"
    echo "Token: ${TOKEN}"
    echo ""
    echo "To connect with token:"
    echo "  export NATS_TOKEN=${TOKEN}"
    echo "  nats pub diss.test.message 'Hello with auth'"
else
    echo "Authentication: Disabled"
    echo ""
    echo "To test without auth:"
    echo "  nats pub diss.test.message 'Hello without auth'"
fi

echo ""
echo "=== Configuration ==="
echo "Configuration saved in: ${CONFIG_DIR}/nats-server.conf"
echo ""
echo "=== Useful Commands ==="
echo "  nats stream ls                                    # List streams"
echo "  nats stream info ${STREAM_NAME}                  # Stream details"
echo "  nats consumer ls ${STREAM_NAME}                  # List consumers"
echo "  nats pub diss.test.message 'Hello'              # Publish test dissemination message"
echo "  nats pub mars.test.message 'Hello'              # Publish test MARS message"
echo "  nats sub 'diss.*'                               # Subscribe to all dissemination events"
echo "  nats sub 'mars.*'                               # Subscribe to all MARS events"
echo "  nats sub '*'                                     # Subscribe to all events"
echo "  nats stream view ${STREAM_NAME}                  # View stream messages"
echo ""
echo "=== Management ==="
echo "To stop: docker stop ${CONTAINER_NAME}"
echo "To remove: docker rm ${CONTAINER_NAME} && docker volume rm nats-jetstream-data"
echo "To restart: docker start ${CONTAINER_NAME}"
echo ""
echo "=== Environment Variables for Aviso Server ==="
echo "export AVISOSERVER_NOTIFICATION_BACKEND__KIND=jetstream"
if [[ "${ENABLE_AUTH}" == "true" ]]; then
    echo "export NATS_TOKEN=${TOKEN}"
fi
echo ""
echo "=== Example Topics Your System Will Create ==="
echo "Dissemination: diss.ecmwf.operational.od.0001.g.20250529.1200.oper.0"
echo "MARS: mars.od.0001.g.20250529.1200.oper.0"
