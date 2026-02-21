#!/usr/bin/env bash
set -euo pipefail
set +x

# This script manages a local NATS NUI container for inspecting JetStream/NATS state.
# It is intentionally idempotent so repeated runs don't require manual docker cleanup.

readonly IMAGE_DEFAULT="ghcr.io/nats-nui/nui"
readonly PORT_DEFAULT="31311"
readonly CONTAINER_DEFAULT="nats-nui"
readonly NETWORK_DEFAULT="aviso-net"
readonly NATS_CONTAINER_DEFAULT="nats-jetstream"

ACTION="start"

usage() {
    cat <<USAGE
Usage: $(basename "$0") [action]

Actions:
  start      Start (or restart) the NATS NUI container (default)
  stop       Stop and remove the container
  restart    Restart the container
  status     Show container status
  logs       Tail container logs

Environment overrides:
  NUI_CONTAINER_NAME   Container name (default: ${CONTAINER_DEFAULT})
  NUI_IMAGE            Image (default: ${IMAGE_DEFAULT})
  NUI_PORT             Host port mapped to container 31311 (default: ${PORT_DEFAULT})
  NUI_DOCKER_NETWORK   Docker network shared with NATS (default: ${NETWORK_DEFAULT})
  NUI_NATS_CONTAINER   NATS container DNS name in that network (default: ${NATS_CONTAINER_DEFAULT})
  NUI_DB_PATH          Host directory for NUI DB (default: ./.nui/db)
  NUI_CREDS_PATH       Host directory for NATS creds (optional, mounted read-only)
  NUI_EXTRA_ARGS       Extra arguments passed to 'docker run'

Examples:
  ./scripts/init_nui.sh
  NUI_PORT=31312 ./scripts/init_nui.sh start
  NUI_CREDS_PATH=./secrets/nats-creds ./scripts/init_nui.sh restart
  ./scripts/init_nui.sh logs
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

if [[ -n "${1:-}" ]]; then
    ACTION="$1"
fi

NUI_CONTAINER_NAME="${NUI_CONTAINER_NAME:-$CONTAINER_DEFAULT}"
NUI_IMAGE="${NUI_IMAGE:-$IMAGE_DEFAULT}"
NUI_PORT="${NUI_PORT:-$PORT_DEFAULT}"
NUI_DOCKER_NETWORK="${NUI_DOCKER_NETWORK:-$NETWORK_DEFAULT}"
NUI_NATS_CONTAINER="${NUI_NATS_CONTAINER:-$NATS_CONTAINER_DEFAULT}"
NUI_DB_PATH="${NUI_DB_PATH:-./.nui/db}"
NUI_CREDS_PATH="${NUI_CREDS_PATH:-}"
NUI_EXTRA_ARGS="${NUI_EXTRA_ARGS:-}"

log() {
    printf '[INFO] %s\n' "$*"
}

fail() {
    printf '[ERROR] %s\n' "$*" >&2
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "Missing required command: $1"
}

container_exists() {
    docker ps -a --format '{{.Names}}' | grep -Fxq "$NUI_CONTAINER_NAME"
}

container_running() {
    docker ps --format '{{.Names}}' | grep -Fxq "$NUI_CONTAINER_NAME"
}

validate_paths() {
    mkdir -p "$NUI_DB_PATH"

    if [[ -n "$NUI_CREDS_PATH" && ! -d "$NUI_CREDS_PATH" ]]; then
        fail "NUI_CREDS_PATH is set but directory does not exist: $NUI_CREDS_PATH"
    fi
}

start_container() {
    validate_paths

    docker network inspect "$NUI_DOCKER_NETWORK" >/dev/null 2>&1 || \
        docker network create "$NUI_DOCKER_NETWORK" >/dev/null

    if container_exists; then
        log "Removing existing container: $NUI_CONTAINER_NAME"
        docker rm -f "$NUI_CONTAINER_NAME" >/dev/null
    fi

    local run_args=(
        --detach
        --name "$NUI_CONTAINER_NAME"
        --network "$NUI_DOCKER_NETWORK"
        --publish "${NUI_PORT}:31311"
        --volume "${NUI_DB_PATH}:/db"
    )

    if [[ -n "$NUI_CREDS_PATH" ]]; then
        run_args+=(--volume "${NUI_CREDS_PATH}:/nats-creds:ro")
    fi

    # shellcheck disable=SC2206
    local extra_args=( $NUI_EXTRA_ARGS )

    log "Starting NATS NUI"
    docker run "${run_args[@]}" "${extra_args[@]}" "$NUI_IMAGE" >/dev/null

    log "Container: $NUI_CONTAINER_NAME"
    log "NUI URL: http://127.0.0.1:${NUI_PORT}"
    log "Docker network: $NUI_DOCKER_NETWORK"
    log "Configure NATS in NUI as: nats://${NUI_NATS_CONTAINER}:4222"
    log "DB path: $NUI_DB_PATH"
    if [[ -n "$NUI_CREDS_PATH" ]]; then
        log "Credentials path: $NUI_CREDS_PATH"
    fi
}

stop_container() {
    if container_exists; then
        log "Stopping/removing container: $NUI_CONTAINER_NAME"
        docker rm -f "$NUI_CONTAINER_NAME" >/dev/null
    else
        log "Container not found: $NUI_CONTAINER_NAME"
    fi
}

status_container() {
    if container_running; then
        docker ps --filter "name=^${NUI_CONTAINER_NAME}$" --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
    elif container_exists; then
        docker ps -a --filter "name=^${NUI_CONTAINER_NAME}$" --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
    else
        log "Container not found: $NUI_CONTAINER_NAME"
    fi
}

logs_container() {
    if container_exists; then
        docker logs -f "$NUI_CONTAINER_NAME"
    else
        fail "Container not found: $NUI_CONTAINER_NAME"
    fi
}

main() {
    require_cmd docker

    case "$ACTION" in
        start)
            start_container
            ;;
        stop)
            stop_container
            ;;
        restart)
            stop_container
            start_container
            ;;
        status)
            status_container
            ;;
        logs)
            logs_container
            ;;
        *)
            usage
            fail "Unknown action: $ACTION"
            ;;
    esac
}

main
