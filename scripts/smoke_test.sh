#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:8000}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-8}"
PASS_COUNT=0
FAIL_COUNT=0

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing required command: $1"
        exit 1
    fi
}

note() {
    printf '[INFO] %s\n' "$*"
}

pass() {
    PASS_COUNT=$((PASS_COUNT + 1))
    printf '[PASS] %s\n' "$*"
}

fail() {
    FAIL_COUNT=$((FAIL_COUNT + 1))
    printf '[FAIL] %s\n' "$*"
}

run_test() {
    local name="$1"
    shift
    if "$@"; then
        pass "$name"
    else
        fail "$name"
    fi
}

json_escape() {
    printf '%s' "$1" | sed 's/"/\\"/g'
}

post_notification() {
    local polygon="$1"
    local note_text="$2"
    local date="${3:-20250706}"
    local time_value="${4:-1200}"
    local escaped_note
    escaped_note="$(json_escape "$note_text")"

    curl -fsS -X POST "${BASE_URL}/api/v1/notification" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"test_polygon\",
          \"identifier\": {
            \"date\": \"${date}\",
            \"time\": \"${time_value}\",
            \"polygon\": \"${polygon}\"
          },
          \"payload\": {\"note\": \"${escaped_note}\"}
        }" >/dev/null
}

post_mars_notification() {
    local note_text="$1"
    local stream_value="$2"
    local date="${3:-20250706}"
    local time_value="${4:-1200}"
    local escaped_note
    escaped_note="$(json_escape "$note_text")"

    curl -fsS -X POST "${BASE_URL}/api/v1/notification" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"mars\",
          \"identifier\": {
            \"class\": \"od\",
            \"expver\": \"0001\",
            \"domain\": \"g\",
            \"date\": \"${date}\",
            \"time\": \"${time_value}\",
            \"stream\": \"${stream_value}\",
            \"step\": \"1\"
          },
          \"payload\": \"${escaped_note}\"
        }" >/dev/null
}

post_dissemination_notification() {
    local note_text="$1"
    local target_value="$2"
    local date="${3:-20250706}"
    local time_value="${4:-1200}"
    local escaped_note
    escaped_note="$(json_escape "$note_text")"

    curl -fsS -X POST "${BASE_URL}/api/v1/notification" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"dissemination\",
          \"identifier\": {
            \"destination\": \"FOO\",
            \"target\": \"${target_value}\",
            \"class\": \"od\",
            \"expver\": \"0001\",
            \"domain\": \"g\",
            \"date\": \"${date}\",
            \"time\": \"${time_value}\",
            \"stream\": \"enfo\",
            \"step\": \"1\"
          },
          \"payload\": {\"note\": \"${escaped_note}\"}
        }" >/dev/null
}

health_check() {
    local out_file
    out_file="$(mktemp)"
    local code
    code="$(curl -s -o "$out_file" -w "%{http_code}" "${BASE_URL}/health")"
    rm -f "$out_file"
    [[ "$code" == "200" ]]
}

replay_requires_start_param() {
    local out_file
    out_file="$(mktemp)"
    local status
    status="$(curl -s -o "$out_file" -w "%{http_code}" \
        -X POST "${BASE_URL}/api/v1/replay" \
        -H "Content-Type: application/json" \
        -d '{
          "event_type": "test_polygon",
          "identifier": {
            "time": "1200",
            "polygon": "(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
          }
        }')"
    rm -f "$out_file"
    [[ "$status" == "400" ]]
}

watch_live_only_behavior() {
    local polygon="(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    local historical_note="smoke-watch-historical-$(date +%s%N)"
    local live_note="smoke-watch-live-$(date +%s%N)"

    post_notification "$polygon" "$historical_note"

    local watch_tmp
    watch_tmp="$(mktemp)"
    curl -sN -X POST "${BASE_URL}/api/v1/watch" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"test_polygon\",
          \"identifier\": {
            \"time\": \"1200\",
            \"polygon\": \"${polygon}\"
          }
        }" >"$watch_tmp" &
    local watch_pid=$!

    sleep 1
    post_notification "$polygon" "$live_note"
    sleep 2

    kill "$watch_pid" >/dev/null 2>&1 || true
    wait "$watch_pid" >/dev/null 2>&1 || true

    local has_live has_historical
    has_live=0
    has_historical=0
    grep -Fq "$live_note" "$watch_tmp" && has_live=1 || true
    grep -Fq "$historical_note" "$watch_tmp" && has_historical=1 || true
    rm -f "$watch_tmp"

    [[ "$has_live" -eq 1 && "$has_historical" -eq 0 ]]
}

replay_from_id_behavior() {
    local polygon="(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    local old_note="smoke-replay-id-old-$(date +%s%N)"
    local new_note="smoke-replay-id-new-$(date +%s%N)"

    post_notification "$polygon" "$old_note"
    post_notification "$polygon" "$new_note"

    local replay_out
    replay_out="$(mktemp)"
    timeout "${TIMEOUT_SECONDS}s" curl -sN -X POST "${BASE_URL}/api/v1/replay" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"test_polygon\",
          \"identifier\": {
            \"time\": \"1200\",
            \"polygon\": \"${polygon}\"
          },
          \"from_id\": \"1\"
        }" >"$replay_out" || true

    local has_new
    has_new=0
    grep -Fq "$new_note" "$replay_out" && has_new=1 || true
    rm -f "$replay_out"
    [[ "$has_new" -eq 1 ]]
}

