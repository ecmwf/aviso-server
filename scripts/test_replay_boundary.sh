#!/usr/bin/env bash
set -euo pipefail

# Own the server and storage; never target a caller's NATS_URL.
container=$(docker run --detach --rm --publish 127.0.0.1::4222 nats:2.14.6 -js)
trap 'docker stop "$container" >/dev/null' EXIT
address=$(docker port "$container" 4222/tcp)
export NATS_URL="nats://$address"
export AVISO_RUN_NATS_TESTS=1
export AVISO_RUN_NATS_WIPE_ALL_TESTS=1
cargo test --locked "$@"
