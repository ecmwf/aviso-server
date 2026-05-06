#!/usr/bin/env bash
set -euo pipefail

# Run auth-o-tron container for local Aviso testing.
#
# Usage:
#   ./scripts/auth-o-tron-docker.sh start
#   ./scripts/auth-o-tron-docker.sh start --detach
#   ./scripts/auth-o-tron-docker.sh stop
#   ./scripts/auth-o-tron-docker.sh status
#   AUTH_O_TRON_PORT=9090 ./scripts/auth-o-tron-docker.sh start
#   AUTH_O_TRON_CONFIG_FILE=/path/to/auth-config.yaml ./scripts/auth-o-tron-docker.sh start
#
# Environment variables:
#   AUTH_O_TRON_IMAGE_REPOSITORY=eccr.ecmwf.int/auth-o-tron/auth-o-tron
#   AUTH_O_TRON_IMAGE_TAG=0.3.3
#   AUTH_O_TRON_IMAGE=<repository:tag> (overrides repository/tag split)
#   AUTH_O_TRON_PORT=8080
#   AUTH_O_TRON_CONTAINER_NAME=auth-o-tron-local
#   AUTH_O_TRON_CONFIG_FILE=<repo>/scripts/example_auth_config.yaml
#
# The default config is intended for local integration tests and does not
# require GitHub/OIDC provider setup.

AUTH_O_TRON_IMAGE_REPOSITORY="${AUTH_O_TRON_IMAGE_REPOSITORY:-eccr.ecmwf.int/auth-o-tron/auth-o-tron}"
AUTH_O_TRON_IMAGE_TAG="${AUTH_O_TRON_IMAGE_TAG:-0.3.3}"
AUTH_O_TRON_IMAGE_DEFAULT="${AUTH_O_TRON_IMAGE_REPOSITORY}:${AUTH_O_TRON_IMAGE_TAG}"
AUTH_O_TRON_IMAGE="${AUTH_O_TRON_IMAGE:-$AUTH_O_TRON_IMAGE_DEFAULT}"
AUTH_O_TRON_PORT="${AUTH_O_TRON_PORT:-8080}"
AUTH_O_TRON_CONTAINER_NAME="${AUTH_O_TRON_CONTAINER_NAME:-auth-o-tron-local}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_CONFIG_FILE="${SCRIPT_DIR}/example_auth_config.yaml"
AUTH_O_TRON_CONFIG_FILE="${AUTH_O_TRON_CONFIG_FILE:-$DEFAULT_CONFIG_FILE}"
COMMAND="start"
DETACH="false"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/auth-o-tron-docker.sh [start|stop|status] [--detach]

Commands:
  start   Start (or restart) local auth-o-tron container (default)
          Runs in foreground unless --detach is provided.
  stop    Stop and remove local auth-o-tron container
  status  Show current container state
EOF
}

if ! command -v docker >/dev/null 2>&1; then
  printf 'Error: Docker CLI is not installed or not in PATH.\n' >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  printf 'Error: Docker daemon is not reachable. Start Docker and retry.\n' >&2
  exit 1
fi

container_exists() {
  docker ps -a --format '{{.Names}}' | grep -Fxq "$AUTH_O_TRON_CONTAINER_NAME"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    start|stop|status)
      COMMAND="$1"
      ;;
    -d|--detach)
      DETACH="true"
      ;;
    help|-h|--help)
      usage
      exit 0
      ;;
    *)
      printf 'Error: unknown argument: %s\n\n' "$1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

case "$COMMAND" in
  stop)
    if container_exists; then
      printf 'Stopping and removing %s\n' "$AUTH_O_TRON_CONTAINER_NAME"
      docker rm -f "$AUTH_O_TRON_CONTAINER_NAME" >/dev/null
    else
      printf 'Container %s is not present\n' "$AUTH_O_TRON_CONTAINER_NAME"
    fi
    exit 0
    ;;
  status)
    if container_exists; then
      running="$(docker inspect -f '{{.State.Running}}' "$AUTH_O_TRON_CONTAINER_NAME")"
      if [[ "$running" == "true" ]]; then
        printf '%s is running\n' "$AUTH_O_TRON_CONTAINER_NAME"
      else
        printf '%s exists but is not running\n' "$AUTH_O_TRON_CONTAINER_NAME"
      fi
    else
      printf '%s is not present\n' "$AUTH_O_TRON_CONTAINER_NAME"
    fi
    exit 0
    ;;
  start)
    ;;
esac

printf 'Pulling image %s\n' "$AUTH_O_TRON_IMAGE"
docker pull "$AUTH_O_TRON_IMAGE"

if [[ ! -f "$AUTH_O_TRON_CONFIG_FILE" ]]; then
  printf 'Error: auth-o-tron config does not exist: %s\n' "$AUTH_O_TRON_CONFIG_FILE" >&2
  exit 1
fi

if container_exists; then
  printf 'Restarting %s\n' "$AUTH_O_TRON_CONTAINER_NAME"
  docker rm -f "$AUTH_O_TRON_CONTAINER_NAME" >/dev/null
fi

docker_args=(
  run
  --rm
  --name "$AUTH_O_TRON_CONTAINER_NAME"
  -p "${AUTH_O_TRON_PORT}:8080"
)

if [[ "$DETACH" == "true" ]]; then
  docker_args+=(-d)
fi

config_dir="$(dirname "$AUTH_O_TRON_CONFIG_FILE")"
config_name="$(basename "$AUTH_O_TRON_CONFIG_FILE")"
config_mount_dir="/etc/auth-o-tron"

docker_args+=(
  -v "${config_dir}:${config_mount_dir}:ro"
  -e "AOT_CONFIG_PATH=${config_mount_dir}/${config_name}"
)

docker_args+=("$AUTH_O_TRON_IMAGE")

printf 'Starting %s\n' "$AUTH_O_TRON_CONTAINER_NAME"
printf '  image: %s\n' "$AUTH_O_TRON_IMAGE"
printf '  listen: http://localhost:%s\n' "$AUTH_O_TRON_PORT"
printf '  config: %s\n' "$AUTH_O_TRON_CONFIG_FILE"
printf 'Make sure Aviso auth.jwt_secret matches auth-o-tron jwt.secret in this config.\n'
if [[ "$DETACH" != "true" ]]; then
  printf 'Container will run in foreground. Press Ctrl+C to stop it.\n'
fi
docker "${docker_args[@]}"

if [[ "$DETACH" == "true" ]]; then
  printf 'Container started in detached mode.\n'
  printf 'Use "./scripts/auth-o-tron-docker.sh status" to check health and "docker logs -f %s" for logs.\n' "$AUTH_O_TRON_CONTAINER_NAME"
fi