replay_from_date_behavior() {
    local polygon="(52.5,13.4,52.6,13.5,52.5,13.6,52.4,13.5,52.5,13.4)"
    local old_note="smoke-replay-date-old-$(date +%s%N)"
    local new_note="smoke-replay-date-new-$(date +%s%N)"

    post_notification "$polygon" "$old_note"
    sleep 1
    local boundary
    boundary="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    sleep 1
    post_notification "$polygon" "$new_note"

    local replay_out
    replay_out="$(mktemp)"
    timeout "${TIMEOUT_SECONDS}s" curl -sN -X POST "${BASE_URL}/api/v1/replay" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"test_polygon\",
          \"identifier\": {
            \"time\": \"1200\",
            \"polygon\": \"${polygon}\"
          },
          \"from_date\": \"${boundary}\"
        }" >"$replay_out" || true

    local has_old has_new
    has_old=0
    has_new=0
    grep -Fq "$old_note" "$replay_out" && has_old=1 || true
    grep -Fq "$new_note" "$replay_out" && has_new=1 || true
    rm -f "$replay_out"
    [[ "$has_new" -eq 1 && "$has_old" -eq 0 ]]
}

replay_mars_from_id_with_dot_identifier() {
    local stream_value="ens.member.$(date +%s%N)"
    local first_note="smoke-mars-replay-first-$(date +%s%N)"
    local second_note="smoke-mars-replay-second-$(date +%s%N)"

    post_mars_notification "$first_note" "$stream_value"
    post_mars_notification "$second_note" "$stream_value"

    local replay_out
    replay_out="$(mktemp)"
    timeout "${TIMEOUT_SECONDS}s" curl -sN -X POST "${BASE_URL}/api/v1/replay" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"mars\",
          \"identifier\": {
            \"class\": \"od\",
            \"expver\": \"0001\",
            \"domain\": \"g\",
            \"date\": \"20250706\",
            \"time\": \"1200\",
            \"stream\": \"${stream_value}\",
            \"step\": \"1\"
          },
          \"from_id\": \"1\"
        }" >"$replay_out" || true

    local has_stream
    has_stream=0
    grep -Fq "$stream_value" "$replay_out" && has_stream=1 || true
    rm -f "$replay_out"
    [[ "$has_stream" -eq 1 ]]
}

watch_diss_from_date_with_dot_identifier() {
    local target_value="target.v1.$(date +%s%N)"
    local historical_note="smoke-diss-watch-old-$(date +%s%N)"
    local live_note="smoke-diss-watch-live-$(date +%s%N)"

    post_dissemination_notification "$historical_note" "$target_value"
    sleep 1
    local boundary
    boundary="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

    local watch_tmp
    watch_tmp="$(mktemp)"
    curl -sN -X POST "${BASE_URL}/api/v1/watch" \
        -H "Content-Type: application/json" \
        -d "{
          \"event_type\": \"dissemination\",
          \"identifier\": {
            \"destination\": \"FOO\",
            \"target\": \"${target_value}\",
            \"class\": \"od\",
            \"expver\": \"0001\",
            \"domain\": \"g\",
            \"date\": \"20250706\",
            \"time\": \"1200\",
            \"stream\": \"enfo\",
            \"step\": \"1\"
          },
          \"from_date\": \"${boundary}\"
        }" >"$watch_tmp" &
    local watch_pid=$!

    sleep 1
    post_dissemination_notification "$live_note" "$target_value"
    sleep 2

    kill "$watch_pid" >/dev/null 2>&1 || true
    wait "$watch_pid" >/dev/null 2>&1 || true

    local has_live has_historical
    has_live=0
    has_historical=0
    grep -Fq "$live_note" "$watch_tmp" && has_live=1 || true
    grep -Fq "$historical_note" "$watch_tmp" && has_historical=1 || true
    rm -f "$watch_tmp"

    [[ "$has_live" -eq 1 && "$has_historical" -eq 0 ]]
}

main() {
    require_cmd curl
    require_cmd grep
    require_cmd date
    require_cmd timeout

    note "Running smoke tests against ${BASE_URL}"
    run_test "health endpoint returns 200" health_check
    run_test "replay requires from_id or from_date" replay_requires_start_param
    run_test "watch without replay params is live-only" watch_live_only_behavior
    run_test "replay with from_id returns historical stream" replay_from_id_behavior
    run_test "replay with from_date excludes older messages" replay_from_date_behavior
    run_test "mars replay with from_id works for dot-containing identifier values" replay_mars_from_id_with_dot_identifier
    run_test "diss watch with from_date excludes old and includes live for dot-containing identifier values" watch_diss_from_date_with_dot_identifier

    echo
    note "Smoke summary: pass=${PASS_COUNT} fail=${FAIL_COUNT}"
    if [[ "$FAIL_COUNT" -gt 0 ]]; then
        exit 1
    fi
}

main "$@"
