#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 0 ]] && [[ $# -ne 2 || $1 != --features || $2 != ecpds ]]; then
    printf 'Usage: bash scripts/test_jetstream_reconnect.sh [--features ecpds]\n' >&2
    exit 2
fi

# The test owns its container, random loopback port, and disposable storage.
# It never uses NATS_URL or an existing NATS instance.
docker pull nats:2.14.6
cargo test --locked --test jetstream_reconnect "$@" -- --ignored --nocapture
